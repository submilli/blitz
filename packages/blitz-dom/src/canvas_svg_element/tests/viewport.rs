//! The viewport, and Blink's viewport dependence.

use super::*;

/// Blink's `HasVisibleViewportDependence`: percentages in visible geometry,
/// relative units where `SVGLength::IsRelative` counts, and all text.
#[test]
fn viewport_dependence_follows_blink() {
    let (mut doc, svg) = page("");
    let mut m = doc.mutate();
    let percent = add(&mut m, svg, "rect", &[("width", "50%"), ("height", "2")]);
    let em_rect = add(&mut m, svg, "rect", &[("width", "1em"), ("height", "2")]);
    let em_line = add(&mut m, svg, "line", &[("x2", "1em"), ("stroke", "red")]);
    let stroked = [
        ("width", "2"),
        ("height", "2"),
        ("stroke", "red"),
        ("stroke-width", "5%"),
    ];
    let stroke = add(&mut m, svg, "rect", &stroked);
    let gradient_only = add(&mut m, svg, "g", &[]);
    add(
        &mut m,
        gradient_only,
        "linearGradient",
        &[("id", "p"), ("x2", "100%")],
    );
    let filled = [("width", "2"), ("height", "2"), ("fill", "url(#p)")];
    add(&mut m, gradient_only, "rect", &filled);
    let instanced = add(&mut m, svg, "g", &[]);
    add(&mut m, instanced, "use", &[("href", "#percent")]);
    add(
        &mut m,
        svg,
        "rect",
        &[("id", "percent"), ("width", "10%"), ("height", "2")],
    );
    let text = add(&mut m, svg, "text", &[]);
    drop(m);
    doc.flush_style_and_layout(0.0);
    for (target, dependent) in [
        (percent, true),
        (em_rect, false),
        (em_line, true),
        (stroke, true),
        (gradient_only, false),
        (instanced, true),
        (text, true),
    ] {
        let plan = crate::canvas_svg_element::plan::Plan::new(&doc, target).unwrap();
        assert_eq!(plan.viewport_dependent, dependent, "{target}");
    }
    let element = doc.capture_svg_element(percent).unwrap().unwrap();
    assert_eq!(element.viewport, Some([300.0, 150.0]));
    let element = doc.capture_svg_element(em_rect).unwrap().unwrap();
    assert_eq!(element.viewport, None);
}

/// Blink's `SVGViewportResolver`: the nearest `viewBox`, else the nearest
/// viewport, the outermost `<svg>`'s being its box (300x150 unsized).
#[test]
fn viewports_resolve_like_blink() {
    let (mut doc, svg) = page("");
    let mut m = doc.mutate();
    let direct = add(&mut m, svg, "rect", &[]);
    let nested = add(&mut m, svg, "svg", &[("width", "50%"), ("height", "20")]);
    let in_nested = add(&mut m, nested, "rect", &[]);
    let boxed = add(
        &mut m,
        nested,
        "svg",
        &[("viewBox", "0 0 7 9"), ("width", "1")],
    );
    let group = add(&mut m, boxed, "g", &[]);
    let in_group = add(&mut m, group, "rect", &[]);
    drop(m);
    doc.flush_style_and_layout(0.0);
    assert_eq!(doc.svg_viewport(direct).unwrap(), Some([300.0, 150.0]));
    assert_eq!(doc.svg_viewport(in_nested).unwrap(), Some([150.0, 20.0]));
    assert_eq!(doc.svg_viewport(in_group).unwrap(), Some([7.0, 9.0]));
    let mut m = doc.mutate();
    m.set_attribute(svg, qual_name!("viewBox"), "0 0 16 8");
    drop(m);
    doc.flush_style_and_layout(0.0);
    assert_eq!(doc.svg_viewport(direct).unwrap(), Some([16.0, 8.0]));
    assert_eq!(doc.svg_viewport(in_nested).unwrap(), Some([8.0, 20.0]));
}

/// A nested viewport's `viewBox` scale is not painted, but percentages
/// inside still resolve against the `viewBox` (Blink's viewport resolver).
#[test]
fn nested_view_boxes_keep_their_percentages() {
    let (mut doc, svg) = page("");
    let mut m = doc.mutate();
    let group = add(&mut m, svg, "g", &[]);
    let nested = [
        ("width", "100"),
        ("height", "100"),
        ("viewBox", "0 0 10 10"),
    ];
    let nested = add(&mut m, group, "svg", &nested);
    add(
        &mut m,
        nested,
        "rect",
        &[("width", "50%"), ("height", "50%")],
    );
    drop(m);
    let element = capture(&mut doc, group).unwrap();
    let bounds = paths(&element)[0].abs_bounding_box();
    assert_eq!((bounds.width(), bounds.height()), (5.0, 5.0));
    // The placed markup keeps the viewBox scale for the bounds.
    assert_eq!(element.bounds.width(), 50.0);
}

/// Undisplayed content has no layout in Blink, so its lengths never make a
/// reference viewport-dependent (`hiddenText` in `element-images.js`).
#[test]
fn undisplayed_content_is_not_viewport_dependent() {
    let (mut doc, svg) = page("");
    let mut m = doc.mutate();
    let group = add(&mut m, svg, "g", &[]);
    add(&mut m, group, "rect", &[("width", "4"), ("height", "4")]);
    add(&mut m, group, "text", &[("style", "display:none")]);
    add(
        &mut m,
        group,
        "rect",
        &[("style", "display:none"), ("width", "50%")],
    );
    drop(m);
    doc.flush_style_and_layout(0.0);
    let plan = crate::canvas_svg_element::plan::Plan::new(&doc, group).unwrap();
    assert!(!plan.viewport_dependent);
}

/// Percentages of an empty viewport resolve to (next to) nothing, as in
/// Chrome (`zeroViewport`), while absolute content paints.
#[test]
fn empty_viewports_resolve_percentages_to_nothing() {
    let (mut doc, svg) = page("");
    let mut m = doc.mutate();
    m.set_attribute(svg, qual_name!("width"), "0");
    m.set_attribute(svg, qual_name!("height"), "0");
    let percent = add(&mut m, svg, "rect", &[("width", "50%"), ("height", "4")]);
    let absolute = add(&mut m, svg, "rect", &[("width", "4"), ("height", "4")]);
    drop(m);
    assert!(capture(&mut doc, percent).unwrap().bounds.width() < 1e-3);
    assert!(capture(&mut doc, absolute).is_some());
}

/// An outermost `<svg>` referenced itself resolves against its own box
/// (Blink's `LayoutSVGRoot::ViewportSize`).
#[test]
fn outermost_svg_targets_use_their_own_box() {
    let (mut doc, svg) = page("");
    doc.flush_style_and_layout(0.0);
    assert_eq!(doc.svg_viewport(svg).unwrap(), Some([300.0, 150.0]));
}

/// An outermost `<svg>` referenced itself sizes against its own box, so its
/// percentage sizes may pass 100%.
#[test]
fn outermost_svg_targets_may_size_past_their_box() {
    let (mut doc, svg) = page("");
    let mut m = doc.mutate();
    m.set_attribute(svg, qual_name!("width"), "150%");
    add(&mut m, svg, "rect", &[("width", "1"), ("height", "1")]);
    drop(m);
    assert!(capture(&mut doc, svg).is_some());
}
