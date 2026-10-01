//! Image-button lifecycle uses queued local responses, never the network.
mod common;
use blitz_dom::{BaseDocument, DocumentConfig};
use blitz_html::DocumentHtmlParser;
use blitz_traits::net::{Bytes, NetHandler, NetProvider, Request};
use common::q;
use std::sync::{Arc, Mutex};

const SVG: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"/>"#;
#[derive(Default)]
struct Replay(Mutex<Vec<(String, Box<dyn NetHandler>)>>);
impl NetProvider for Replay {
    fn fetch(&self, _: usize, request: Request, handler: Box<dyn NetHandler>) {
        self.0
            .lock()
            .unwrap()
            .push((request.url.to_string(), handler));
    }
}
impl Replay {
    fn complete(&self, suffix: &str) {
        self.complete_with(suffix, SVG);
    }
    fn complete_with(&self, suffix: &str, data: &'static [u8]) {
        let (url, handler) = {
            let mut pending = self.0.lock().unwrap();
            let index = pending
                .iter()
                .position(|(url, _)| url.ends_with(suffix))
                .unwrap();
            pending.remove(index)
        };
        handler.bytes(url, Bytes::from_static(data));
    }
    fn count(&self) -> usize {
        self.0.lock().unwrap().len()
    }
}
fn document(html: &str) -> (BaseDocument, Arc<Replay>) {
    let net = Arc::new(Replay::default());
    let mut doc = BaseDocument::new(DocumentConfig {
        base_url: Some("https://example.test/".into()),
        net_provider: Some(net.clone()),
        ..Default::default()
    });
    DocumentHtmlParser::parse_into_mutator(&mut doc.mutate(), html);
    (doc, net)
}

#[test]
fn loaded_dimensions_follow_layout_and_hidden_detached_fallbacks() {
    let (mut doc, net) = document(
        "<div id=p><input id=i type=image src=a.svg width=40 height=30 style='width:90px;height:50px;border:5px solid;padding:7px'></div>",
    );
    let id = q(&doc, "#i");
    doc.resolve(0.0);
    assert_eq!(doc.image_input_dimensions(id), (40, 30));
    net.complete("a.svg");
    doc.resolve(0.0);
    assert_eq!(doc.image_input_dimensions(id), (90, 50));
    doc.mutate().set_style_property(id, "display", "contents");
    doc.resolve(0.0);
    assert_eq!(doc.image_input_dimensions(id), (40, 30));
    doc.mutate().remove_style_property(id, "display");
    let parent = q(&doc, "#p");
    doc.mutate().set_style_property(parent, "display", "none");
    doc.resolve(0.0);
    assert_eq!(doc.image_input_dimensions(id), (40, 30));
    doc.mutate().remove_node(id);
    doc.resolve(0.0);
    assert_eq!(doc.image_input_dimensions(id), (40, 30));
    for name in ["width", "height"] {
        doc.mutate().remove_attribute_by_name(id, name);
    }
    assert_eq!(doc.image_input_dimensions(id), (1, 1));
    doc.mutate()
        .set_attribute_by_name(id, "type", "text")
        .unwrap();
    assert_eq!(doc.image_input_dimensions(id), (0, 0));
    doc.mutate()
        .set_attribute_by_name(id, "type", "image")
        .unwrap();
    assert_eq!(doc.image_input_dimensions(id), (1, 1));
    doc.mutate()
        .set_attribute_by_name(id, "src", "b.svg")
        .unwrap();
    assert_eq!(doc.image_input_dimensions(id), (1, 1));
    assert_eq!(
        net.count(),
        1,
        "detached input starts a replacement request"
    );
    doc.mutate().remove_attribute_by_name(id, "src");
    assert_eq!(doc.image_input_dimensions(id), (0, 0));
}

#[test]
fn source_and_type_changes_reject_stale_completions_and_deduplicate_waiters() {
    let (mut doc, net) = document("<input id=i type=text src=a.svg>");
    let id = q(&doc, "#i");
    assert_eq!(net.count(), 0);
    doc.mutate()
        .set_attribute_by_name(id, "type", "IMAGE")
        .unwrap();
    for _ in 0..100 {
        doc.mutate()
            .set_attribute_by_name(id, "src", "a.svg")
            .unwrap();
    }
    assert_eq!(net.count(), 1);
    doc.mutate()
        .set_attribute_by_name(id, "src", "b.svg")
        .unwrap();
    net.complete("a.svg");
    doc.handle_messages();
    assert_eq!(doc.image_input_dimensions(id), (0, 0));
    doc.mutate()
        .set_attribute_by_name(id, "type", "text")
        .unwrap();
    net.complete("b.svg");
    doc.handle_messages();
    assert!(
        doc.get_node(id)
            .unwrap()
            .element_data()
            .unwrap()
            .image_data()
            .is_none()
    );
    doc.mutate()
        .set_attribute_by_name(id, "type", "image")
        .unwrap();
    net.complete("b.svg");
    doc.handle_messages();
    doc.mutate().remove_node(id);
    assert_eq!(doc.image_input_dimensions(id), (1, 1));
}

#[test]
fn detached_source_replacement_and_failure_update_intrinsic_dimensions() {
    let (mut doc, net) = document("<input id=i type=image src=a.svg>");
    let id = q(&doc, "#i");
    net.complete("a.svg");
    doc.handle_messages();
    doc.mutate().remove_node(id);
    doc.mutate()
        .set_attribute_by_name(id, "src", "b.svg")
        .unwrap();
    assert_eq!(doc.image_input_dimensions(id), (1, 1));
    net.complete_with(
        "b.svg",
        br#"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="3"/>"#,
    );
    doc.handle_messages();
    assert_eq!(doc.image_input_dimensions(id), (2, 3));
    doc.mutate()
        .set_attribute_by_name(id, "src", "bad.svg")
        .unwrap();
    net.complete_with("bad.svg", b"invalid image");
    doc.handle_messages();
    assert_eq!(doc.image_input_dimensions(id), (0, 0));
}

#[test]
fn superseded_same_url_completions_do_not_consume_replacement_waiters() {
    for first in [SVG, b"invalid image".as_slice()] {
        let (mut doc, net) = document("<input id=i type=image src=a.svg>");
        let id = q(&doc, "#i");
        doc.mutate()
            .set_attribute_by_name(id, "src", "b.svg")
            .unwrap();
        doc.mutate()
            .set_attribute_by_name(id, "src", "a.svg")
            .unwrap();
        net.complete_with("a.svg", first);
        doc.handle_messages();
        assert_eq!(doc.image_input_dimensions(id), (0, 0));
        net.complete_with(
            "a.svg",
            br#"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="3"/>"#,
        );
        doc.handle_messages();
        doc.mutate().remove_node(id);
        assert_eq!(doc.image_input_dimensions(id), (2, 3));
        net.complete("b.svg");
        doc.handle_messages();
        assert_eq!(doc.image_input_dimensions(id), (2, 3));
        doc.mutate()
            .set_attribute_by_name(id, "src", "http://[")
            .unwrap();
        assert_eq!(doc.image_input_dimensions(id), (2, 3));
        doc.handle_messages();
        assert_eq!(doc.image_input_dimensions(id), (0, 0));
    }
}

#[test]
fn restoring_image_type_preserves_cached_image_after_invalid_source_in_text_state() {
    let (mut doc, net) = document("<input id=i type=image src=a.svg>");
    let id = q(&doc, "#i");
    net.complete("a.svg");
    doc.handle_messages();
    doc.mutate().remove_node(id);
    doc.mutate()
        .set_attribute_by_name(id, "type", "text")
        .unwrap();
    doc.mutate()
        .set_attribute_by_name(id, "src", "http://[")
        .unwrap();
    doc.mutate()
        .set_attribute_by_name(id, "type", "image")
        .unwrap();
    doc.handle_messages();
    assert_eq!(doc.image_input_dimensions(id), (1, 1));
    doc.mutate()
        .set_attribute_by_name(id, "src", "http://[")
        .unwrap();
    assert_eq!(doc.image_input_dimensions(id), (1, 1));
    doc.handle_messages();
    assert_eq!(doc.image_input_dimensions(id), (0, 0));
}

#[test]
fn unchanged_image_type_or_reattachment_preserves_queued_source_failure() {
    for reattach in [false, true] {
        let (mut doc, net) = document("<body><input id=i type=image src=a.svg></body>");
        let id = q(&doc, "#i");
        net.complete("a.svg");
        doc.handle_messages();
        doc.mutate().remove_node(id);
        doc.mutate()
            .set_attribute_by_name(id, "src", "http://[")
            .unwrap();
        if reattach {
            let body = q(&doc, "body");
            doc.mutate().append_children(body, &[id]);
        } else {
            doc.mutate()
                .set_attribute_by_name(id, "type", "image")
                .unwrap();
        }
        assert!(
            doc.get_node(id)
                .unwrap()
                .element_data()
                .unwrap()
                .image_data()
                .is_some()
        );
        doc.handle_messages();
        assert!(
            doc.get_node(id)
                .unwrap()
                .element_data()
                .unwrap()
                .image_data()
                .is_none()
        );
    }
}
