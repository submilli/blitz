use super::tests::element;
fn document() -> (BaseDocument, NodeId) {
    let (mut doc, body) = super::tests::document();
    let mut viewport = doc.viewport().clone();
    viewport.window_size = (800, 600);
    doc.set_viewport(viewport);
    (doc, body)
}
use crate::{BaseDocument, NodeId, VisibilityBudget};

fn visible(doc: &BaseDocument, target: NodeId) -> bool {
    let geometry = doc.intersection_observer_geometry(target, None, [(0.0, false); 4]);
    doc.intersection_observer_visibility(target, geometry.hit, &mut VisibilityBudget::default())
}

#[test]
fn visibility_tracks_effects_and_admission() {
    let (mut doc, body) = document();
    let target = element(
        &mut doc,
        body,
        "div",
        "width:100px;height:100px;background:green",
    );
    doc.resolve(0.0);
    assert!(visible(&doc, target), "ordinary unoccluded target");
    let geometry = doc.intersection_observer_geometry(target, None, [(0.0, false); 4]);
    assert!(!doc.intersection_observer_visibility(
        target,
        geometry.hit,
        &mut VisibilityBudget::new(0)
    ));
    for (css, expected) in [
        ("opacity:.99", false),
        ("filter:blur(0px)", false),
        ("visibility:hidden", false),
        ("transform:translate(5px)", true),
        ("transform:scale(2)", true),
        ("transform:scale(.5)", false),
        ("transform:scale(2,1)", false),
        ("transform:rotate(10deg)", false),
    ] {
        doc.mutate().set_attribute(
            target,
            crate::qual_name!("style"),
            format!("width:100px;height:100px;{css}"),
        );
        doc.resolve(0.0);
        assert_eq!(visible(&doc, target), expected, "{css}");
    }
}

#[test]
fn visibility_uses_paint_order_and_ignores_pointer_policy() {
    for (css, expected) in [
        ("background:black", false),
        ("background:transparent", false),
        ("background:black;pointer-events:none", false),
        ("background:black;opacity:0", true),
        ("background:black;z-index:-1", true),
        ("background:black;left:200px", true),
    ] {
        let (mut doc, body) = document();
        let target = element(
            &mut doc,
            body,
            "div",
            "position:absolute;width:100px;height:100px",
        );
        element(
            &mut doc,
            body,
            "div",
            &format!("position:absolute;left:0;top:0;width:100px;height:100px;{css}"),
        );
        doc.resolve(0.0);
        assert_eq!(visible(&doc, target), expected, "{css}");
    }
}

#[test]
fn clipped_geometry_and_nested_work_are_admitted() {
    let (mut doc, body) = document();
    let target = element(
        &mut doc,
        body,
        "div",
        "position:absolute;left:0;top:0;width:100px;height:100px;clip-path:inset(10px)",
    );
    doc.resolve(0.0);
    let geometry = doc.intersection_observer_geometry(target, None, [(0.0, false); 4]);
    assert_eq!(geometry.ratio, 0.64);
    assert!(visible(&doc, target));
    let mut parent = body;
    for _ in 0..40 {
        parent = element(
            &mut doc,
            parent,
            "div",
            "overflow:hidden;width:100px;height:100px;transform:translate(0px)",
        );
    }
    element(&mut doc, parent, "div", "width:100px;height:100px");
    doc.resolve(0.0);
    let mut budget = VisibilityBudget::new(1000);
    assert!(!doc.intersection_observer_visibility(target, geometry.hit, &mut budget));
    assert!(!doc.intersection_observer_visibility(target, geometry.hit, &mut budget));
}

#[test]
fn static_ancestor_shapes_and_explicit_root_follow_chrome() {
    let (mut doc, body) = document();
    let parent = element(
        &mut doc,
        body,
        "div",
        "width:100px;height:100px;clip-path:inset(10px)",
    );
    let target = element(
        &mut doc,
        parent,
        "div",
        "position:absolute;width:100px;height:100px",
    );
    doc.resolve(0.0);
    assert_eq!(
        doc.intersection_observer_geometry(target, None, [(0.0, false); 4])
            .ratio,
        0.64
    );
    doc.mutate().set_attribute(
        parent,
        crate::qual_name!("style"),
        "position:relative;width:100px;height:100px;clip-path:inset(10px)",
    );
    doc.resolve(0.0);
    assert_eq!(
        doc.intersection_observer_geometry(target, Some(parent), [(0.0, false); 4])
            .ratio,
        1.0
    );
    doc.mutate().set_attribute(
        target,
        crate::qual_name!("style"),
        "width:100px;height:100px;clip-path:circle(0px)",
    );
    doc.resolve(0.0);
    assert_eq!(
        doc.intersection_observer_geometry(target, None, [(0.0, false); 4])
            .ratio,
        0.0
    );
    assert!(!visible(&doc, target));
}

#[test]
fn geometry_admission_reaches_a_deep_hoisted_candidate_and_stays_exhausted() {
    let (mut doc, body) = document();
    let target = element(
        &mut doc,
        body,
        "div",
        "position:absolute;left:0;top:0;width:100px;height:100px",
    );
    let mut parent = body;
    for _ in 0..40 {
        parent = element(
            &mut doc,
            parent,
            "div",
            "position:absolute;left:200px;overflow:hidden;width:100px;height:100px",
        );
    }
    element(
        &mut doc,
        parent,
        "div",
        "position:absolute;z-index:1;left:-8000px;width:100px;height:100px",
    );
    doc.resolve(0.0);
    let geometry = doc.intersection_observer_geometry(target, None, [(0.0, false); 4]);
    let mut budget = VisibilityBudget::default();
    assert!(!doc.intersection_observer_visibility(target, geometry.hit, &mut budget));
    assert!(!doc.intersection_observer_visibility(target, geometry.hit, &mut budget));
}

#[test]
fn nonsquare_shape_radii_and_generated_paths_are_bounded() {
    let (mut doc, body) = document();
    let target = element(
        &mut doc,
        body,
        "div",
        "width:100px;height:50px;clip-path:circle(20%)",
    );
    doc.resolve(0.0);
    assert!(
        (doc.intersection_observer_geometry(target, None, [(0.0, false); 4])
            .ratio
            - 0.2)
            .abs()
            < 0.001
    );
    doc.mutate().set_attribute(
        target,
        crate::qual_name!("style"),
        "width:100px;height:50px;clip-path:ellipse(closest-side closest-side)",
    );
    doc.resolve(0.0);
    assert_eq!(
        doc.intersection_observer_geometry(target, None, [(0.0, false); 4])
            .ratio,
        1.0
    );
    let repeated = "A100 100 0 1 1 200 0 A100 100 0 1 1 0 0 ".repeat(2000);
    for path in [
        "M-1e30 0 A1e30 1e30 0 1 1 1e30 0".to_owned(),
        format!("M0 0 {repeated}"),
    ] {
        doc.mutate().set_attribute(
            target,
            crate::qual_name!("style"),
            format!("width:100px;height:100px;clip-path:path('{path}')"),
        );
        doc.resolve(0.0);
        let style = doc.get_node(target).unwrap().primary_styles().unwrap();
        let style::values::computed::basic_shape::ClipPath::Shape(shape, _) =
            style.clone_clip_path()
        else {
            panic!("test path parsed")
        };
        assert!(
            crate::basic_shape_to_path(
                &shape,
                crate::ShapeReferenceBox {
                    x: 0.0,
                    y: 0.0,
                    width: 100.0,
                    height: 100.0
                }
            )
            .is_none()
        );
    }
}

#[test]
fn rotated_svg_arc_geometry_matches_the_engine_svg_parser() {
    use kurbo::Shape;
    let (mut doc, body) = document();
    let path = "M10 20 A70 30 35 1 1 130 80 Z";
    let target = element(
        &mut doc,
        body,
        "div",
        &format!("width:200px;height:200px;clip-path:path('{path}')"),
    );
    doc.resolve(0.0);
    let style = doc.get_node(target).unwrap().primary_styles().unwrap();
    let style::values::computed::basic_shape::ClipPath::Shape(shape, _) = style.clone_clip_path()
    else {
        panic!("test path parsed")
    };
    let actual = crate::basic_shape_to_path(
        &shape,
        crate::ShapeReferenceBox {
            x: 0.0,
            y: 0.0,
            width: 200.0,
            height: 200.0,
        },
    )
    .unwrap()
    .bounding_box();
    let expected = kurbo::BezPath::from_svg(path).unwrap().bounding_box();
    for (actual, expected) in [
        (actual.x0, expected.x0),
        (actual.y0, expected.y0),
        (actual.x1, expected.x1),
        (actual.y1, expected.y1),
    ] {
        assert!((actual - expected).abs() < 0.1, "{actual} vs {expected}");
    }
}
