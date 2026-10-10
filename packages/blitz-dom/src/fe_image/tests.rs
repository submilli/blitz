//! `feImage` requests: synchronous `data:` images, No-CORS fetches that apply
//! only to the current `href`, fragments, and inert documents.
use super::FeImageTarget;
use crate::node::ImageData;
use crate::{BaseDocument, DocumentConfig, NodeId, QualName, qual_name};
use blitz_traits::net::{Bytes, CorsSettings, NetHandler, NetProvider, Request, ResponseMetadata};
use std::io::Cursor;
use std::sync::{Arc, Mutex};

/// Holds every request until the test answers it.
#[derive(Default)]
struct Deferred(Mutex<Vec<(Request, Box<dyn NetHandler>)>>);
impl NetProvider for Deferred {
    fn fetch(&self, _: usize, request: Request, handler: Box<dyn NetHandler>) {
        self.0.lock().unwrap().push((request, handler));
    }
}
impl Deferred {
    fn urls(&self) -> Vec<(String, Option<CorsSettings>)> {
        let requests = self.0.lock().unwrap();
        requests
            .iter()
            .map(|(r, _)| (r.url.to_string(), r.image))
            .collect()
    }
    fn take(&self) -> Box<dyn NetHandler> {
        self.0.lock().unwrap().remove(0).1
    }
}

fn png() -> Vec<u8> {
    let mut bytes = Cursor::new(Vec::new());
    image::RgbaImage::from_pixel(1, 1, image::Rgba([10, 20, 30, 255]))
        .write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap();
    bytes.into_inner()
}

fn data_png() -> String {
    let bytes = png();
    let encoded = percent_encoding::percent_encode(&bytes, percent_encoding::NON_ALPHANUMERIC);
    format!("data:image/png,{encoded}")
}

fn answer(handler: Box<dyn NetHandler>, url: &str, origin_clean: bool) {
    handler.bytes_with_metadata(
        url.into(),
        Bytes::from(png()),
        ResponseMetadata {
            image_origin_clean: origin_clean,
            ..Default::default()
        },
    );
}

fn svg_name(local: &str) -> QualName {
    QualName::new(None, crate::ns!(svg), local.into())
}

fn document(provider: Arc<Deferred>) -> (BaseDocument, NodeId) {
    let mut doc = BaseDocument::new(DocumentConfig {
        base_url: Some("https://page.test/doc".into()),
        net_provider: Some(provider),
        ..Default::default()
    });
    let root = doc.root_node().id;
    let mut m = doc.mutate();
    let html = m.create_element(qual_name!("html", html), vec![]);
    m.pre_insert(html, root, None).unwrap();
    drop(m);
    (doc, html)
}

fn fe_image(doc: &mut BaseDocument, parent: NodeId, href: &str) -> NodeId {
    let mut m = doc.mutate();
    let node = m.create_element(svg_name("feImage"), vec![]);
    m.set_attribute(node, qual_name!("href"), href);
    m.append_children(parent, &[node]);
    node
}

fn target(doc: &BaseDocument, node: NodeId) -> FeImageTarget {
    doc.fe_image_target(node, &mut super::InlineDecodes::default())
}

/// The loaded image's readback authority, or `None` while nothing is loaded.
fn loaded(doc: &BaseDocument, node: NodeId) -> Option<bool> {
    match target(doc, node) {
        FeImageTarget::Fetched(Some(ImageData::Raster(raster))) => Some(raster.origin_clean),
        FeImageTarget::Fetched(Some(ImageData::Svg(svg))) => Some(svg.origin_clean),
        _ => None,
    }
}

#[test]
fn data_images_decode_synchronously_and_stay_readable() {
    let provider = Arc::new(Deferred::default());
    let (mut doc, html) = document(provider.clone());
    let raster = fe_image(&mut doc, html, &data_png());
    assert_eq!(loaded(&doc, raster), Some(true));
    let svg = fe_image(
        &mut doc,
        html,
        "data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg' width='4' height='4'/>",
    );
    assert_eq!(loaded(&doc, svg), Some(true));
    let invalid = fe_image(&mut doc, html, "data:image/png;base64,AAAA");
    assert!(matches!(
        target(&doc, invalid),
        FeImageTarget::Fetched(None)
    ));
    assert!(provider.urls().is_empty(), "data: URLs are never fetched");
}

#[test]
fn external_images_fetch_no_cors_and_apply_to_the_current_href_only() {
    let provider = Arc::new(Deferred::default());
    let (mut doc, html) = document(provider.clone());
    let node = fe_image(&mut doc, html, "a.png");
    assert_eq!(
        provider.urls(),
        [("https://page.test/a.png".into(), Some(CorsSettings::NoCors))]
    );
    assert!(matches!(target(&doc, node), FeImageTarget::Fetched(None)));
    answer(provider.take(), "https://page.test/a.png", true);
    doc.handle_messages();
    assert_eq!(loaded(&doc, node), Some(true));

    // Setting the same href again, or moving the element, keeps its request.
    doc.mutate()
        .set_attribute(node, qual_name!("href"), "a.png");
    let mut m = doc.mutate();
    m.remove_node(node);
    m.append_children(html, &[node]);
    drop(m);
    assert!(provider.urls().is_empty());
    assert_eq!(loaded(&doc, node), Some(true));

    // A completion for a superseded href is ignored.
    doc.mutate()
        .set_attribute(node, qual_name!("href"), "https://cdn.test/b.png");
    doc.mutate()
        .set_attribute(node, qual_name!("href"), "https://cdn.test/c.png");
    assert_eq!(provider.urls().len(), 2);
    answer(provider.take(), "https://cdn.test/b.png", false);
    doc.handle_messages();
    assert_eq!(loaded(&doc, node), None);
    answer(provider.take(), "https://cdn.test/c.png", false);
    doc.handle_messages();
    assert_eq!(loaded(&doc, node), Some(false), "cross-origin pixels taint");

    // Another element reuses the cached image, and a failure loads nothing.
    let cached = fe_image(&mut doc, html, "https://cdn.test/c.png");
    assert!(provider.urls().is_empty());
    assert_eq!(loaded(&doc, cached), Some(false));
    doc.mutate()
        .set_attribute(cached, qual_name!("href"), "https://cdn.test/missing.png");
    provider.take().failed();
    doc.handle_messages();
    assert!(matches!(target(&doc, cached), FeImageTarget::Fetched(None)));
}

#[test]
fn removed_elements_stop_waiting() {
    let provider = Arc::new(Deferred::default());
    let (mut doc, html) = document(provider.clone());
    let node = fe_image(&mut doc, html, "slow.png");
    assert_eq!(doc.pending_images.len(), 1);
    doc.mutate().remove_and_drop_node(node);
    assert!(doc.pending_images.is_empty());
    assert!(doc.fe_images.requests.is_empty());
}

#[test]
fn fragments_and_missing_hrefs_fetch_nothing() {
    let provider = Arc::new(Deferred::default());
    let (mut doc, html) = document(provider.clone());
    let fragment = fe_image(&mut doc, html, "#shape");
    assert!(matches!(
        target(&doc, fragment),
        FeImageTarget::Fragment(id) if id == "shape"
    ));
    let absolute = fe_image(&mut doc, html, "https://page.test/doc#a%20b");
    assert!(matches!(
        target(&doc, absolute),
        FeImageTarget::Fragment(id) if id == "a b"
    ));
    let empty = fe_image(&mut doc, html, "");
    assert!(matches!(target(&doc, empty), FeImageTarget::Unfetched));
    // `xlink:href` applies only without `href`.
    let xlink = QualName::new(None, crate::ns!(xlink), "href".into());
    let mut m = doc.mutate();
    m.set_attribute(empty, xlink.clone(), "#other");
    drop(m);
    assert!(matches!(target(&doc, empty), FeImageTarget::Unfetched));
    doc.mutate().clear_attribute(empty, qual_name!("href"));
    assert!(matches!(
        target(&doc, empty),
        FeImageTarget::Fragment(id) if id == "other"
    ));
    assert!(provider.urls().is_empty());
}

#[test]
fn inert_documents_never_fetch() {
    let provider = Arc::new(Deferred::default());
    let (mut doc, html) = document(provider.clone());
    let mut m = doc.mutate();
    let template = m.create_element(qual_name!("template", html), vec![]);
    m.append_children(html, &[template]);
    let contents = m.template_contents(template);
    let node = m.create_element(svg_name("feImage"), vec![]);
    m.append_children(contents, &[node]);
    m.set_attribute(node, qual_name!("href"), "inert.png");
    drop(m);
    assert!(provider.urls().is_empty());
    assert!(doc.fe_images.requests.is_empty());
    assert_eq!(loaded(&doc, node), None);
}

#[test]
fn only_connected_elements_fetch() {
    let provider = Arc::new(Deferred::default());
    let (mut doc, html) = document(provider.clone());
    let mut m = doc.mutate();
    let node = m.create_element(svg_name("feImage"), vec![]);
    m.set_attribute(node, qual_name!("href"), "detached.png");
    drop(m);
    assert!(provider.urls().is_empty(), "a detached element waits");
    doc.mutate().append_children(html, &[node]);
    assert_eq!(provider.urls().len(), 1);
    // Disconnecting cancels the request, as Blink clears its resource.
    doc.mutate().remove_node(node);
    assert!(doc.pending_images.is_empty());
    assert!(doc.fe_images.requests.is_empty());
}

#[test]
fn equal_data_urls_share_one_cached_decode() {
    let provider = Arc::new(Deferred::default());
    let (mut doc, html) = document(provider.clone());
    let pixels = |doc: &BaseDocument, node| match target(doc, node) {
        FeImageTarget::Fetched(Some(ImageData::Raster(raster))) => raster.data.data().as_ptr(),
        _ => panic!("a decoded raster"),
    };
    let first = fe_image(&mut doc, html, &data_png());
    let second = fe_image(&mut doc, html, &data_png());
    assert_eq!(pixels(&doc, first), pixels(&doc, second));
}

#[test]
fn draws_bound_the_data_images_they_decode_themselves() {
    let provider = Arc::new(Deferred::default());
    let (mut doc, _) = document(provider);
    // Detached, so no update decodes it: a draw decodes it within its budget.
    let mut m = doc.mutate();
    let node = m.create_element(svg_name("feImage"), vec![]);
    m.set_attribute(node, qual_name!("href"), data_png());
    drop(m);
    let decoded = |budget| {
        let mut inline = super::InlineDecodes(budget);
        matches!(
            doc.fe_image_target(node, &mut inline),
            FeImageTarget::Fetched(Some(_))
        )
    };
    assert!(decoded(4), "a 1×1 image takes four bytes");
    assert!(!decoded(3));
}

#[test]
fn bare_fragments_name_elements_whatever_the_base_url() {
    let provider = Arc::new(Deferred::default());
    let (mut doc, html) = document(provider.clone());
    let mut m = doc.mutate();
    let base = m.create_element(qual_name!("base", html), vec![]);
    m.set_attribute(base, qual_name!("href"), "https://cdn.test/assets/");
    m.append_children(html, &[base]);
    drop(m);
    let node = fe_image(&mut doc, html, "#shape");
    assert!(matches!(target(&doc, node), FeImageTarget::Fragment(id) if id == "shape"));
    assert!(provider.urls().is_empty());
}

#[test]
fn failed_requests_are_fetched_again() {
    let provider = Arc::new(Deferred::default());
    let (mut doc, html) = document(provider.clone());
    let node = fe_image(&mut doc, html, "flaky.png");
    provider.take().failed();
    doc.handle_messages();
    doc.mutate()
        .set_attribute(node, qual_name!("href"), "flaky.png");
    assert_eq!(provider.urls().len(), 1, "setting href again refetches");
    answer(provider.take(), "https://page.test/flaky.png", true);
    doc.handle_messages();
    assert_eq!(loaded(&doc, node), Some(true));
}

#[test]
fn images_the_cache_refuses_are_retained_within_a_budget() {
    let provider = Arc::new(Deferred::default());
    let (mut doc, html) = document(provider);
    for index in 0..crate::image_request::MAX_CACHED_IMAGES {
        let key = crate::image_request::ImageKey::no_cors(format!("https://filler.test/{index}"));
        doc.image_cache.insert(key, ImageData::None);
    }
    let node = fe_image(&mut doc, html, &data_png());
    assert_eq!(loaded(&doc, node), Some(true));
    assert_eq!(doc.fe_images.owned_bytes, 4);
    doc.mutate().remove_and_drop_node(node);
    assert_eq!(doc.fe_images.owned_bytes, 0);
}

#[test]
fn retained_svg_documents_are_charged_by_size() {
    let provider = Arc::new(Deferred::default());
    let (mut doc, html) = document(provider);
    for index in 0..crate::image_request::MAX_CACHED_IMAGES {
        let key = crate::image_request::ImageKey::no_cors(format!("https://filler.test/{index}"));
        doc.image_cache.insert(key, ImageData::None);
    }
    let rects = "<rect width='1' height='1'/>".repeat(10);
    let svg = format!(
        "data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg' width='4' height='4'>{rects}</svg>"
    );
    let node = fe_image(&mut doc, html, &svg);
    assert!(loaded(&doc, node).is_some());
    assert!(doc.fe_images.owned_bytes >= 10 * 1024);
}

#[test]
fn a_failed_request_joins_a_fetch_another_element_started() {
    let provider = Arc::new(Deferred::default());
    let (mut doc, html) = document(provider.clone());
    let node = fe_image(&mut doc, html, "shared.png");
    provider.take().failed();
    doc.handle_messages();
    let other = fe_image(&mut doc, html, "shared.png");
    assert_eq!(provider.urls().len(), 1);
    doc.mutate()
        .set_attribute(node, qual_name!("href"), "shared.png");
    assert_eq!(provider.urls().len(), 1, "joins the pending fetch");
    answer(provider.take(), "https://page.test/shared.png", true);
    doc.handle_messages();
    assert_eq!(loaded(&doc, node), Some(true));
    assert_eq!(loaded(&doc, other), Some(true));
}
