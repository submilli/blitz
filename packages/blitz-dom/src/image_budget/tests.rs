use super::{DecodedImageBudget, MIN_CHARGE, MIN_PRUNE_ENTRIES};
use crate::image_state::{ImageEventKind, ImageRequestState};
use crate::node::{RasterImageData, Status};
use crate::{BaseDocument, DocumentConfig, NodeId, qual_name};
use blitz_traits::net::{Bytes, NetHandler, NetProvider, Request, ResponseMetadata};
use std::io::Cursor;
use std::sync::Arc;

fn raster(side: u32) -> RasterImageData {
    let bytes = (side * side * 4) as usize;
    RasterImageData::new(side, side, Arc::new(vec![0; bytes]))
}

#[test]
fn reservations_past_the_limit_are_refused_until_released() {
    let budget = DecodedImageBudget::with_limit(100);
    let first = budget.reserve(60).expect("fits");
    assert!(budget.reserve(50).is_none());
    drop(first);
    assert!(budget.reserve(50).is_some(), "an unused reservation frees");
}

#[test]
fn images_count_once_until_their_last_holder_drops_them() {
    let budget = DecodedImageBudget::with_limit(100);
    let image = raster(4);
    budget.reserve(64).unwrap().track_raster(&image);
    let shared = image.clone();
    assert_eq!(budget.retained_bytes(), 64, "clones share one decode");
    assert!(budget.reserve(64).is_none());
    drop(image);
    assert!(budget.reserve(64).is_none(), "still held by a clone");
    drop(shared);
    assert_eq!(budget.retained_bytes(), 0);
    assert!(budget.reserve(64).is_some());
}

#[test]
fn dropped_images_do_not_accumulate_entries() {
    let budget = DecodedImageBudget::with_limit(usize::MAX);
    let held = raster(1);
    budget.reserve(4).unwrap().track_raster(&held);
    for _ in 0..10_000 {
        budget.reserve(4).unwrap().track_raster(&raster(1));
    }
    let ledger = budget.ledger();
    assert!(
        ledger.entries.len() <= MIN_PRUNE_ENTRIES,
        "{}",
        ledger.entries.len()
    );
    drop(ledger);
    assert_eq!(
        budget.retained_bytes(),
        MIN_CHARGE,
        "tiny images cost the least charge"
    );
}

/// Answers every request at once with a distinct 32×32 PNG (4 KiB decoded),
/// `no-store` when the URL asks for it.
struct Images;
impl NetProvider for Images {
    fn fetch(&self, _: usize, request: Request, handler: Box<dyn NetHandler>) {
        let shade = request.url.path().len() as u8;
        let mut png = Cursor::new(Vec::new());
        image::RgbaImage::from_pixel(32, 32, image::Rgba([shade, 0, 0, 255]))
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        handler.bytes_with_metadata(
            request.url.to_string(),
            Bytes::from(png.into_inner()),
            ResponseMetadata {
                no_store: request.url.query() == Some("no-store"),
                ..Default::default()
            },
        );
    }
}

const DECODED: usize = 32 * 32 * 4;

fn document(budget: &DecodedImageBudget) -> BaseDocument {
    BaseDocument::new(DocumentConfig {
        base_url: Some("https://page.test/".into()),
        net_provider: Some(Arc::new(Images)),
        decoded_image_budget: Some(budget.clone()),
        ..Default::default()
    })
}

fn img(doc: &mut BaseDocument, src: &str) -> NodeId {
    let root = doc.root_node().id;
    let mut mutation = doc.mutate();
    let node = mutation.create_element(qual_name!("img", html), vec![]);
    mutation.append_children(root, &[node]);
    mutation.set_attribute(node, qual_name!("src"), src);
    drop(mutation);
    doc.handle_messages();
    node
}

fn state(doc: &BaseDocument, node: NodeId) -> ImageRequestState {
    doc.image_status(node).unwrap().state
}

#[test]
fn images_past_the_budget_break_and_released_images_free_it() {
    let budget = DecodedImageBudget::with_limit(2 * DECODED);
    let mut doc = document(&budget);
    let first = img(&mut doc, "a.png?no-store");
    let second = img(&mut doc, "bb.png?no-store");
    let third = img(&mut doc, "ccc.png?no-store");
    assert_eq!(state(&doc, first), ImageRequestState::CompletelyAvailable);
    assert_eq!(state(&doc, second), ImageRequestState::CompletelyAvailable);
    assert_eq!(state(&doc, third), ImageRequestState::Broken);
    let events = doc.take_image_events();
    assert!(
        events
            .iter()
            .any(|event| event.node == third && event.kind == ImageEventKind::Error),
        "{events:?}"
    );
    assert_eq!(budget.retained_bytes(), 2 * DECODED);

    doc.mutate().remove_and_drop_node(first);
    let fourth = img(&mut doc, "dddd.png?no-store");
    assert_eq!(state(&doc, fourth), ImageRequestState::CompletelyAvailable);
    assert_eq!(budget.retained_bytes(), 2 * DECODED);
}

#[test]
fn cached_images_stay_charged_and_are_shared_without_a_new_charge() {
    let budget = DecodedImageBudget::with_limit(2 * DECODED);
    let mut doc = document(&budget);
    let first = img(&mut doc, "a.png");
    let _second = img(&mut doc, "bb.png");
    doc.mutate().remove_and_drop_node(first);
    assert_eq!(
        budget.retained_bytes(),
        2 * DECODED,
        "the cache still holds it"
    );
    let again = img(&mut doc, "a.png");
    assert_eq!(state(&doc, again), ImageRequestState::CompletelyAvailable);
    let other = img(&mut doc, "ccc.png");
    assert_eq!(state(&doc, other), ImageRequestState::Broken);
}

#[test]
fn documents_sharing_a_budget_share_its_limit() {
    let budget = DecodedImageBudget::with_limit(DECODED);
    let mut page = document(&budget);
    let mut frame = document(&budget);
    let loaded = img(&mut page, "a.png");
    let refused = img(&mut frame, "bb.png");
    assert_eq!(state(&page, loaded), ImageRequestState::CompletelyAvailable);
    assert_eq!(state(&frame, refused), ImageRequestState::Broken);
}

fn background(doc: &mut BaseDocument, parent: NodeId, src: &str) -> NodeId {
    let mut mutation = doc.mutate();
    let node = mutation.create_element(qual_name!("div", html), vec![]);
    mutation.set_attribute(
        node,
        qual_name!("style"),
        format!("background-image:url({src})"),
    );
    mutation.append_children(parent, &[node]);
    drop(mutation);
    doc.resolve(0.0);
    doc.handle_messages();
    node
}

fn layer_loaded(doc: &BaseDocument, node: NodeId) -> bool {
    let layers = &doc.nodes[node].element_data().unwrap().background_images;
    matches!(layers.first(), Some(Some(layer)) if layer.status == Status::Ok)
}

#[test]
fn css_image_layers_past_the_budget_stay_unloaded() {
    let budget = DecodedImageBudget::with_limit(DECODED);
    let mut doc = document(&budget);
    let root = doc.root_node().id;
    let mut mutation = doc.mutate();
    let html = mutation.create_element(qual_name!("html", html), vec![]);
    mutation.append_children(root, &[html]);
    drop(mutation);
    let first = background(&mut doc, html, "a.png?no-store");
    let second = background(&mut doc, html, "bb.png?no-store");
    assert!(layer_loaded(&doc, first));
    assert!(!layer_loaded(&doc, second));
    assert_eq!(budget.retained_bytes(), DECODED);
}

#[test]
fn pressure_counts_admitted_and_refused_bytes() {
    let budget = DecodedImageBudget::with_limit(100);
    let held = budget.reserve(80).unwrap();
    assert!(budget.reserve(90).is_none());
    drop(held);
    assert_eq!(budget.pressure(), 170);
    assert_eq!(budget.limit(), 100);
}

#[cfg(feature = "svg")]
fn inline_svg(budget: &DecodedImageBudget) -> (BaseDocument, NodeId) {
    let mut doc = document(budget);
    let root = doc.root_node().id;
    let mut mutation = doc.mutate();
    let html = mutation.create_element(qual_name!("html", html), vec![]);
    let svg = mutation.create_element(qual_name!("svg", svg), vec![]);
    let rect = mutation.create_element(qual_name!("rect", svg), vec![]);
    mutation.set_attribute(svg, qual_name!("width"), "4");
    mutation.set_attribute(svg, qual_name!("height"), "4");
    mutation.set_attribute(rect, qual_name!("width"), "4");
    mutation.set_attribute(rect, qual_name!("height"), "4");
    mutation.append_children(svg, &[rect]);
    mutation.append_children(html, &[svg]);
    mutation.append_children(root, &[html]);
    drop(mutation);
    doc.resolve(0.0);
    (doc, svg)
}

#[cfg(feature = "svg")]
fn draws(doc: &BaseDocument, svg: NodeId) -> bool {
    matches!(
        doc.nodes[svg].element_data().unwrap().special_data,
        crate::node::SpecialElementData::Image(_)
    )
}

#[cfg(feature = "svg")]
#[test]
fn inline_svg_trees_are_charged_once_and_past_the_budget_draw_nothing() {
    let budget = DecodedImageBudget::default();
    let (mut doc, svg) = inline_svg(&budget);
    assert!(draws(&doc, svg));
    let charged = budget.retained_bytes();
    assert!(charged > 0);
    for width in ["5", "6", "7"] {
        let pressure = budget.pressure();
        doc.mutate().set_attribute(svg, qual_name!("width"), width);
        doc.resolve(0.0);
        assert!(budget.pressure() > pressure, "the tree was parsed again");
        assert!(draws(&doc, svg));
        assert_eq!(budget.retained_bytes(), charged, "the old tree is released");
    }

    let small = DecodedImageBudget::with_limit(charged - 1);
    let (doc, svg) = inline_svg(&small);
    assert!(!draws(&doc, svg));
    assert_eq!(small.retained_bytes(), 0);
}
