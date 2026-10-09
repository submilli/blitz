//! Image request identity: CORS settings, superseded completions, redirects
//! and failures.
use super::ImageKey;
use crate::{BaseDocument, DocumentConfig, NodeId, qual_name};
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
    fn modes(&self) -> Vec<Option<CorsSettings>> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .map(|(r, _)| r.image)
            .collect()
    }
    fn take(&self, index: usize) -> (Request, Box<dyn NetHandler>) {
        self.0.lock().unwrap().remove(index)
    }
}

fn png() -> Bytes {
    let mut bytes = Cursor::new(Vec::new());
    image::RgbaImage::from_pixel(1, 1, image::Rgba([10, 20, 30, 255]))
        .write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap();
    Bytes::from(bytes.into_inner())
}

fn answer(handler: Box<dyn NetHandler>, final_url: &str, origin_clean: bool) {
    handler.bytes_with_metadata(
        final_url.into(),
        png(),
        ResponseMetadata {
            image_origin_clean: origin_clean,
            ..Default::default()
        },
    );
}

fn document(provider: Arc<Deferred>) -> BaseDocument {
    let mut doc = BaseDocument::new(DocumentConfig {
        base_url: Some("https://page.test/".into()),
        net_provider: Some(provider),
        ..Default::default()
    });
    let root = doc.root_node().id;
    let mut m = doc.mutate();
    let html = m.create_element(qual_name!("html", html), vec![]);
    m.pre_insert(html, root, None).unwrap();
    drop(m);
    doc
}

fn connected_img(doc: &mut BaseDocument, crossorigin: Option<&str>) -> NodeId {
    let root = doc.root_element().id;
    let mut m = doc.mutate();
    let img = m.create_element(qual_name!("img", html), vec![]);
    if let Some(value) = crossorigin {
        m.set_attribute(img, qual_name!("crossorigin"), value);
    }
    m.set_attribute(img, qual_name!("src"), "https://cdn.test/a.png");
    m.pre_insert(img, root, None).unwrap();
    img
}

fn origin_clean(doc: &BaseDocument, node: NodeId) -> Option<bool> {
    let element = doc.nodes[node].element_data()?;
    Some(element.raster_image_data()?.origin_clean)
}

#[test]
fn cors_settings_are_part_of_request_and_cache_identity() {
    let provider = Arc::new(Deferred::default());
    let mut doc = document(provider.clone());
    let plain = connected_img(&mut doc, None);
    let anonymous = connected_img(&mut doc, Some("anonymous"));
    let empty = connected_img(&mut doc, Some(""));
    let credentialed = connected_img(&mut doc, Some("USE-CREDENTIALS"));
    assert_eq!(
        provider.modes(),
        [
            Some(CorsSettings::NoCors),
            Some(CorsSettings::Anonymous),
            Some(CorsSettings::UseCredentials),
        ],
        "an invalid or empty value shares the Anonymous request"
    );
    while !provider.0.lock().unwrap().is_empty() {
        let (request, handler) = provider.take(0);
        answer(
            handler,
            request.url.as_str(),
            request.image != Some(CorsSettings::NoCors),
        );
    }
    doc.handle_messages();
    assert_eq!(origin_clean(&doc, plain), Some(false));
    for node in [anonymous, empty, credentialed] {
        assert_eq!(origin_clean(&doc, node), Some(true));
    }
    // A later element reuses the image for its own setting only.
    let again = connected_img(&mut doc, Some("anonymous"));
    let tainted = connected_img(&mut doc, None);
    assert!(provider.modes().is_empty());
    assert_eq!(origin_clean(&doc, again), Some(true));
    assert_eq!(origin_clean(&doc, tainted), Some(false));
}

#[test]
fn a_changed_cors_setting_reloads_and_ignores_the_superseded_completion() {
    let provider = Arc::new(Deferred::default());
    let mut doc = document(provider.clone());
    let img = connected_img(&mut doc, None);
    doc.mutate()
        .set_attribute(img, qual_name!("crossorigin"), "anonymous");
    // The same state under another spelling is not a relevant mutation.
    doc.mutate()
        .set_attribute(img, qual_name!("crossorigin"), "x");
    assert_eq!(
        provider.modes(),
        [Some(CorsSettings::NoCors), Some(CorsSettings::Anonymous)]
    );
    let (cors, cors_handler) = provider.take(1);
    answer(cors_handler, cors.url.as_str(), true);
    doc.handle_messages();
    assert_eq!(origin_clean(&doc, img), Some(true));
    let (plain, plain_handler) = provider.take(0);
    answer(plain_handler, plain.url.as_str(), false);
    doc.handle_messages();
    assert_eq!(
        origin_clean(&doc, img),
        Some(true),
        "a stale no-CORS response cannot replace the current image"
    );
    assert!(doc.pending_images.is_empty());
    // The superseded request was aborted, so its response was not kept:
    // removing the attribute fetches the no-CORS image again.
    doc.mutate().clear_attribute(img, qual_name!("crossorigin"));
    assert_eq!(provider.modes(), [Some(CorsSettings::NoCors)]);
    assert_eq!(origin_clean(&doc, img), None);
}

#[test]
fn redirected_images_complete_under_their_requested_url() {
    let provider = Arc::new(Deferred::default());
    let mut doc = document(provider.clone());
    let img = connected_img(&mut doc, Some("anonymous"));
    let (_, handler) = provider.take(0);
    answer(handler, "https://other.test/final.png", true);
    doc.handle_messages();
    assert_eq!(origin_clean(&doc, img), Some(true));
    let key = ImageKey::new("https://cdn.test/a.png", CorsSettings::Anonymous);
    assert!(doc.image_cache.contains_key(&key));
    assert!(doc.pending_images.is_empty());
}

#[test]
fn a_failed_request_breaks_its_current_image_and_releases_waiters() {
    let provider = Arc::new(Deferred::default());
    let mut doc = document(provider.clone());
    let img = connected_img(&mut doc, None);
    let (request, handler) = provider.take(0);
    answer(handler, request.url.as_str(), false);
    doc.handle_messages();
    assert_eq!(origin_clean(&doc, img), Some(false));
    doc.mutate()
        .set_attribute(img, qual_name!("crossorigin"), "use-credentials");
    let (_, handler) = provider.take(0);
    handler.failed();
    doc.handle_messages();
    assert_eq!(
        origin_clean(&doc, img),
        None,
        "a failed CORS load is broken"
    );
    assert!(doc.pending_images.is_empty());
}

#[test]
fn the_decoded_image_cache_is_bounded() {
    let provider = Arc::new(Deferred::default());
    let mut doc = document(provider);
    let image = || {
        crate::node::ImageData::Raster(crate::node::RasterImageData::new(
            1,
            1,
            Arc::new(vec![0; 4]),
        ))
    };
    for index in 0..(super::MAX_CACHED_IMAGES + 10) {
        doc.cache_image(
            ImageKey::no_cors(format!("https://cdn.test/{index}")),
            &image(),
        );
    }
    assert_eq!(doc.image_cache.len(), super::MAX_CACHED_IMAGES);
    doc.image_cache.clear();
    let large = crate::node::ImageData::Raster(crate::node::RasterImageData::new(
        4096,
        4097,
        Arc::new(vec![0; 4096 * 4097 * 4]),
    ));
    doc.cache_image(ImageKey::no_cors("https://cdn.test/large"), &large);
    assert!(doc.image_cache.is_empty(), "one image over the byte budget");
}

#[test]
fn a_base_url_change_does_not_supersede_an_image_in_flight() {
    let provider = Arc::new(Deferred::default());
    let mut doc = document(provider.clone());
    let root = doc.root_element().id;
    let mut m = doc.mutate();
    let img = m.create_element(qual_name!("img", html), vec![]);
    m.set_attribute(img, qual_name!("crossorigin"), "");
    m.set_attribute(img, qual_name!("src"), "img/a.png");
    m.pre_insert(img, root, None).unwrap();
    drop(m);
    doc.set_base_url("https://page.test/en/other/");
    let (request, handler) = provider.take(0);
    assert_eq!(request.url.as_str(), "https://page.test/img/a.png");
    answer(handler, request.url.as_str(), true);
    doc.handle_messages();
    assert_eq!(origin_clean(&doc, img), Some(true));
    assert!(doc.current_image_requests.is_empty());
}

#[test]
fn removing_src_supersedes_the_request_in_flight() {
    let provider = Arc::new(Deferred::default());
    let mut doc = document(provider.clone());
    let img = connected_img(&mut doc, None);
    doc.mutate().clear_attribute(img, qual_name!("src"));
    let (request, handler) = provider.take(0);
    answer(handler, request.url.as_str(), false);
    doc.handle_messages();
    assert_eq!(origin_clean(&doc, img), None, "the late image is not shown");
    assert!(doc.current_image_requests.is_empty());
}
