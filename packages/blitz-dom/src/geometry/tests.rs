use crate::{BaseDocument, DocumentConfig, NodeId, QualName, ScrollBehavior, ns};

pub(super) fn element(doc: &mut BaseDocument, parent: NodeId, name: &str, css: &str) -> NodeId {
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
pub(super) fn document() -> (BaseDocument, NodeId) {
    let mut doc = BaseDocument::new(DocumentConfig::default());
    let root = doc.root_node().id;
    let html = element(&mut doc, root, "html", "margin:0;padding:0");
    let body = element(&mut doc, html, "body", "margin:0;padding:0");
    (doc, body)
}

#[test]
fn initial_containing_block_retains_absolute_overflow_and_viewport_fixed_position() {
    let (mut doc, body) = document();
    let mut viewport = doc.viewport().clone();
    viewport.window_size = (800, 600);
    doc.set_viewport(viewport);
    let target = element(
        &mut doc,
        body,
        "div",
        "position:absolute;top:1400px;width:80px;height:30px",
    );
    let fixed = element(
        &mut doc,
        body,
        "div",
        "position:fixed;bottom:0;width:80px;height:30px",
    );
    doc.resolve(0.0);
    assert_eq!(doc.get_client_bounding_rect(fixed).unwrap().y, 570.0);
    doc.scroll_into_view(
        target,
        ScrollBehavior::Instant,
        crate::ScrollLogicalPosition::Nearest,
        crate::ScrollLogicalPosition::Nearest,
    );
    let rect = doc.get_client_bounding_rect(target).unwrap();
    assert_eq!(rect.y, 570.0);
    assert_eq!(doc.viewport_scroll().y, 830.0);
    assert_eq!(doc.element_from_point(40.0, 585.0), Some(target));
    assert_eq!(doc.get_client_bounding_rect(fixed).unwrap().y, 570.0);
    doc.resolve(0.0);
    assert_eq!(doc.get_client_bounding_rect(target).unwrap().y, 570.0);
}

#[test]
fn oversized_embedding_scrolls_the_child_rectangle_instead_of_the_host() {
    let (mut doc, body) = document();
    let mut viewport = doc.viewport().clone();
    viewport.window_size = (800, 600);
    doc.set_viewport(viewport);
    doc.embedder_loads_iframes = true;
    let host = element(
        &mut doc,
        body,
        "iframe",
        "width:100px;height:1000px;border:0",
    );
    doc.resolve(0.0);
    let rect = doc
        .scroll_embedded_rect_into_view(host, kurbo::Rect::new(10.0, 900.0, 90.0, 930.0))
        .unwrap();
    assert_eq!(rect.y0, 570.0);
    assert_eq!(rect.y1, 600.0);
    assert!(doc.viewport_scroll().y > 0.0);
}

#[test]
fn viewport_fixed_boxes_do_not_expand_document_scrolling() {
    let (mut doc, body) = document();
    let mut viewport = doc.viewport().clone();
    viewport.window_size = (800, 600);
    doc.set_viewport(viewport);
    let fixed = element(
        &mut doc,
        body,
        "div",
        "position:fixed;top:1400px;width:80px;height:30px",
    );
    doc.resolve(0.0);
    doc.scroll_viewport_by(0.0, 2000.0);
    assert_eq!(doc.viewport_scroll().y, 0.0);
    assert_eq!(doc.get_client_bounding_rect(fixed).unwrap().y, 1400.0);
    doc.resolve(0.0);
    doc.scroll_viewport_by(0.0, 2000.0);
    assert_eq!(doc.viewport_scroll().y, 0.0);
}

#[test]
fn initial_fixed_and_absolute_boxes_retain_equal_z_tree_order() {
    for fixed_first in [true, false] {
        let (mut doc, body) = document();
        let mut viewport = doc.viewport().clone();
        viewport.window_size = (800, 600);
        doc.set_viewport(viewport);
        let positions = if fixed_first {
            ["fixed", "absolute"]
        } else {
            ["absolute", "fixed"]
        };
        element(
            &mut doc,
            body,
            "div",
            &format!(
                "position:{};left:0;top:0;width:80px;height:30px",
                positions[0]
            ),
        );
        let last = element(
            &mut doc,
            body,
            "div",
            &format!(
                "position:{};left:0;top:0;width:80px;height:30px",
                positions[1]
            ),
        );
        doc.resolve(0.0);
        assert_eq!(doc.element_from_point(40.0, 15.0), Some(last));
        doc.resolve(0.0);
        assert_eq!(doc.element_from_point(40.0, 15.0), Some(last));
    }
}

#[test]
fn observer_resize_is_logical_untransformed_and_scaled() {
    let (mut doc, body) = document();
    let target = element(
        &mut doc,
        body,
        "div",
        "width:100px;height:50px;padding:5px;border:2px solid;writing-mode:vertical-rl;transform:scale(2)",
    );
    doc.resolve(0.0);
    let geometry = doc.resize_observer_geometry(target);
    assert_eq!(geometry.content, (50.0, 100.0));
    assert_eq!(geometry.border, (64.0, 114.0));
    assert_eq!(
        (geometry.content_rect.width, geometry.content_rect.height),
        (100.0, 50.0)
    );
    let mut viewport = doc.viewport().clone();
    viewport.hidpi_scale = 2.0;
    doc.set_viewport(viewport);
    doc.resolve(0.0);
    assert_eq!(doc.resize_observer_geometry(target).device, (100.0, 200.0));
    doc.set_style_property(target, "display", "none");
    doc.resolve(0.0);
    assert_eq!(doc.resize_observer_geometry(target).content, (0.0, 0.0));
}

#[test]
fn observer_intersection_clips_ancestors_and_requires_explicit_root_ancestry() {
    let (mut doc, body) = document();
    doc.set_viewport(blitz_traits::shell::Viewport::new(
        800,
        600,
        1.0,
        blitz_traits::shell::ColorScheme::Light,
    ));
    let root = element(
        &mut doc,
        body,
        "div",
        "width:100px;height:50px;overflow:hidden",
    );
    let target = element(
        &mut doc,
        root,
        "div",
        "width:20px;height:20px;margin-top:40px",
    );
    let outside = element(&mut doc, body, "div", "width:20px;height:20px");
    doc.resolve(0.0);
    let result = doc.intersection_observer_geometry(target, None, [(0.0, false); 4]);
    assert!(result.intersects);
    assert_eq!(result.ratio, 0.5);
    let result = doc.intersection_observer_geometry(outside, Some(root), [(0.0, false); 4]);
    assert!(!result.intersects);
    assert_eq!(result.bounds.width, 0.0);
    doc.set_style_property(target, "margin-top", "60px");
    doc.resolve(0.0);
    assert!(
        !doc.intersection_observer_geometry(target, None, [(0.0, false); 4])
            .intersects
    );
}

#[test]
fn observer_fractional_sizes_and_transformed_root_margins() {
    let (mut doc, body) = document();
    let root = element(
        &mut doc,
        body,
        "div",
        "position:absolute;left:200px;top:200px;width:100.25px;height:100.75px;overflow:hidden;transform:scale(2);transform-origin:0 0",
    );
    let target = element(
        &mut doc,
        root,
        "div",
        "width:10.25px;height:20.75px;padding:1.25px;border:0.5px solid",
    );
    doc.resolve(0.0);
    let geometry = doc.resize_observer_geometry(target);
    assert_eq!(geometry.content, (10.25, 20.75));
    assert_eq!(geometry.border, (14.75, 25.25));
    let geometry = doc.intersection_observer_geometry(target, Some(root), [(10.0, false); 4]);
    assert_eq!(
        (
            geometry.root.x,
            geometry.root.y,
            geometry.root.width,
            geometry.root.height
        ),
        (180.0, 180.0, 240.5, 241.5)
    );
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

#[test]
fn subdocument_projection_preserves_content_insets_and_nested_transforms() {
    use kurbo::Point;
    let (mut doc, body) = document();
    doc.embedder_loads_iframes = true;
    let wrapper = element(
        &mut doc,
        body,
        "div",
        "position:absolute;left:20px;top:30px;transform:scale(2);transform-origin:0 0",
    );
    let host = element(
        &mut doc,
        wrapper,
        "iframe",
        "position:absolute;left:10px;top:15px;width:100px;height:50px;border:5px solid;padding:3px;transform:rotate(90deg);transform-origin:0 0",
    );
    doc.resolve(0.0);
    let projection = doc.content_box_to_viewport(host).unwrap();
    let origin = projection * Point::ZERO;
    let x = projection * Point::new(1.0, 0.0);
    let y = projection * Point::new(0.0, 1.0);
    assert!(
        (origin.x - 24.0).abs() < 0.01 && (origin.y - 76.0).abs() < 0.01,
        "{origin:?}"
    );
    assert!((x.x - origin.x).abs() < 0.01 && (x.y - origin.y - 2.0).abs() < 0.01);
    assert!((y.x - origin.x + 2.0).abs() < 0.01 && (y.y - origin.y).abs() < 0.01);
    doc.set_style_property(host, "display", "none");
    doc.resolve(0.0);
    assert!(doc.content_box_to_viewport(host).is_none());
}

#[test]
fn ancestor_overflow_clips_frame_hits_even_with_large_scrollable_extent() {
    let (mut doc, body) = document();
    let mut viewport = doc.viewport().clone();
    viewport.window_size = (800, 600);
    doc.set_viewport(viewport);
    doc.embedder_loads_iframes = true;
    let clip = element(
        &mut doc,
        body,
        "div",
        "width:1px;height:1px;overflow:hidden",
    );
    let host = element(&mut doc, clip, "iframe", "width:100px;height:80px;border:0");
    doc.resolve(0.0);
    assert_ne!(doc.element_from_point(40.0, 15.0), Some(host));
    doc.set_style_property(clip, "overflow", "visible");
    doc.resolve(0.0);
    assert_eq!(
        doc.element_from_point(40.0, 15.0),
        Some(host),
        "host rect {:?}",
        doc.get_client_bounding_rect(host)
    );
}

#[test]
fn overflow_clip_preserves_border_box_hit_ownership() {
    let (mut doc, body) = document();
    let mut viewport = doc.viewport().clone();
    viewport.window_size = (800, 600);
    doc.set_viewport(viewport);
    let target = element(
        &mut doc,
        body,
        "div",
        "width:100px;height:100px;border:20px solid;overflow:hidden",
    );
    doc.resolve(0.0);
    assert_eq!(doc.element_from_point(5.0, 5.0), Some(target));
    assert_eq!(doc.element_from_point(135.0, 135.0), Some(target));
}
