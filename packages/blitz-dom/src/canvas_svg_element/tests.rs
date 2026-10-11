use crate::canvas_svg_image::CanvasSvgElement;
use crate::{BaseDocument, DocumentConfig, DocumentMutator, NodeId, QualName, qual_name};

/// A document with `css` in a `<style>` and an empty outermost `<svg>`.
fn page(css: &str) -> (BaseDocument, NodeId) {
    let mut doc = BaseDocument::new(DocumentConfig {
        base_url: Some("https://example.test/page".into()),
        ..Default::default()
    });
    let root = doc.root_node_id;
    let mut m = doc.mutate();
    let html = m.create_element(qual_name!("html", html), vec![]);
    m.append_children(root, &[html]);
    let style = m.create_element(qual_name!("style", html), vec![]);
    m.set_text_content(style, css);
    let svg = m.create_element(qual_name!("svg", svg), vec![]);
    m.append_children(html, &[style, svg]);
    drop(m);
    (doc, svg)
}

/// Append an SVG element to `parent`.
fn add(m: &mut DocumentMutator<'_>, parent: NodeId, tag: &str, attrs: &[(&str, &str)]) -> NodeId {
    let name = QualName {
        prefix: None,
        ns: crate::Namespace::from("http://www.w3.org/2000/svg"),
        local: tag.into(),
    };
    let element = m.create_element(name, vec![]);
    for (attr, value) in attrs {
        let name = QualName {
            prefix: None,
            ns: crate::Namespace::from(""),
            local: (*attr).into(),
        };
        m.set_attribute(element, name, *value);
    }
    m.append_children(parent, &[element]);
    element
}

fn capture(doc: &mut BaseDocument, target: NodeId) -> Option<CanvasSvgElement> {
    doc.flush_style_and_layout(0.0);
    doc.capture_svg_element(target).ok().flatten()
}

/// The painted markup of `target`'s reference.
fn painted(doc: &mut BaseDocument, target: NodeId) -> String {
    doc.flush_style_and_layout(0.0);
    let plan = super::plan::Plan::new(doc, target).unwrap();
    let viewport = doc.svg_viewport(target).unwrap();
    super::markup::write(doc, &plan, viewport).unwrap().painted
}

/// The paths of a parsed tree, in paint order.
fn paths(element: &CanvasSvgElement) -> Vec<usvg::Path> {
    fn collect(group: &usvg::Group, found: &mut Vec<usvg::Path>) {
        for node in group.children() {
            match node {
                usvg::Node::Path(path) => found.push((**path).clone()),
                usvg::Node::Group(group) => collect(group, found),
                _ => {}
            }
        }
    }
    let mut found = Vec::new();
    collect(element.tree.root(), &mut found);
    found
}

fn fill_color(path: &usvg::Path) -> Option<(u8, u8, u8)> {
    match path.fill()?.paint() {
        usvg::Paint::Color(color) => Some((color.red, color.green, color.blue)),
        _ => None,
    }
}

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

/// `<use>` targets become copies that keep only their declared styles, so
/// they inherit from the `<use>`; its offset only places the bounds.
#[test]
fn use_copies_inherit_from_the_use() {
    let (mut doc, svg) = page("");
    let mut m = doc.mutate();
    let group = add(&mut m, svg, "g", &[]);
    let attrs = [("href", "#target"), ("x", "3"), ("fill", "rgb(0,0,255)")];
    add(&mut m, group, "use", &attrs);
    add(
        &mut m,
        svg,
        "rect",
        &[("id", "target"), ("width", "2"), ("height", "2")],
    );
    drop(m);
    let element = capture(&mut doc, group).unwrap();
    let paths = paths(&element);
    assert_eq!(paths.len(), 1);
    assert_eq!(fill_color(&paths[0]), Some((0, 0, 255)));
    assert_eq!(paths[0].abs_transform(), usvg::Transform::identity());
    assert_eq!(element.bounds.x(), 3.0);
}

/// A `<use>` whose instance would contain itself is left out, as in Blink;
/// the rest of the subtree still paints.
#[test]
fn cyclic_uses_are_left_out() {
    let (mut doc, svg) = page("");
    let mut m = doc.mutate();
    let group = add(&mut m, svg, "g", &[("id", "self")]);
    add(&mut m, group, "rect", &[("width", "2"), ("height", "2")]);
    add(&mut m, group, "use", &[("href", "#self")]);
    add(&mut m, group, "use", &[("href", "#a")]);
    let a = add(&mut m, svg, "g", &[("id", "a")]);
    add(
        &mut m,
        a,
        "rect",
        &[("x", "4"), ("width", "1"), ("height", "1")],
    );
    add(&mut m, a, "use", &[("href", "#b")]);
    let b = add(&mut m, svg, "g", &[("id", "b")]);
    add(&mut m, b, "use", &[("href", "#a")]);
    drop(m);
    let markup = painted(&mut doc, group);
    // Only the `<use>` of `#a` survives: `#self` and the `#a`/`#b` loop do not.
    assert_eq!(markup.matches("<use").count(), 1, "{markup}");
    let element = capture(&mut doc, group).unwrap();
    assert_eq!(paths(&element).len(), 2);
}

/// Nested `<use>`s multiply: past the expansion bound the reference draws
/// nothing, before the parser builds anything.
#[test]
fn use_expansion_is_bounded() {
    let (mut doc, svg) = page("");
    let mut m = doc.mutate();
    add(
        &mut m,
        svg,
        "rect",
        &[("id", "l0"), ("width", "1"), ("height", "1")],
    );
    for level in 1..=8 {
        let group = add(&mut m, svg, "g", &[("id", &format!("l{level}"))]);
        for _ in 0..4 {
            add(
                &mut m,
                group,
                "use",
                &[("href", &format!("#l{}", level - 1))],
            );
        }
    }
    let small = add(&mut m, svg, "g", &[]);
    add(&mut m, small, "use", &[("href", "#l2")]);
    let huge = add(&mut m, svg, "g", &[]);
    add(&mut m, huge, "use", &[("href", "#l8")]);
    drop(m);
    doc.flush_style_and_layout(0.0);
    let small = doc.capture_svg_element(small).unwrap().unwrap();
    assert_eq!(paths(&small).len(), 16);
    // 4^8 rects.
    assert!(doc.capture_svg_element(huge).is_err());
}

#[test]
fn paint_servers_outside_the_subtree_are_copied() {
    let (mut doc, svg) = page("");
    let mut m = doc.mutate();
    let defs = add(&mut m, svg, "defs", &[]);
    let gradient = add(&mut m, defs, "linearGradient", &[("id", "g")]);
    add(
        &mut m,
        gradient,
        "stop",
        &[("offset", "0"), ("stop-color", "lime")],
    );
    add(
        &mut m,
        gradient,
        "stop",
        &[("offset", "1"), ("stop-color", "blue")],
    );
    let size = [("width", "2"), ("height", "2")];
    let rect = add(
        &mut m,
        svg,
        "rect",
        &[("fill", "url(#g)"), size[0], size[1]],
    );
    let fallback = [("fill", "url(#none) rgb(255,0,0)"), size[0], size[1]];
    let missing = add(&mut m, svg, "rect", &fallback);
    drop(m);
    let element = capture(&mut doc, rect).unwrap();
    let paint = paths(&element)[0].fill().unwrap().paint().clone();
    assert!(matches!(paint, usvg::Paint::LinearGradient(_)));
    let element = capture(&mut doc, missing).unwrap();
    assert_eq!(fill_color(&paths(&element)[0]), Some((255, 0, 0)));
}

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
        let plan = super::plan::Plan::new(&doc, target).unwrap();
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

/// The parser sees no page IDs, classes, scripts or event handlers: copies
/// carry generated IDs, and names XML cannot hold stay out.
#[test]
fn markup_carries_no_page_identifiers() {
    let (mut doc, svg) = page("");
    let mut m = doc.mutate();
    let attrs = [
        ("id", "page-id"),
        ("class", "c"),
        ("onclick", "x()"),
        ("a\"b", "1"),
    ];
    let group = add(&mut m, svg, "g", &attrs);
    add(&mut m, group, "script", &[]);
    add(&mut m, group, "use", &[("href", "#other")]);
    add(
        &mut m,
        svg,
        "rect",
        &[("id", "other"), ("width", "1"), ("height", "1")],
    );
    drop(m);
    let markup = painted(&mut doc, group);
    for absent in ["page-id", "class", "onclick", "script", "a\"b", "other"] {
        assert!(!markup.contains(absent), "{absent} in {markup}");
    }
    assert!(markup.contains("href=\"#c1\""));
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
