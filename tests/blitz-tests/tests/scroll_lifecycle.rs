//! Scroll offsets belong to rendered boxes, not retained detached DOM nodes.
use blitz_dom::{DocumentConfig, FrozenClock, ScrollBehavior};
use blitz_html::HtmlDocument;
use blitz_traits::node_id::NodeId;
use markup5ever::{LocalName, Namespace, QualName};
use std::sync::Arc;

fn fixture(incremental: bool) -> (HtmlDocument, NodeId, NodeId, NodeId) {
    let mut doc = HtmlDocument::from_html(
        r#"<!doctype html><body style="margin:0"><section id="parent">
        <div id="scroller" style="width:50px;height:50px;overflow:hidden">
        <div id="target" style="width:100px;height:100px;margin-left:60px;margin-top:60px"></div>
        </div></section>
        <div id="other" style="width:50px;height:50px;overflow:hidden">
        <!-- Keep non-layout nodes in the disconnected subtree coverage. -->
        <div style="width:200px;height:200px"></div></div></body>"#,
        DocumentConfig {
            incremental: Some(incremental),
            clock: Some(Arc::new(FrozenClock)),
            ..Default::default()
        },
    );
    doc.resolve(0.0);
    let parent = doc.query_selector("#parent").unwrap().unwrap();
    let scroller = doc.query_selector("#scroller").unwrap().unwrap();
    let target = doc.query_selector("#target").unwrap().unwrap();
    doc.scroll_to(scroller, 60.0, 60.0, ScrollBehavior::Instant);
    assert_position(&doc, scroller, target, 60.0, 0.0);
    (doc, parent, scroller, target)
}

fn assert_position(
    doc: &HtmlDocument,
    scroller: NodeId,
    target: NodeId,
    offset: f64,
    position: f64,
) {
    let scroll = doc.cssom_scroll_position(Some(scroller));
    assert_eq!((scroll.x, scroll.y), (offset, offset));
    let rect = doc.get_client_bounding_rect(target).unwrap();
    let root = doc.get_client_bounding_rect(scroller).unwrap();
    assert_eq!((rect.x - root.x, rect.y - root.y), (position, position));
}

#[test]
fn detached_subtree_loses_scroll_offsets_with_or_without_layout_flush() {
    for incremental in [false, true] {
        for remove_ancestor in [false, true] {
            for flush_detached in [false, true] {
                let (mut doc, parent, scroller, target) = fixture(incremental);
                let removed = if remove_ancestor { parent } else { scroller };
                let original_parent = doc.get_node(removed).unwrap().parent.unwrap();
                doc.mutate().remove_node(removed);
                if flush_detached {
                    doc.resolve(0.0);
                }
                assert_eq!(
                    doc.cssom_scroll_position(Some(scroller)),
                    blitz_dom::Point::ZERO
                );
                assert_eq!(
                    *doc.get_node(scroller).unwrap().scroll_offset(),
                    blitz_dom::Point::ZERO
                );
                assert!(doc.get_client_bounding_rect(target).is_none());
                doc.mutate().append_children(original_parent, &[removed]);
                doc.resolve(0.0);
                assert_position(&doc, scroller, target, 0.0, 60.0);
            }
        }
    }
}

#[test]
fn connected_reparent_discards_descendant_scroll_offset() {
    for incremental in [false, true] {
        let (mut doc, parent, scroller, target) = fixture(incremental);
        let other = doc.query_selector("#other").unwrap().unwrap();
        doc.mutate().append_children(other, &[parent]);
        doc.resolve(0.0);
        assert_position(&doc, scroller, target, 0.0, 60.0);
    }
}

#[test]
fn temporarily_hidden_ancestor_preserves_scroll_offset() {
    for incremental in [false, true] {
        for flush_hidden in [false, true] {
            let (mut doc, parent, scroller, target) = fixture(incremental);
            let style = QualName::new(None, Namespace::from(""), LocalName::from("style"));
            doc.mutate()
                .set_attribute(parent, style.clone(), "display:none");
            if flush_hidden {
                doc.resolve(0.0);
                assert_eq!(
                    doc.cssom_scroll_position(Some(scroller)),
                    blitz_dom::Point::ZERO
                );
            }
            doc.mutate().set_attribute(parent, style, "display:block");
            doc.resolve(0.0);
            assert_position(&doc, scroller, target, 60.0, 0.0);
        }
    }
}

#[test]
fn detaching_document_element_clears_scroll_before_empty_document_return() {
    let (mut doc, _, scroller, target) = fixture(true);
    let root = doc.query_selector("html").unwrap().unwrap();
    let document = doc.get_node(root).unwrap().parent.unwrap();
    doc.mutate().remove_node(root);
    doc.resolve(0.0);
    doc.mutate().append_children(document, &[root]);
    doc.resolve(0.0);
    assert_position(&doc, scroller, target, 0.0, 60.0);
}

#[test]
fn detached_scroller_animation_cannot_restore_discarded_offset() {
    let (mut doc, parent, scroller, target) = fixture(true);
    doc.scroll_to(scroller, 100.0, 100.0, ScrollBehavior::Smooth);
    assert!(doc.has_scroll_animation());
    doc.mutate().remove_node(scroller);
    assert!(!doc.has_scroll_animation());
    doc.mutate().append_children(parent, &[scroller]);
    doc.resolve(0.0);
    assert_position(&doc, scroller, target, 0.0, 60.0);
}

#[test]
fn connected_scroller_keeps_offset_across_layout_flushes() {
    for incremental in [false, true] {
        let (mut doc, _, scroller, target) = fixture(incremental);
        doc.resolve(0.0);
        assert_position(&doc, scroller, target, 60.0, 0.0);
    }
}

#[test]
fn detaching_one_scroller_preserves_other_scroll_animations() {
    let (mut doc, parent, scroller, target) = fixture(true);
    let other = doc.query_selector("#other").unwrap().unwrap();
    doc.scroll_to(scroller, 100.0, 100.0, ScrollBehavior::Smooth);
    doc.scroll_to(other, 100.0, 100.0, ScrollBehavior::Smooth);
    doc.mutate().remove_node(scroller);
    doc.resolve(0.0);
    assert!(doc.has_scroll_animation());
    doc.scroll_to(other, 0.0, 0.0, ScrollBehavior::Instant);
    assert!(!doc.has_scroll_animation());
    doc.mutate().append_children(parent, &[scroller]);
    doc.resolve(0.0);
    assert_position(&doc, scroller, target, 0.0, 60.0);
}
