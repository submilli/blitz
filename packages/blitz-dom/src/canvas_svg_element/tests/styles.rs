//! Computed styles, presentation attributes and effects.

use super::*;

#[test]
fn computed_styles_reach_the_painted_tree() {
    let (mut doc, svg) = page(".blue{fill:rgb(0,0,255)} g.outer rect{stroke:rgb(0,255,0)}");
    let mut m = doc.mutate();
    m.set_attribute(svg, qual_name!("fill"), "rgb(255,0,0)");
    let group = add(
        &mut m,
        svg,
        "g",
        &[("class", "outer"), ("color", "rgb(1,2,3)")],
    );
    let size = [("width", "2"), ("height", "2")];
    let inherited = add(&mut m, group, "rect", &size);
    let classed = add(
        &mut m,
        group,
        "rect",
        &[("class", "blue"), ("fill", "red"), size[0], size[1]],
    );
    let current = add(
        &mut m,
        group,
        "rect",
        &[("fill", "currentColor"), size[0], size[1]],
    );
    drop(m);
    for (target, fill) in [
        (inherited, (255, 0, 0)),
        (classed, (0, 0, 255)),
        (current, (1, 2, 3)),
    ] {
        let element = capture(&mut doc, target).expect("the rect paints");
        let paths = paths(&element);
        assert_eq!(paths.len(), 1);
        assert_eq!(fill_color(&paths[0]), Some(fill));
        assert!(
            paths[0].stroke().is_some(),
            "a descendant selector styles it"
        );
    }
}

/// Blink paints the subtree without its paint property nodes, but culls it to
/// the visual rectangle with its transforms.
#[test]
fn transforms_and_effects_are_left_out_of_paint_but_not_bounds() {
    let (mut doc, svg) = page(".half{opacity:0.5}");
    let mut m = doc.mutate();
    let group = add(&mut m, svg, "g", &[("transform", "translate(1,1)")]);
    let attrs = [
        ("class", "half"),
        ("transform", "translate(2,3)"),
        ("width", "2"),
        ("height", "2"),
    ];
    add(&mut m, group, "rect", &attrs);
    drop(m);
    let element = capture(&mut doc, group).unwrap();
    let paths = paths(&element);
    assert_eq!(paths.len(), 1);
    assert_eq!(paths[0].abs_transform(), usvg::Transform::identity());
    assert!(!painted(&mut doc, group).contains("0.5"));
    // The group's own transform and the rect's both place the bounds.
    let bounds = element.bounds;
    assert_eq!((bounds.x(), bounds.y(), bounds.width()), (3.0, 4.0, 2.0));
}

/// A rule node may hold blocks of another document's lock; reading one
/// through the wrong guard yields nothing rather than panicking.
#[test]
fn declarations_of_another_lock_are_skipped() {
    let ours = style::shared_lock::SharedRwLock::new();
    let theirs = style::shared_lock::SharedRwLock::new();
    let block = theirs.wrap(style::properties::PropertyDeclarationBlock::new());
    assert!(block.try_read_with(&ours.read()).is_none());
    assert!(block.try_read_with(&theirs.read()).is_some());
}

/// SVG attributes take unitless lengths: `font-size="20"` reaches the
/// computed style that replaces the attribute.
#[test]
fn unitless_presentation_lengths_apply() {
    let (mut doc, svg) = page("");
    let mut m = doc.mutate();
    let text = add(
        &mut m,
        svg,
        "text",
        &[("font-size", "20"), ("stroke-width", "3")],
    );
    drop(m);
    let markup = painted(&mut doc, text);
    assert!(markup.contains("font-size=\"20px\""), "{markup}");
    assert!(markup.contains("stroke-width=\"3px\""), "{markup}");
}

/// Stop colors, text anchors and markers reach computed style from
/// presentation attributes, `style` attributes and stylesheets alike.
#[test]
fn stops_anchors_and_markers_come_from_style() {
    let (mut doc, svg) = page(".anchored{text-anchor:middle}");
    let mut m = doc.mutate();
    let defs = add(&mut m, svg, "defs", &[]);
    let gradient = add(&mut m, defs, "linearGradient", &[("id", "g")]);
    add(
        &mut m,
        gradient,
        "stop",
        &[("style", "stop-color:rgb(0,255,0);stop-opacity:0.5")],
    );
    let marker = add(&mut m, defs, "marker", &[("id", "m")]);
    add(&mut m, marker, "rect", &[("width", "1"), ("height", "1")]);
    let group = add(&mut m, svg, "g", &[("marker-end", "url(#m)")]);
    let line = [("x2", "4"), ("stroke", "url(#g)")];
    let path = add(&mut m, group, "line", &line);
    let text = add(&mut m, svg, "text", &[("class", "anchored")]);
    drop(m);
    let markup = painted(&mut doc, path);
    assert!(
        markup.contains("stop-color=\"rgba(0,255,0,1)\" stop-opacity=\"0.5\""),
        "{markup}"
    );
    // The marker comes from the group outside the subtree, through style.
    assert!(markup.contains("marker-end=\"url(#c"), "{markup}");
    let element = capture(&mut doc, path).unwrap();
    assert!(element.tree.root().has_children());
    assert!(painted(&mut doc, text).contains("text-anchor=\"middle\""));
}

/// Patterns paint their content through `PaintResourceSubtree` too: Chrome
/// draws a pattern's translated child untranslated (`patternTransform` in
/// `element-images.js`).
#[test]
fn pattern_content_leaves_its_transforms_out() {
    let (mut doc, svg) = page("");
    let mut m = doc.mutate();
    let pattern = [
        ("id", "p"),
        ("width", "4"),
        ("height", "4"),
        ("patternUnits", "userSpaceOnUse"),
    ];
    let pattern = add(&mut m, svg, "pattern", &pattern);
    add(
        &mut m,
        pattern,
        "rect",
        &[
            ("transform", "translate(1,1)"),
            ("width", "1"),
            ("height", "1"),
        ],
    );
    let rect = add(
        &mut m,
        svg,
        "rect",
        &[("fill", "url(#p)"), ("width", "4"), ("height", "4")],
    );
    drop(m);
    let markup = painted(&mut doc, rect);
    assert!(!markup.contains("translate"), "{markup}");
}

/// Styles are written as presentation attributes, parsed one value each: a
/// family name with escapes and semicolons cannot add a `fill` naming a copy.
#[test]
fn style_values_cannot_inject_declarations() {
    let css = r#".x{font-family:"a\\", "x;fill:url(#c1);y"}"#;
    let (mut doc, svg) = page(css);
    let mut m = doc.mutate();
    let gradient = add(&mut m, svg, "linearGradient", &[("id", "g")]);
    add(&mut m, gradient, "stop", &[("stop-color", "lime")]);
    let group = add(&mut m, svg, "g", &[]);
    add(
        &mut m,
        group,
        "rect",
        &[("fill", "url(#g)"), ("width", "1"), ("height", "1")],
    );
    let attrs = [
        ("class", "x"),
        ("fill", "rgb(255,0,0)"),
        ("width", "1"),
        ("height", "1"),
    ];
    add(&mut m, group, "rect", &attrs);
    drop(m);
    let markup = painted(&mut doc, group);
    assert!(!markup.contains(" style="), "{markup}");
    let element = capture(&mut doc, group).unwrap();
    assert_eq!(fill_color(&paths(&element)[1]), Some((255, 0, 0)));
}

/// Blink clips markers as their own `overflow` says, while painting no other
/// overflow clip.
#[test]
fn marker_overflow_is_kept() {
    let (mut doc, svg) = page("");
    let mut m = doc.mutate();
    add(
        &mut m,
        svg,
        "marker",
        &[("id", "m"), ("overflow", "visible")],
    );
    let line = add(
        &mut m,
        svg,
        "line",
        &[("x2", "4"), ("stroke", "red"), ("marker-end", "url(#m)")],
    );
    drop(m);
    let markup = painted(&mut doc, line);
    assert!(markup.contains("<marker id=\"c1\""), "{markup}");
    assert!(markup.contains("overflow=\"visible\""), "{markup}");
}

/// Characters XML forbids are replaced, so a page's text cannot make the
/// parser refuse the whole reference.
#[test]
fn characters_xml_forbids_are_replaced() {
    let (mut doc, svg) = page("");
    let mut m = doc.mutate();
    let group = add(&mut m, svg, "g", &[("data-x", "a\u{1}b")]);
    add(&mut m, group, "rect", &[("width", "1"), ("height", "1")]);
    let text = add(&mut m, group, "text", &[]);
    let content = m.create_text_node("x\u{2}y");
    m.append_children(text, &[content]);
    drop(m);
    assert!(capture(&mut doc, group).is_some());
}

/// Names the HTML parser accepts but XML does not stay out of the markup,
/// which would otherwise not parse at all.
#[test]
fn names_outside_xml_stay_out() {
    let (mut doc, svg) = page("");
    let mut m = doc.mutate();
    let group = add(&mut m, svg, "g", &[("data\u{B5}", "1"), ("\u{B2}x", "1")]);
    add(&mut m, group, "rect", &[("width", "1"), ("height", "1")]);
    drop(m);
    assert!(capture(&mut doc, group).is_some());
}
