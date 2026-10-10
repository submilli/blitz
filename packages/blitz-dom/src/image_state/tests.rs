//! `<img>` request state, staged updates and events, against the Chrome
//! behavior recorded in submilli-browser's `fixtures/local/image-state`.
use super::*;
use crate::{DocumentConfig, qual_name};
use blitz_traits::net::{Bytes, NetHandler, NetProvider, Request};
use std::io::Cursor;
use std::sync::{Arc, Mutex};

/// Holds requests until the test answers them; answers `data:` at once.
#[derive(Default)]
struct Provider(Mutex<Vec<(Request, Box<dyn NetHandler>)>>);
impl NetProvider for Provider {
    fn fetch(&self, _: usize, request: Request, handler: Box<dyn NetHandler>) {
        if request.url.scheme() == "data" {
            handler.bytes(request.url.to_string(), png());
            return;
        }
        self.0.lock().unwrap().push((request, handler));
    }
}
impl Provider {
    fn urls(&self) -> Vec<String> {
        let requests = self.0.lock().unwrap();
        requests.iter().map(|(r, _)| r.url.to_string()).collect()
    }
    fn answer(&self, doc: &mut BaseDocument, bytes: Option<Bytes>) {
        let (request, handler) = self.0.lock().unwrap().remove(0);
        match bytes {
            Some(bytes) => handler.bytes(request.url.to_string(), bytes),
            None => handler.failed(),
        }
        doc.handle_messages();
    }
}

fn png() -> Bytes {
    let mut bytes = Cursor::new(Vec::new());
    image::RgbaImage::from_pixel(3, 2, image::Rgba([10, 20, 30, 255]))
        .write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap();
    Bytes::from(bytes.into_inner())
}

fn document(provider: Arc<Provider>) -> BaseDocument {
    let mut doc = BaseDocument::new(DocumentConfig {
        base_url: Some("https://page.test/".into()),
        net_provider: Some(provider),
        ..Default::default()
    });
    doc.embedder_updates_images = true;
    doc
}

/// The embedder's two stages after a mutation: immediate, then stable.
fn run(doc: &mut BaseDocument, stage: ImageUpdateStage) {
    let root = doc.root_node_id;
    doc.run_image_updates(stage, |document| document == root);
}
fn stable(doc: &mut BaseDocument) {
    run(doc, ImageUpdateStage::StableState);
}

fn image(doc: &mut BaseDocument) -> NodeId {
    doc.mutate().create_element(qual_name!("img", html), vec![])
}

fn set_src(doc: &mut BaseDocument, img: NodeId, src: &str) {
    doc.mutate().set_attribute(img, qual_name!("src"), src);
    run(doc, ImageUpdateStage::Immediate);
}

/// `complete`, natural size and `currentSrc`, as the fixture records them.
fn snap(doc: &BaseDocument, img: NodeId) -> String {
    let status = doc.image_status(img).unwrap();
    let (width, height) = status.natural_size;
    let url = doc.image_current_url(img).replace("https://page.test", "");
    format!("{},{width},{height},{url}", status.complete(true))
}

/// Events still current, as the embedder would dispatch them.
fn events(doc: &mut BaseDocument) -> Vec<(NodeId, ImageEventKind)> {
    doc.take_image_events()
        .into_iter()
        .filter(|event| doc.image_status(event.node).unwrap().generation == event.generation)
        .map(|event| (event.node, event.kind))
        .collect()
}

fn loaded(doc: &mut BaseDocument, provider: &Provider, src: &str) -> NodeId {
    let img = image(doc);
    set_src(doc, img, src);
    stable(doc);
    provider.answer(doc, Some(png()));
    doc.take_image_events();
    img
}

#[test]
fn a_fetch_waits_for_the_stable_state_and_completes_with_a_load() {
    let provider = Arc::new(Provider::default());
    let mut doc = document(provider.clone());
    let img = image(&mut doc);
    assert_eq!(snap(&doc, img), "true,0,0,");
    set_src(&mut doc, img, "/a.png");
    assert_eq!(snap(&doc, img), "false,0,0,");
    assert!(
        provider.urls().is_empty(),
        "no fetch before the stable state"
    );
    stable(&mut doc);
    assert_eq!(provider.urls(), ["https://page.test/a.png"]);
    assert_eq!(snap(&doc, img), "false,0,0,");
    assert!(doc.images_delay_load(doc.root_node_id));
    provider.answer(&mut doc, Some(png()));
    assert_eq!(snap(&doc, img), "true,3,2,/a.png");
    assert_eq!(events(&mut doc), [(img, ImageEventKind::Load)]);
    assert!(!doc.images_delay_load(doc.root_node_id));
}

#[test]
fn mutations_before_the_stable_state_start_one_request() {
    let provider = Arc::new(Provider::default());
    let mut doc = document(provider.clone());
    let img = image(&mut doc);
    set_src(&mut doc, img, "/k1.png");
    set_src(&mut doc, img, "/k2.png");
    stable(&mut doc);
    assert_eq!(provider.urls(), ["https://page.test/k2.png"]);

    // A later update that needs no fetch cancels the deferred one.
    let other = image(&mut doc);
    set_src(&mut doc, other, "/x2.png");
    set_src(&mut doc, other, "");
    stable(&mut doc);
    assert_eq!(provider.urls().len(), 1);
    assert_eq!(snap(&doc, other), "true,0,0,");
    assert_eq!(events(&mut doc), [(other, ImageEventKind::Error)]);
}

#[test]
fn available_images_complete_immediately_and_newer_updates_cancel_events() {
    let provider = Arc::new(Provider::default());
    let mut doc = document(provider.clone());
    let first = loaded(&mut doc, &provider, "/a.png");
    let cached = image(&mut doc);
    set_src(&mut doc, cached, "/a.png");
    assert_eq!(snap(&doc, cached), "true,3,2,/a.png");
    // The same URL again reloads from the list, with another load.
    set_src(&mut doc, first, "/a.png");
    assert_eq!(snap(&doc, first), "true,3,2,/a.png");
    assert_eq!(
        events(&mut doc),
        [
            (cached, ImageEventKind::Load),
            (first, ImageEventKind::Load)
        ]
    );

    // An available image, then one to fetch: only the fetch's load fires,
    // and the old image stays until the stable state.
    set_src(&mut doc, first, "/a.png");
    set_src(&mut doc, first, "/x4.png");
    assert_eq!(snap(&doc, first), "false,3,2,/a.png");
    stable(&mut doc);
    assert_eq!(snap(&doc, first), "false,0,0,");
    provider.answer(&mut doc, Some(png()));
    assert_eq!(events(&mut doc), [(first, ImageEventKind::Load)]);
    assert_eq!(snap(&doc, first), "true,3,2,/x4.png");
}

#[test]
fn removed_and_empty_sources_reset_at_once() {
    let provider = Arc::new(Provider::default());
    let mut doc = document(provider.clone());
    let img = loaded(&mut doc, &provider, "/c.png");
    doc.mutate().clear_attribute(img, qual_name!("src"));
    run(&mut doc, ImageUpdateStage::Immediate);
    assert_eq!(snap(&doc, img), "true,0,0,");
    assert!(events(&mut doc).is_empty(), "no src, no event");

    let empty = loaded(&mut doc, &provider, "/e.png");
    set_src(&mut doc, empty, "");
    assert_eq!(snap(&doc, empty), "true,0,0,");
    let status = doc.image_status(empty).unwrap();
    assert_eq!(status.state, ImageRequestState::Unavailable, "not broken");
    assert_eq!(events(&mut doc), [(empty, ImageEventKind::Error)]);
}

#[test]
fn failures_and_invalid_urls_break_the_image() {
    let provider = Arc::new(Provider::default());
    let mut doc = document(provider.clone());
    let missing = image(&mut doc);
    set_src(&mut doc, missing, "/missing.png");
    stable(&mut doc);
    provider.answer(&mut doc, None);
    assert_eq!(snap(&doc, missing), "true,0,0,/missing.png");
    assert_eq!(
        doc.image_status(missing).unwrap().state,
        ImageRequestState::Broken
    );
    assert_eq!(events(&mut doc), [(missing, ImageEventKind::Error)]);

    let invalid = loaded(&mut doc, &provider, "/v.png");
    set_src(&mut doc, invalid, "http://[");
    assert_eq!(snap(&doc, invalid), "false,3,2,/v.png");
    stable(&mut doc);
    assert_eq!(snap(&doc, invalid), "true,0,0,http://[");
    assert_eq!(
        doc.image_status(invalid).unwrap().state,
        ImageRequestState::Broken
    );
    assert_eq!(events(&mut doc), [(invalid, ImageEventKind::Error)]);
}

#[test]
fn a_superseded_request_completes_nothing() {
    let provider = Arc::new(Provider::default());
    let mut doc = document(provider.clone());
    let img = image(&mut doc);
    set_src(&mut doc, img, "/first.png");
    stable(&mut doc);
    set_src(&mut doc, img, "/second.png");
    stable(&mut doc);
    provider.answer(&mut doc, Some(png()));
    assert_eq!(snap(&doc, img), "false,0,0,");
    assert!(events(&mut doc).is_empty());
    provider.answer(&mut doc, Some(png()));
    assert_eq!(snap(&doc, img), "true,3,2,/second.png");
    assert_eq!(events(&mut doc), [(img, ImageEventKind::Load)]);
}

#[test]
fn synchronous_answers_apply_within_the_update() {
    let provider = Arc::new(Provider::default());
    let mut doc = document(provider);
    let img = image(&mut doc);
    set_src(&mut doc, img, "data:image/png;base64,AA==");
    assert_eq!(snap(&doc, img), "false,0,0,");
    stable(&mut doc);
    assert_eq!(snap(&doc, img), "true,3,2,data:image/png;base64,AA==");
    assert_eq!(events(&mut doc), [(img, ImageEventKind::Load)]);
}

#[test]
fn images_without_a_browsing_context_never_load() {
    let provider = Arc::new(Provider::default());
    let mut doc = document(provider.clone());
    let mut m = doc.mutate();
    let inert = m.try_create_document_node().unwrap();
    let img = m.create_element(qual_name!("img", html), vec![]);
    m.set_node_document(img, inert);
    m.set_attribute(img, qual_name!("src"), "/inert.png");
    drop(m);
    stable(&mut doc);
    assert!(provider.urls().is_empty());
    assert!(events(&mut doc).is_empty());
    assert!(!doc.images_delay_load(inert));

    // Adoption into the page's document is a relevant mutation.
    let root = doc.root_node_id;
    doc.mutate().set_node_document(img, root);
    stable(&mut doc);
    assert_eq!(provider.urls(), ["https://page.test/inert.png"]);
}

#[test]
fn parsed_images_update_on_creation() {
    let provider = Arc::new(Provider::default());
    let mut doc = document(provider.clone());
    let src = crate::Attribute {
        name: qual_name!("src"),
        value: "/parsed.png".into(),
    };
    let img = doc
        .mutate()
        .create_element(qual_name!("img", html), vec![src]);
    assert_eq!(snap(&doc, img), "false,0,0,");
    stable(&mut doc);
    assert_eq!(provider.urls(), ["https://page.test/parsed.png"]);
}

#[test]
fn queued_events_are_bounded() {
    let provider = Arc::new(Provider::default());
    let mut doc = document(provider);
    let img = image(&mut doc);
    for _ in 0..MAX_QUEUED_EVENTS + 10 {
        set_src(&mut doc, img, "");
    }
    assert_eq!(doc.take_image_events().len(), MAX_QUEUED_EVENTS);
}

#[test]
fn dropping_an_image_releases_its_state_and_request() {
    let provider = Arc::new(Provider::default());
    let mut doc = document(provider.clone());
    let img = image(&mut doc);
    set_src(&mut doc, img, "/dropped.png");
    stable(&mut doc);
    assert!(doc.images.states.contains_key(&img));
    doc.drop_node_ignoring_parent(img);
    assert!(doc.images.states.is_empty());
    assert!(doc.current_image_requests.is_empty());
    assert!(doc.pending_images.is_empty());
    provider.answer(&mut doc, Some(png()));
    assert!(doc.take_image_events().is_empty());
}

#[test]
fn without_an_embedder_mutators_run_updates() {
    let provider = Arc::new(Provider::default());
    let mut doc = document(provider.clone());
    doc.embedder_updates_images = false;
    let img = image(&mut doc);
    doc.mutate()
        .set_attribute(img, qual_name!("src"), "/plain.png");
    assert_eq!(provider.urls(), ["https://page.test/plain.png"]);
    provider.answer(&mut doc, Some(png()));
    assert_eq!(snap(&doc, img), "true,3,2,/plain.png");
}

#[test]
fn images_declaring_huge_bitmaps_are_broken_before_decoding() {
    let provider = Arc::new(Provider::default());
    let mut doc = document(provider.clone());
    let img = image(&mut doc);
    set_src(&mut doc, img, "/bomb.png");
    stable(&mut doc);
    provider.answer(&mut doc, Some(crate::image_decode::declared_png(100_000)));
    let status = doc.image_status(img).unwrap();
    assert_eq!(status.state, ImageRequestState::Broken);
    assert_eq!(events(&mut doc), [(img, ImageEventKind::Error)]);
}

#[test]
fn images_sharing_a_fetch_complete_in_request_order() {
    let provider = Arc::new(Provider::default());
    let mut doc = document(provider.clone());
    let images: Vec<NodeId> = (0..32).map(|_| image(&mut doc)).collect();
    for &img in &images {
        set_src(&mut doc, img, "/shared.png");
    }
    stable(&mut doc);
    assert_eq!(provider.urls().len(), 1);
    // A waiter leaving keeps the others in order.
    set_src(&mut doc, images[0], "");
    doc.take_image_events();
    provider.answer(&mut doc, Some(png()));
    let order: Vec<NodeId> = events(&mut doc).into_iter().map(|(node, _)| node).collect();
    assert_eq!(order, images[1..]);
}

#[test]
fn the_immediate_stage_sees_each_mutation_once() {
    let provider = Arc::new(Provider::default());
    let mut doc = document(provider.clone());
    let deferred = image(&mut doc);
    set_src(&mut doc, deferred, "/deferred.png");
    assert!(doc.images.fresh.is_empty());
    assert_eq!(doc.images.deferred, [deferred]);
    // Another element's mutation does not revisit the deferred one.
    let other = image(&mut doc);
    set_src(&mut doc, other, "");
    assert_eq!(doc.images.deferred, [deferred]);
    assert!(!doc.has_image_updates(ImageUpdateStage::Immediate));
    // A new mutation of the deferred element does.
    set_src(&mut doc, deferred, "");
    assert!(!doc.image_status(deferred).unwrap().update_pending);
    assert!(!doc.images_delay_load(doc.root_node_id));
    assert_eq!(snap(&doc, deferred), "true,0,0,");
}

#[test]
fn a_changed_referrer_policy_is_a_relevant_mutation() {
    let provider = Arc::new(Provider::default());
    let mut doc = document(provider.clone());
    let img = loaded(&mut doc, &provider, "/policy.png");
    let revision = doc.image_status(img).unwrap().revision;
    let set = |doc: &mut BaseDocument, value: &str| {
        doc.mutate()
            .set_attribute(img, qual_name!("referrerpolicy"), value);
    };
    set(&mut doc, "no-referrer");
    assert_eq!(doc.image_status(img).unwrap().revision, revision + 1);
    set(&mut doc, "NO-REFERRER");
    assert_eq!(
        doc.image_status(img).unwrap().revision,
        revision + 1,
        "same state"
    );
    // Invalid values share the empty state.
    set(&mut doc, "bogus");
    assert_eq!(doc.image_status(img).unwrap().revision, revision + 2);
    set(&mut doc, "other");
    assert_eq!(doc.image_status(img).unwrap().revision, revision + 2);
    doc.mutate()
        .clear_attribute(img, qual_name!("referrerpolicy"));
    assert_eq!(doc.image_status(img).unwrap().revision, revision + 2);
}
