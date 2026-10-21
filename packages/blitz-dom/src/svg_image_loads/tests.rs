//! SVG images whose `<image>` elements load: fetched ones complete a task
//! after they arrive, `feImage` `data:` ones show what has loaded at once, as
//! submilli-browser's `fixtures/local/canvas-svg-filters/nested*.js` record
//! Chrome doing.
use super::MAX_LOADED_BODIES;
use crate::fe_image::{FeImageTarget, InlineDecodes};
use crate::image_request::ImageKey;
use crate::image_state::{ImageEventKind, ImageRequestState, ImageUpdateStage};
use crate::node::ImageData;
use crate::svg_subresources::BodyDigest;
use crate::{BaseDocument, DocumentConfig, NodeId, QualName, qual_name};
use blitz_traits::net::{Bytes, NetHandler, NetProvider, Request};
use std::io::Cursor;
use std::sync::{Arc, Mutex};

/// Holds every request until the test answers it.
#[derive(Default)]
struct Provider(Mutex<Vec<(Request, Box<dyn NetHandler>)>>);
impl NetProvider for Provider {
    fn fetch(&self, _: usize, request: Request, handler: Box<dyn NetHandler>) {
        self.0.lock().unwrap().push((request, handler));
    }
}
impl Provider {
    /// Answer the oldest request with `body`, as a host answers.
    fn answer(&self, doc: &mut BaseDocument, body: Vec<u8>) {
        let (request, handler) = self.0.lock().unwrap().remove(0);
        handler.bytes(request.url.to_string(), Bytes::from(body));
        doc.handle_messages();
    }
}

fn png(blue: u8) -> String {
    let mut bytes = Cursor::new(Vec::new());
    image::RgbaImage::from_pixel(1, 1, image::Rgba([0, 0, blue, 255]))
        .write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap();
    let bytes = bytes.into_inner();
    let encoded = percent_encoding::percent_encode(&bytes, percent_encoding::NON_ALPHANUMERIC);
    format!("data:image/png,{encoded}")
}

/// An SVG document with a lime rectangle and an `<image>` of `href`.
fn nesting(href: &str) -> String {
    format!(
        "<svg xmlns='http://www.w3.org/2000/svg' width='4' height='4'><rect width='2' height='4' fill='lime'/><image x='2' href='{href}' width='2' height='4'/></svg>"
    )
}

fn data_svg(svg: &str) -> String {
    let encoded = percent_encoding::utf8_percent_encode(svg, percent_encoding::NON_ALPHANUMERIC);
    format!("data:image/svg+xml,{encoded}")
}

/// A document whose embedder runs image updates, as submilli-browser's does.
fn document(provider: Arc<Provider>) -> (BaseDocument, NodeId) {
    let mut doc = BaseDocument::new(DocumentConfig {
        base_url: Some("https://page.test/".into()),
        net_provider: Some(provider),
        ..Default::default()
    });
    doc.embedder_updates_images = true;
    let root = doc.root_node().id;
    let mut m = doc.mutate();
    let html = m.create_element(qual_name!("html", html), vec![]);
    m.pre_insert(html, root, None).unwrap();
    drop(m);
    (doc, html)
}

/// Run the queued image updates, as the embedder does at a stable state.
fn stable(doc: &mut BaseDocument) {
    let root = doc.root_node_id;
    doc.run_image_updates(ImageUpdateStage::StableState, |document| document == root);
}

/// Run the completions queued since the last call, as the embedder's tasks.
fn complete_loads(doc: &mut BaseDocument) -> usize {
    let loads = doc.take_svg_image_loads();
    let count = loads.len();
    for load in loads {
        doc.complete_svg_image_load(load);
    }
    count
}

fn append(doc: &mut BaseDocument, parent: NodeId, name: QualName, url: &str) -> NodeId {
    let attribute = match name.local.as_ref() {
        "img" => qual_name!("src"),
        _ => qual_name!("href"),
    };
    let mut m = doc.mutate();
    let node = m.create_element(name, vec![]);
    m.set_attribute(node, attribute, url);
    m.append_children(parent, &[node]);
    drop(m);
    stable(doc);
    node
}

fn fe_image(doc: &mut BaseDocument, parent: NodeId, href: &str) -> NodeId {
    let name = QualName::new(None, crate::ns!(svg), "feImage".into());
    append(doc, parent, name, href)
}

fn img(doc: &mut BaseDocument, parent: NodeId, src: &str) -> NodeId {
    append(doc, parent, qual_name!("img", html), src)
}

/// The `<image>` images an `feImage`'s SVG image shows, or `None` without one.
fn shown_images(doc: &BaseDocument, node: NodeId) -> Option<usize> {
    match doc.fe_image_target(node, &mut InlineDecodes::default()) {
        FeImageTarget::Fetched(Some(ImageData::Svg(svg))) => Some(images(svg.tree.root())),
        _ => None,
    }
}

fn images(group: &usvg::Group) -> usize {
    group
        .children()
        .iter()
        .map(|node| match node {
            usvg::Node::Group(group) => images(group),
            usvg::Node::Image(_) => 1,
            _ => 0,
        })
        .sum()
}

fn state(doc: &BaseDocument, node: NodeId) -> ImageRequestState {
    doc.image_status(node).unwrap().state
}

#[test]
fn fe_image_data_svg_images_show_unloaded_images_a_task_later() {
    let (mut doc, html) = document(Arc::default());
    let nested = png(201);
    let node = fe_image(&mut doc, html, &data_svg(&nesting(&nested)));
    assert_eq!(shown_images(&doc, node), Some(0), "shown without its image");
    assert_eq!(complete_loads(&mut doc), 1);
    assert_eq!(shown_images(&doc, node), Some(1));
    // Another SVG image showing the loaded image, and the same one again,
    // show it at once.
    let other = format!(
        "<svg xmlns='http://www.w3.org/2000/svg'><image href='{nested}' width='4' height='4'/></svg>"
    );
    let again = fe_image(&mut doc, html, &data_svg(&other));
    let same = fe_image(&mut doc, html, &data_svg(&nesting(&nested)));
    let shown = (shown_images(&doc, again), shown_images(&doc, same));
    assert_eq!(shown, (Some(1), Some(1)));
    assert_eq!(complete_loads(&mut doc), 0);
}

#[test]
fn images_loaded_for_img_show_in_fe_image_svg_images_at_once() {
    let provider = Arc::new(Provider::default());
    let (mut doc, html) = document(provider.clone());
    let data = png(202);
    let preloaded = img(&mut doc, html, &data);
    let (body, _) = data_url::DataUrl::process(&data)
        .unwrap()
        .decode_to_vec()
        .unwrap();
    provider.answer(&mut doc, body);
    assert_eq!(
        state(&doc, preloaded),
        ImageRequestState::CompletelyAvailable
    );
    let node = fe_image(&mut doc, html, &data_svg(&nesting(&data)));
    assert_eq!(shown_images(&doc, node), Some(1));
    assert_eq!(complete_loads(&mut doc), 0);
}

#[test]
fn fetched_svg_images_with_image_elements_complete_a_task_later() {
    let provider = Arc::new(Provider::default());
    let (mut doc, html) = document(provider.clone());
    let plain = img(&mut doc, html, "plain.svg");
    let empty = "<svg xmlns='http://www.w3.org/2000/svg' width='4' height='4'/>";
    provider.answer(&mut doc, empty.into());
    assert_eq!(state(&doc, plain), ImageRequestState::CompletelyAvailable);
    assert_eq!(complete_loads(&mut doc), 0);
    // Even an `<image>` that never loads anything delays the image.
    let first = img(&mut doc, html, "nested.svg");
    provider.answer(&mut doc, nesting("https://elsewhere.test/a.png").into());
    assert_eq!(state(&doc, first), ImageRequestState::Unavailable);
    assert!(doc.images_delay_load(doc.root_node_id));
    // A waiter joining meanwhile completes with it.
    let second = img(&mut doc, html, "nested.svg");
    doc.take_image_events();
    assert_eq!(complete_loads(&mut doc), 1);
    for element in [first, second] {
        assert_eq!(state(&doc, element), ImageRequestState::CompletelyAvailable);
    }
    let events: Vec<_> = doc
        .take_image_events()
        .iter()
        .map(|event| (event.node, event.kind))
        .collect();
    let loads = [
        (first, ImageEventKind::Load),
        (second, ImageEventKind::Load),
    ];
    assert_eq!(events, loads);
    assert!(!doc.images_delay_load(doc.root_node_id));
}

/// Only a readable SVG image's `<image>` images count as loaded: otherwise
/// whether they appear at once would reveal what the opaque document holds.
#[test]
fn only_readable_svg_images_bring_their_images() {
    let nested = png(206);
    for (origin_clean, shown) in [(false, 0), (true, 1)] {
        let provider = Arc::new(Provider::default());
        let (mut doc, html) = document(provider.clone());
        let element = img(&mut doc, html, "nested.svg");
        let (request, handler) = provider.0.lock().unwrap().remove(0);
        let metadata = blitz_traits::net::ResponseMetadata {
            image_origin_clean: origin_clean,
            ..Default::default()
        };
        let body = Bytes::from(nesting(&nested));
        handler.bytes_with_metadata(request.url.to_string(), body, metadata);
        doc.handle_messages();
        complete_loads(&mut doc);
        assert_eq!(state(&doc, element), ImageRequestState::CompletelyAvailable);
        let node = fe_image(&mut doc, html, &data_svg(&nesting(&nested)));
        assert_eq!(shown_images(&doc, node), Some(shown), "{origin_clean}");
    }
}

/// An SVG image shown at once brings its hidden `<image>` images a task later.
#[test]
fn hidden_images_of_an_svg_image_shown_at_once_load_a_task_later() {
    let (mut doc, html) = document(Arc::default());
    let hidden = png(209);
    let svg = format!(
        "<svg xmlns='http://www.w3.org/2000/svg'><image style='display:none' href='{hidden}'/></svg>"
    );
    fe_image(&mut doc, html, &data_svg(&svg));
    let same_turn = fe_image(&mut doc, html, &data_svg(&nesting(&hidden)));
    assert_eq!(shown_images(&doc, same_turn), Some(0));
    // The hidden image's load and the second image's completion.
    assert_eq!(complete_loads(&mut doc), 2);
    let other = format!(
        "<svg xmlns='http://www.w3.org/2000/svg'><image href='{hidden}' width='4' height='4'/></svg>"
    );
    let later = fe_image(&mut doc, html, &data_svg(&other));
    assert_eq!(shown_images(&doc, later), Some(1));
}

/// A `data:` image fetched for another element (a CSS image goes through the
/// network queue) does not hold back an `feImage` of the same URL, and a
/// failed completion leaves the request to be retried.
#[test]
fn fe_image_data_images_do_not_wait_for_other_fetches() {
    let provider = Arc::new(Provider::default());
    let (mut doc, html) = document(provider.clone());
    let plain = data_svg("<svg xmlns='http://www.w3.org/2000/svg' width='4' height='4'/>");
    img(&mut doc, html, &plain);
    let node = fe_image(&mut doc, html, &plain);
    assert_eq!(shown_images(&doc, node), Some(0));
    // Complete: stored, with nothing due.
    assert!(doc.image_cache.contains_key(&ImageKey::no_cors(&plain)));
    assert!(doc.take_svg_image_loads().is_empty());
    // An omitting image joins the pending fetch; its failure leaves it partial
    // and incomplete, so setting the same `href` again decodes it again.
    let nested = data_svg(&nesting(&png(207)));
    let element = img(&mut doc, html, &nested);
    let node = fe_image(&mut doc, html, &nested);
    assert_eq!(shown_images(&doc, node), Some(0));
    let request_id = doc.pending_images[&ImageKey::no_cors(&nested)].request_id;
    doc.fail_image_request(&nested, request_id);
    assert_eq!(shown_images(&doc, node), Some(0));
    assert_eq!(state(&doc, element), ImageRequestState::Broken);
    doc.mutate()
        .set_attribute(node, qual_name!("href"), &nested);
    stable(&mut doc);
    assert_eq!(complete_loads(&mut doc), 1);
    assert_eq!(shown_images(&doc, node), Some(1));
}

/// Changing `href` while the full image is due drops that completion.
#[test]
fn fe_image_href_changes_drop_a_due_completion() {
    let (mut doc, html) = document(Arc::default());
    let first = data_svg(&nesting(&png(208)));
    let node = fe_image(&mut doc, html, &first);
    let loads = doc.take_svg_image_loads();
    assert_eq!(loads.len(), 1, "the full image is due");
    doc.mutate()
        .set_attribute(node, qual_name!("href"), "#none");
    stable(&mut doc);
    for load in loads {
        doc.complete_svg_image_load(load);
    }
    assert!(doc.pending_images.is_empty());
    assert!(doc.image_cache.is_empty());
}

#[test]
fn a_deferred_completion_whose_waiters_left_completes_nothing() {
    let provider = Arc::new(Provider::default());
    let (mut doc, html) = document(provider.clone());
    let element = img(&mut doc, html, "nested.svg");
    provider.answer(&mut doc, nesting(&png(203)).into());
    doc.mutate().clear_attribute(element, qual_name!("src"));
    stable(&mut doc);
    assert_eq!(complete_loads(&mut doc), 1);
    assert_eq!(state(&doc, element), ImageRequestState::Unavailable);
    assert!(doc.pending_images.is_empty());
}

#[test]
fn documents_without_an_embedder_wait_for_nothing() {
    let provider = Arc::new(Provider::default());
    let (mut doc, html) = document(provider.clone());
    doc.embedder_updates_images = false;
    let node = fe_image(&mut doc, html, &data_svg(&nesting(&png(204))));
    assert_eq!(shown_images(&doc, node), Some(1));
    let element = img(&mut doc, html, "nested.svg");
    provider.answer(&mut doc, nesting(&png(205)).into());
    assert_eq!(state(&doc, element), ImageRequestState::CompletelyAvailable);
    assert!(doc.take_svg_image_loads().is_empty());
}

#[test]
fn loaded_bodies_forget_the_oldest_past_their_bound() {
    let (mut doc, _) = document(Arc::default());
    let body = |index: usize| BodyDigest::of(&index.to_le_bytes());
    for index in 0..=MAX_LOADED_BODIES {
        doc.note_loaded_body(body(index));
    }
    let loads = &doc.svg_image_loads;
    assert_eq!(loads.loaded.len(), MAX_LOADED_BODIES);
    assert_eq!(loads.order.len(), MAX_LOADED_BODIES);
    assert!(!loads.loaded.contains(&body(0)));
    assert!(loads.loaded.contains(&body(MAX_LOADED_BODIES)));
}

#[test]
fn canceled_svg_completions_release_waiters_and_ignore_late_tasks() {
    for transferred in [false, true] {
        let provider = Arc::new(Provider::default());
        let (mut doc, html) = document(provider.clone());
        let body = nesting(&png(203));
        let node = img(&mut doc, html, &data_svg(&body));
        provider.answer(&mut doc, body.into_bytes());
        let live = img(&mut doc, html, "https://page.test/pending.png");
        assert_eq!(doc.pending_images.len(), 2);
        assert!(!doc.image_status(node).unwrap().complete(true));
        let tasks = if transferred {
            doc.take_svg_image_loads()
        } else {
            vec![]
        };
        assert_eq!(tasks.len(), usize::from(transferred));
        doc.cancel_svg_image_loads();
        assert!(doc.take_svg_image_loads().is_empty());
        assert_eq!(
            doc.pending_images.len(),
            1,
            "live host request is independent"
        );
        assert_eq!(state(&doc, node), ImageRequestState::Broken);
        assert!(!doc.current_image_requests.contains_key(&node));
        let events = doc.take_image_events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, ImageEventKind::Error);
        for task in tasks {
            doc.complete_svg_image_load(task);
        }
        assert_eq!(state(&doc, node), ImageRequestState::Broken);
        assert!(
            doc.take_image_events().is_empty(),
            "late completion cannot resurrect"
        );
        let data = png(204);
        let (body, _) = data_url::DataUrl::process(&data)
            .unwrap()
            .decode_to_vec()
            .unwrap();
        provider.answer(&mut doc, body);
        assert_eq!(state(&doc, live), ImageRequestState::CompletelyAvailable);
        assert!(doc.pending_images.is_empty());
        doc.mutate().remove_node(node);
        doc.reclaim_detached_nodes(&[]);
        assert!(
            doc.get_node(node).is_none(),
            "canceled image no longer roots its node"
        );
    }
}

#[test]
fn canceled_fetched_svg_response_does_not_complete_or_remember_its_bodies() {
    let provider = Arc::new(Provider::default());
    let (mut doc, html) = document(provider.clone());
    let nested = png(205);
    let body = nesting(&nested);
    let node = img(&mut doc, html, "https://page.test/nested.svg");
    provider.answer(&mut doc, body.into_bytes());
    let tasks = doc.take_svg_image_loads();
    assert_eq!(tasks.len(), 1);
    assert_eq!(doc.pending_images.len(), 1);
    doc.cancel_svg_image_loads();
    assert!(doc.pending_images.is_empty());
    for task in tasks {
        doc.complete_svg_image_load(task);
    }
    assert_eq!(state(&doc, node), ImageRequestState::Broken);
    let next = fe_image(&mut doc, html, &data_svg(&nesting(&nested)));
    assert_eq!(shown_images(&doc, next), Some(0));
    doc.cancel_svg_image_loads();
    assert!(doc.take_svg_image_loads().is_empty());
    assert_eq!(shown_images(&doc, next), Some(0));
}

#[test]
fn cancellation_invalidates_transferred_body_tasks_but_accepts_new_tasks() {
    let (mut doc, html) = document(Arc::default());
    let body = nesting(&png(207));
    let node = fe_image(&mut doc, html, &data_svg(&body));
    let tasks = doc.take_svg_image_loads();
    assert_eq!(tasks.len(), 1);
    doc.cancel_svg_image_loads();
    for task in tasks {
        doc.complete_svg_image_load(task);
    }
    assert_eq!(shown_images(&doc, node), Some(0));
    let next = fe_image(&mut doc, html, &data_svg(&body));
    assert_eq!(shown_images(&doc, next), Some(0));
    assert_eq!(complete_loads(&mut doc), 1);
    assert_eq!(shown_images(&doc, next), Some(1));
}
