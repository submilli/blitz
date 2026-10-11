//! Bounds on what one reference expands to before parsing.

use super::*;

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

/// Every copy is built by the parser wherever it stands, `<defs>` included:
/// a chain of copies that each instance a large tree counts that tree once
/// per copy, though the chain is within the nesting bound.
#[test]
fn expansion_counts_every_copy() {
    let (mut doc, svg) = page("");
    let mut m = doc.mutate();
    // A hidden SVG of its own, which layout never parses: usvg recurses once
    // per `<use>` in a chain (SUB-1570).
    let html = m.doc.nodes[svg].parent.unwrap();
    let sprites = add(&mut m, html, "svg", &[("style", "display:none")]);
    add(
        &mut m,
        sprites,
        "rect",
        &[("id", "d0"), ("width", "1"), ("height", "1")],
    );
    // d7 expands to 2^7 rects.
    for level in 1..=7 {
        let group = add(&mut m, sprites, "g", &[("id", &format!("d{level}"))]);
        for _ in 0..2 {
            add(
                &mut m,
                group,
                "use",
                &[("href", &format!("#d{}", level - 1))],
            );
        }
    }
    // A chain of 30 groups, each using the next, the last using d7.
    for link in 0..30 {
        let group = add(&mut m, sprites, "g", &[("id", &format!("f{link}"))]);
        let next = if link == 29 {
            "#d7".to_owned()
        } else {
            format!("#f{}", link + 1)
        };
        add(&mut m, group, "use", &[("href", &next)]);
    }
    let chained = add(&mut m, svg, "g", &[]);
    add(&mut m, chained, "use", &[("href", "#f0")]);
    let direct = add(&mut m, svg, "g", &[]);
    add(&mut m, direct, "use", &[("href", "#d7")]);
    drop(m);
    doc.flush_style_and_layout(0.0);
    assert!(doc.capture_svg_element(direct).unwrap().is_some());
    assert!(doc.capture_svg_element(chained).is_err());
}

/// Instances copy their content too: large path data weighs more than an
/// element.
#[test]
fn expansion_weighs_content() {
    let (mut doc, svg) = page("");
    let mut m = doc.mutate();
    let data = format!("M0 0{}", " L1 1".repeat(20_000));
    add(&mut m, svg, "path", &[("id", "p"), ("d", &data)]);
    let group = add(&mut m, svg, "g", &[]);
    for _ in 0..100 {
        add(&mut m, group, "use", &[("href", "#p")]);
    }
    drop(m);
    doc.flush_style_and_layout(0.0);
    assert!(doc.capture_svg_element(group).is_err());
}

/// A chain of light `<use>`s stays within the weight but not the depth the
/// parser recurses to.
#[test]
fn use_nesting_is_bounded() {
    let (mut doc, svg) = page("");
    let mut m = doc.mutate();
    let html = m.doc.nodes[svg].parent.unwrap();
    let sprites = add(&mut m, html, "svg", &[("style", "display:none")]);
    add(&mut m, sprites, "g", &[("id", "l0")]);
    for link in 1..=65 {
        let group = add(&mut m, sprites, "g", &[("id", &format!("l{link}"))]);
        add(
            &mut m,
            group,
            "use",
            &[("href", &format!("#l{}", link - 1))],
        );
    }
    let shallow = add(&mut m, svg, "g", &[]);
    add(&mut m, shallow, "use", &[("href", "#l40")]);
    let deep = add(&mut m, svg, "g", &[]);
    add(&mut m, deep, "use", &[("href", "#l65")]);
    drop(m);
    doc.flush_style_and_layout(0.0);
    assert!(doc.capture_svg_element(shallow).is_ok());
    assert!(doc.capture_svg_element(deep).is_err());
}

/// Markers are built once per vertex and patterns once per element they
/// paint: chains of them multiply, and are refused before parsing.
#[test]
fn nested_markers_and_patterns_are_bounded() {
    let (mut doc, svg) = page("");
    let mut m = doc.mutate();
    let html = m.doc.nodes[svg].parent.unwrap();
    let sprites = add(&mut m, html, "svg", &[("style", "display:none")]);
    let vertices = format!("M0 0{}", " L1 1".repeat(200));
    for level in 0..3 {
        let marker = add(&mut m, sprites, "marker", &[("id", &format!("m{level}"))]);
        let path = [
            ("d", vertices.as_str()),
            ("marker-mid", &format!("url(#m{})", level + 1)),
        ];
        add(&mut m, marker, "path", &path);
    }
    add(&mut m, sprites, "marker", &[("id", "m3")]);
    for level in 0..20 {
        let pattern = add(&mut m, sprites, "pattern", &[("id", &format!("p{level}"))]);
        for _ in 0..2 {
            let fill = [
                ("fill", &*format!("url(#p{})", level + 1)),
                ("width", "1"),
                ("height", "1"),
            ];
            add(&mut m, pattern, "rect", &fill);
        }
    }
    add(&mut m, sprites, "pattern", &[("id", "p20")]);
    let marked = add(
        &mut m,
        svg,
        "path",
        &[("d", &vertices), ("marker-mid", "url(#m0)")],
    );
    let patterned = add(
        &mut m,
        svg,
        "rect",
        &[("fill", "url(#p0)"), ("width", "1"), ("height", "1")],
    );
    drop(m);
    doc.flush_style_and_layout(0.0);
    assert!(doc.capture_svg_element(marked).is_err());
    assert!(doc.capture_svg_element(patterned).is_err());
}

/// The bytes each copy writes, styles included, count once per build: a
/// stylesheet can give a light element long values.
#[test]
fn expanded_markup_is_bounded() {
    let family = "f".repeat(4000);
    let dashes = vec!["1"; 800].join(",");
    let css = format!(".d{{font-family:{family};stroke-dasharray:{dashes}}}");
    let (mut doc, svg) = page(&css);
    let mut m = doc.mutate();
    let html = m.doc.nodes[svg].parent.unwrap();
    let sprites = add(&mut m, html, "svg", &[("style", "display:none")]);
    let dashed = add(&mut m, svg, "g", &[("id", "t")]);
    add(
        &mut m,
        dashed,
        "line",
        &[("class", "d"), ("x2", "1"), ("stroke", "red")],
    );
    // 5^4 builds of the line weigh about 2200, within the weight bound, but
    // write about 5 MB of styles.
    for level in 0..4 {
        let group = add(&mut m, sprites, "g", &[("id", &format!("f{level}"))]);
        let target = if level == 0 {
            "#t".to_owned()
        } else {
            format!("#f{}", level - 1)
        };
        for _ in 0..5 {
            add(&mut m, group, "use", &[("href", &target)]);
        }
    }
    let fanned = add(&mut m, svg, "g", &[]);
    add(&mut m, fanned, "use", &[("href", "#f3")]);
    drop(m);
    doc.flush_style_and_layout(0.0);
    let markup = painted(&mut doc, dashed);
    assert!(markup.len() > 8000, "{}", markup.len());
    assert!(doc.capture_svg_element(fanned).is_err());
}

/// Elements nested deeper than the parser's recursion allows are refused.
#[test]
fn element_nesting_is_bounded() {
    let (mut doc, svg) = page("");
    let mut m = doc.mutate();
    // A hidden SVG of its own, which layout never parses (SUB-1570).
    let html = m.doc.nodes[svg].parent.unwrap();
    let sprites = add(&mut m, html, "svg", &[("style", "display:none")]);
    let root = add(&mut m, sprites, "g", &[]);
    let mut parent = root;
    for _ in 0..130 {
        parent = add(&mut m, parent, "g", &[]);
    }
    add(&mut m, parent, "rect", &[("width", "1"), ("height", "1")]);
    drop(m);
    doc.flush_style_and_layout(0.0);
    assert!(doc.capture_svg_element(root).is_err());
}

/// Blink draws no markers on rectangles, circles and ellipses; the parser
/// would, once per vertex, so they are written without markers and a chain
/// of such markers costs nothing.
#[test]
fn shapes_without_markers_are_written_without_them() {
    let (mut doc, svg) = page("");
    let mut m = doc.mutate();
    let html = m.doc.nodes[svg].parent.unwrap();
    let sprites = add(&mut m, html, "svg", &[("style", "display:none")]);
    for level in 0..3 {
        let marker = add(&mut m, sprites, "marker", &[("id", &format!("m{level}"))]);
        let next = [("marker-mid", &*format!("url(#m{})", level + 1))];
        let group = add(&mut m, marker, "g", &next);
        for _ in 0..300 {
            add(&mut m, group, "rect", &[("width", "1"), ("height", "1")]);
        }
    }
    add(&mut m, sprites, "marker", &[("id", "m3")]);
    let group = add(&mut m, svg, "g", &[("marker-mid", "url(#m0)")]);
    add(&mut m, group, "rect", &[("width", "1"), ("height", "1")]);
    drop(m);
    let markup = painted(&mut doc, group);
    let rects = markup.matches("<rect").count();
    let unmarked = markup
        .matches("marker-start=\"none\" marker-mid=\"none\" marker-end=\"none\"></rect>")
        .count();
    assert_eq!(rects, unmarked, "{markup}");
    // Each marker is built once where defined: the chain costs nothing.
    assert!(capture(&mut doc, group).is_some());
}

/// Markers and patterns on a `<use>` are inherited by its content: they are
/// built for each of its marked vertices and painted elements.
#[test]
fn markers_and_patterns_inherited_through_uses_are_counted() {
    let (mut doc, svg) = page("");
    let mut m = doc.mutate();
    let html = m.doc.nodes[svg].parent.unwrap();
    let sprites = add(&mut m, html, "svg", &[("style", "display:none")]);
    let marker = add(&mut m, sprites, "marker", &[("id", "m")]);
    for _ in 0..100 {
        add(&mut m, marker, "rect", &[("width", "1"), ("height", "1")]);
    }
    let pattern = add(&mut m, sprites, "pattern", &[("id", "p")]);
    for _ in 0..100 {
        add(&mut m, pattern, "rect", &[("width", "1"), ("height", "1")]);
    }
    let vertices = format!("M0 0{}", " L1 1".repeat(200));
    add(&mut m, sprites, "path", &[("id", "long"), ("d", &vertices)]);
    let painters = add(&mut m, sprites, "g", &[("id", "many")]);
    for _ in 0..200 {
        add(&mut m, painters, "rect", &[("width", "1"), ("height", "1")]);
    }
    let marked = add(&mut m, svg, "g", &[]);
    add(
        &mut m,
        marked,
        "use",
        &[("href", "#long"), ("marker-mid", "url(#m)")],
    );
    let patterned = add(&mut m, svg, "g", &[]);
    add(
        &mut m,
        patterned,
        "use",
        &[("href", "#many"), ("fill", "url(#p)")],
    );
    drop(m);
    doc.flush_style_and_layout(0.0);
    assert!(doc.capture_svg_element(marked).is_err());
    assert!(doc.capture_svg_element(patterned).is_err());
}

/// Start and end markers are built once, middle ones per vertex: an
/// ordinary chart's markers fit.
#[test]
fn ordinary_markers_fit() {
    let (mut doc, svg) = page("");
    let mut m = doc.mutate();
    let marker = add(&mut m, svg, "marker", &[("id", "dot")]);
    for _ in 0..3 {
        add(&mut m, marker, "circle", &[("r", "1")]);
    }
    let points: Vec<String> = (0..250).map(|x| format!("{x},{}", x % 7)).collect();
    let attrs = [
        ("points", &*points.join(" ")),
        ("marker-start", "url(#dot)"),
        ("marker-mid", "url(#dot)"),
        ("marker-end", "url(#dot)"),
        ("stroke", "red"),
    ];
    let chart = add(&mut m, svg, "polyline", &attrs);
    drop(m);
    assert!(capture(&mut doc, chart).is_some());
}

/// Vertices are counted with the parser's grammar: numbers packed as
/// `.1.1.1.1` are separate coordinates, each pair a vertex.
#[test]
fn packed_path_numbers_count_as_vertices() {
    let (mut doc, svg) = page("");
    let mut m = doc.mutate();
    let marker = add(&mut m, svg, "marker", &[("id", "m")]);
    for _ in 0..100 {
        add(&mut m, marker, "rect", &[("width", "1"), ("height", "1")]);
    }
    let data = format!("M0 0L{}", ".1.1".repeat(2000));
    let path = add(
        &mut m,
        svg,
        "path",
        &[("d", &data), ("marker-mid", "url(#m)")],
    );
    drop(m);
    doc.flush_style_and_layout(0.0);
    assert!(doc.capture_svg_element(path).is_err());
}

/// Elements without styles inherit marker and pattern attributes from their
/// containers in the parser: each painted or marked descendant counts.
#[test]
fn references_unstyled_containers_pass_on_are_counted() {
    let (mut doc, svg) = page("");
    let mut m = doc.mutate();
    let html = m.doc.nodes[svg].parent.unwrap();
    let sprites = add(&mut m, html, "svg", &[("style", "display:none")]);
    let vertices = format!("M0 0{}", " L1 1".repeat(200));
    for level in 0..2 {
        let marker = add(&mut m, sprites, "marker", &[("id", &format!("m{level}"))]);
        let next = format!("url(#m{})", level + 1);
        let group = add(&mut m, marker, "g", &[("marker-mid", &next)]);
        add(&mut m, group, "path", &[("d", &vertices)]);
    }
    let last = add(&mut m, sprites, "marker", &[("id", "m2")]);
    for _ in 0..300 {
        add(&mut m, last, "rect", &[("width", "1"), ("height", "1")]);
    }
    let pattern = add(&mut m, sprites, "pattern", &[("id", "p")]);
    for _ in 0..100 {
        add(&mut m, pattern, "rect", &[("width", "1"), ("height", "1")]);
    }
    let painters = add(
        &mut m,
        sprites,
        "g",
        &[("id", "icons"), ("fill", "url(#p)")],
    );
    for _ in 0..200 {
        add(&mut m, painters, "rect", &[("width", "1"), ("height", "1")]);
    }
    let marked = add(
        &mut m,
        svg,
        "path",
        &[("d", &vertices), ("marker-mid", "url(#m0)")],
    );
    let patterned = add(&mut m, svg, "g", &[]);
    add(&mut m, patterned, "use", &[("href", "#icons")]);
    drop(m);
    doc.flush_style_and_layout(0.0);
    assert!(doc.capture_svg_element(marked).is_err());
    assert!(doc.capture_svg_element(patterned).is_err());
}

/// The parser splits arcs into curves by their radius: radii it could not
/// subdivide within bounds are refused, in shapes and in path data.
#[test]
fn huge_arc_radii_are_refused() {
    let (mut doc, svg) = page("");
    let mut m = doc.mutate();
    let circle = add(&mut m, svg, "circle", &[("r", "3e38")]);
    let rounded = add(
        &mut m,
        svg,
        "rect",
        &[("width", "1"), ("height", "1"), ("rx", "1e30")],
    );
    let arc = add(&mut m, svg, "path", &[("d", "M0 0A1e30 1e30 0 1 1 2e30 0")]);
    let ordinary = add(&mut m, svg, "circle", &[("r", "1e6"), ("fill", "red")]);
    drop(m);
    doc.flush_style_and_layout(0.0);
    for target in [circle, rounded, arc] {
        assert!(doc.capture_svg_element(target).is_err(), "{target}");
    }
    assert!(doc.capture_svg_element(ordinary).is_ok());
}

/// Text is shaped and flattened per character, so it weighs per character.
#[test]
fn text_weighs_per_character() {
    let (mut doc, svg) = page("");
    let mut m = doc.mutate();
    let text = add(&mut m, svg, "text", &[]);
    let content = m.create_text_node("x".repeat(20_000));
    m.append_children(text, &[content]);
    drop(m);
    doc.flush_style_and_layout(0.0);
    assert!(doc.capture_svg_element(text).is_err());
}
