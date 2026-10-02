use crate::{BaseDocument, DocumentConfig, NodeId, QualName, ScrollBehavior, ns};

fn element(doc: &mut BaseDocument, parent: NodeId, name: &str, css: &str) -> NodeId {
    let id = {
        let mut mutation = doc.mutate();
        let id = mutation.create_element(QualName::new(None, ns!(html), name.into()), vec![]);
        mutation.append_children(parent, &[id]);
        id
    };
    doc.mutate()
        .set_attribute(id, QualName::new(None, ns!(), "style".into()), css);
    id
}
fn document() -> (BaseDocument, NodeId) {
    let mut doc = BaseDocument::new(DocumentConfig::default());
    let root = doc.root_node().id;
    let html = element(&mut doc, root, "html", "margin:0;padding:0");
    let body = element(&mut doc, html, "body", "margin:0;padding:0");
    (doc, body)
}
#[test]
fn hidden_descendants_and_detached_nodes_have_no_rectangles() {
    let (mut doc, body) = document();
    let parent = element(&mut doc, body, "div", "width:100px");
    let child = element(&mut doc, parent, "div", "width:30px;height:20px");
    doc.resolve(0.0);
    assert_eq!(doc.node_client_rects(child).len(), 1);
    doc.set_style_property(parent, "display", "none");
    doc.resolve(0.0);
    assert!(doc.node_client_rects(parent).is_empty());
    assert!(doc.node_client_rects(child).is_empty());
    let detached = doc
        .mutate()
        .create_element(QualName::new(None, ns!(html), "div".into()), vec![]);
    assert!(doc.node_client_rects(detached).is_empty());
}
#[test]
fn own_scroll_moves_content_and_not_the_scroller_box() {
    let (mut doc, body) = document();
    let parent = element(
        &mut doc,
        body,
        "div",
        "width:100px;height:50px;padding:5px;border:2px solid;overflow:hidden",
    );
    let child = element(&mut doc, parent, "div", "width:300px;height:150px");
    doc.resolve(0.0);
    let before = doc.get_client_bounding_rect(parent).unwrap();
    let content = doc.get_client_bounding_rect(child).unwrap();
    doc.scroll_to(parent, 30.0, 20.0, ScrollBehavior::Instant);
    assert_eq!(doc.get_client_bounding_rect(parent).unwrap(), before);
    let after = doc.get_client_bounding_rect(child).unwrap();
    assert_eq!((after.x, after.y), (content.x - 30.0, content.y - 20.0));
}
#[test]
fn transforms_project_boxes_without_changing_layout_size() {
    let (mut doc, body) = document();
    let parent = element(
        &mut doc,
        body,
        "div",
        "width:100px;height:50px;transform:translate(5px,6px)",
    );
    let child = element(
        &mut doc,
        parent,
        "div",
        "width:20px;height:10px;transform:translate(10px,15px) scale(2)",
    );
    doc.resolve(0.0);
    let rect = doc.get_client_bounding_rect(child).unwrap();
    assert_eq!(
        (rect.x, rect.y, rect.width, rect.height),
        (5.0, 16.0, 40.0, 20.0)
    );
    assert_eq!(doc.get_node(child).unwrap().final_layout().size.width, 20.0);
}

#[test]
fn cssom_hidden_scroll_and_nested_alignment_follow_box_lifecycle() {
    use crate::ScrollLogicalPosition;
    let (mut doc, body) = document();
    let outer = element(
        &mut doc,
        body,
        "div",
        "position:relative;width:200px;height:100px;overflow:hidden",
    );
    let inner = element(
        &mut doc,
        outer,
        "div",
        "position:relative;margin-top:200px;width:150px;height:80px;overflow:hidden",
    );
    element(&mut doc, inner, "div", "height:120px");
    let target = element(&mut doc, inner, "div", "height:20px;width:20px");
    element(&mut doc, inner, "div", "height:120px");
    element(&mut doc, outer, "div", "height:200px");
    doc.resolve(0.0);
    doc.scroll_into_view(
        target,
        ScrollBehavior::Instant,
        ScrollLogicalPosition::Start,
        ScrollLogicalPosition::Nearest,
    );
    assert_eq!(doc.cssom_scroll_position(Some(outer)).y, 200.0);
    assert_eq!(doc.cssom_scroll_position(Some(inner)).y, 120.0);
    doc.set_style_property(outer, "display", "none");
    doc.resolve(0.0);
    assert_eq!(doc.cssom_scroll_position(Some(inner)).y, 0.0);
    doc.scroll_cssom(Some(inner), None, Some(1.0), false, ScrollBehavior::Instant);
    doc.set_style_property(outer, "display", "block");
    doc.resolve(0.0);
    assert_eq!(doc.cssom_scroll_position(Some(inner)).y, 120.0);
}

#[test]
fn nested_smooth_scrolls_advance_together_and_interrupt_per_target() {
    use crate::ScrollLogicalPosition;
    use std::sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    };
    struct Clock(AtomicU64);
    impl crate::Clock for Clock {
        fn now_ms(&self) -> f64 {
            self.0.load(Ordering::Relaxed) as f64
        }
    }
    let clock = Arc::new(Clock(AtomicU64::new(0)));
    let (mut doc, body) = document();
    doc.clock = clock.clone();
    let outer = element(
        &mut doc,
        body,
        "div",
        "position:relative;width:200px;height:100px;overflow:hidden",
    );
    let inner = element(
        &mut doc,
        outer,
        "div",
        "position:relative;margin-top:200px;width:150px;height:80px;overflow:hidden",
    );
    element(&mut doc, inner, "div", "height:120px");
    let target = element(&mut doc, inner, "div", "height:20px;width:20px");
    element(&mut doc, inner, "div", "height:120px");
    element(&mut doc, outer, "div", "height:200px");
    doc.resolve(0.0);
    doc.scroll_into_view(
        target,
        ScrollBehavior::Smooth,
        ScrollLogicalPosition::Start,
        ScrollLogicalPosition::Nearest,
    );
    assert_eq!(doc.cssom_scroll_position(Some(inner)).y, 0.0);
    assert_eq!(doc.cssom_scroll_position(Some(outer)).y, 0.0);
    clock.0.store(150, Ordering::Relaxed);
    doc.resolve_scroll_animation();
    assert_eq!(doc.cssom_scroll_position(Some(inner)).y, 60.0);
    assert_eq!(doc.cssom_scroll_position(Some(outer)).y, 100.0);
    clock.0.store(300, Ordering::Relaxed);
    doc.resolve_scroll_animation();
    assert_eq!(doc.cssom_scroll_position(Some(inner)).y, 120.0);
    assert_eq!(doc.cssom_scroll_position(Some(outer)).y, 200.0);
    doc.scroll_to(inner, 0.0, 0.0, ScrollBehavior::Smooth);
    doc.scroll_to(outer, 0.0, 0.0, ScrollBehavior::Smooth);
    doc.scroll_to(inner, 0.0, 30.0, ScrollBehavior::Instant);
    clock.0.store(600, Ordering::Relaxed);
    doc.resolve_scroll_animation();
    assert_eq!(doc.cssom_scroll_position(Some(inner)).y, 30.0);
    assert_eq!(doc.cssom_scroll_position(Some(outer)).y, 0.0);
}

#[test]
fn fixed_box_retains_viewport_position_after_scroll() {
    let (mut doc, body) = document();
    element(&mut doc, body, "div", "height:2000px");
    let fixed = element(
        &mut doc,
        body,
        "div",
        "position:fixed;top:20px;left:10px;width:40px;height:30px",
    );
    doc.resolve(0.0);
    let root = doc.root_element().id;
    doc.scroll_to(root, 0.0, 100.0, ScrollBehavior::Instant);
    assert_eq!(doc.get_client_bounding_rect(fixed).unwrap().y, 20.0);
}

#[test]
fn root_scroll_into_view_reaches_viewport_in_both_document_modes() {
    use crate::ScrollLogicalPosition;
    for quirks in [false, true] {
        let (mut doc, body) = document();
        let document = doc.root_node().id;
        doc.mutate().document_metadata_mut(document).quirks = quirks;
        element(&mut doc, body, "div", "height:2000px");
        doc.resolve(0.0);
        let root = doc.root_element().id;
        doc.scroll_to(root, 0.0, 100.0, ScrollBehavior::Instant);
        assert_eq!(doc.viewport_scroll().y, 100.0);
        doc.scroll_into_view(
            root,
            ScrollBehavior::Instant,
            ScrollLogicalPosition::Start,
            ScrollLogicalPosition::Nearest,
        );
        assert_eq!(doc.viewport_scroll().y, 0.0);
    }
}
