//! Paints the parser builds out of sight of the elements naming them: text
//! and its decorations, text paths and context paints.

use super::*;

/// Within text a link is text however deeply it nests: its characters are
/// charged as the parser shapes them.
#[test]
fn deep_links_in_text_are_text() {
    let (mut doc, svg) = page("");
    let mut m = doc.mutate();
    let html = m.doc.nodes[svg].parent.unwrap();
    let sprites = add(&mut m, html, "svg", &[("style", "display:none")]);
    let text = add(&mut m, sprites, "text", &[]);
    let mut parent = text;
    for _ in 0..70 {
        parent = add(&mut m, parent, "a", &[]);
    }
    let content = m.create_text_node("x".repeat(20_000));
    m.append_children(parent, &[content]);
    drop(m);
    doc.flush_style_and_layout(0.0);
    assert!(doc.capture_svg_element(text).is_err());
}

/// A text path converts the shape it follows each time it is built: a large
/// path followed by many instanced text paths is refused.
#[test]
fn text_paths_build_their_shape_each_time() {
    let (mut doc, svg) = page("");
    let mut m = doc.mutate();
    let html = m.doc.nodes[svg].parent.unwrap();
    let sprites = add(&mut m, html, "svg", &[("style", "display:none")]);
    let d = format!("M0 0{}", " L1 1".repeat(2000));
    add(&mut m, sprites, "path", &[("id", "p"), ("d", d.as_str())]);
    let text = add(&mut m, sprites, "text", &[("id", "t0")]);
    let text_path = add(&mut m, text, "textPath", &[("href", "#p")]);
    let content = m.create_text_node("x");
    m.append_children(text_path, &[content]);
    for level in 1..=3 {
        let group = add(&mut m, sprites, "g", &[("id", &format!("t{level}"))]);
        for _ in 0..9 {
            add(
                &mut m,
                group,
                "use",
                &[("href", &format!("#t{}", level - 1))],
            );
        }
    }
    let few = add(&mut m, svg, "g", &[]);
    add(&mut m, few, "use", &[("href", "#t1")]);
    let many = add(&mut m, svg, "g", &[]);
    add(&mut m, many, "use", &[("href", "#t3")]);
    drop(m);
    doc.flush_style_and_layout(0.0);
    assert!(doc.capture_svg_element(few).is_ok());
    assert!(doc.capture_svg_element(many).is_err());
}

/// A `<use>` in a marker that passes `context-fill` to its content builds the
/// marked element's pattern for every element the content paints.
#[test]
fn context_paints_through_uses_are_counted() {
    let (mut doc, svg) = page("");
    let mut m = doc.mutate();
    let html = m.doc.nodes[svg].parent.unwrap();
    let sprites = add(&mut m, html, "svg", &[("style", "display:none")]);
    let pattern = add(&mut m, sprites, "pattern", &[("id", "p")]);
    for _ in 0..10 {
        add(&mut m, pattern, "rect", &[("width", "1"), ("height", "1")]);
    }
    let rects = add(&mut m, sprites, "g", &[("id", "h")]);
    for _ in 0..100 {
        add(&mut m, rects, "rect", &[("width", "1"), ("height", "1")]);
    }
    let group = add(&mut m, sprites, "g", &[("id", "g")]);
    for _ in 0..9 {
        add(&mut m, group, "use", &[("href", "#h")]);
    }
    let marker = add(&mut m, sprites, "marker", &[("id", "m")]);
    add(
        &mut m,
        marker,
        "use",
        &[("href", "#g"), ("fill", "context-fill")],
    );
    let attrs = [
        ("d", "M0 0 L1 1"),
        ("fill", "url(#p)"),
        ("marker-start", "url(#m)"),
    ];
    let path = add(&mut m, svg, "path", &attrs);
    drop(m);
    doc.flush_style_and_layout(0.0);
    assert!(doc.capture_svg_element(path).is_err());
}

/// Each decoration paints text again with its paint server: decorated text
/// filled with a pattern is charged for them.
#[test]
fn decorated_text_paints_are_counted() {
    let (mut doc, svg) = page("");
    let mut m = doc.mutate();
    let html = m.doc.nodes[svg].parent.unwrap();
    let sprites = add(&mut m, html, "svg", &[("style", "display:none")]);
    let pattern = add(&mut m, sprites, "pattern", &[("id", "p")]);
    for _ in 0..10 {
        add(&mut m, pattern, "rect", &[("width", "1"), ("height", "1")]);
    }
    let mut text = |decoration: Option<&str>| {
        let mut attrs = vec![("fill", "url(#p)")];
        attrs.extend(decoration.map(|value| ("text-decoration", value)));
        let text = add(&mut m, sprites, "text", &attrs);
        let content = m.create_text_node("x".repeat(200));
        m.append_children(text, &[content]);
        text
    };
    let plain = text(None);
    let decorated = text(Some("underline overline line-through"));
    drop(m);
    doc.flush_style_and_layout(0.0);
    assert!(doc.capture_svg_element(plain).is_ok());
    assert!(doc.capture_svg_element(decorated).is_err());
}

/// Decorations take their paint from the element declaring them: a pattern
/// on decorated text is charged for its styled spans' decorations too.
#[test]
fn decorations_charge_the_declaring_paint() {
    let (mut doc, svg) = page("");
    let mut m = doc.mutate();
    let html = m.doc.nodes[svg].parent.unwrap();
    let sprites = add(&mut m, html, "svg", &[("style", "display:none")]);
    let pattern = add(&mut m, sprites, "pattern", &[("id", "p")]);
    for _ in 0..10 {
        add(&mut m, pattern, "rect", &[("width", "1"), ("height", "1")]);
    }
    let attrs = [
        ("fill", "url(#p)"),
        ("text-decoration", "underline overline line-through"),
    ];
    let text = add(&mut m, svg, "text", &attrs);
    let span = add(&mut m, text, "tspan", &[("fill", "red")]);
    let content = m.create_text_node("x".repeat(200));
    m.append_children(span, &[content]);
    drop(m);
    doc.flush_style_and_layout(0.0);
    assert!(doc.capture_svg_element(text).is_err());
}

/// A context-painted element passes its context paint on to its own
/// markers: the outer pattern is built per build of both markers.
#[test]
fn context_paints_through_nested_markers_are_counted() {
    let (mut doc, svg) = page("");
    let mut m = doc.mutate();
    let html = m.doc.nodes[svg].parent.unwrap();
    let sprites = add(&mut m, html, "svg", &[("style", "display:none")]);
    let pattern = add(&mut m, sprites, "pattern", &[("id", "p")]);
    for _ in 0..50 {
        add(&mut m, pattern, "rect", &[("width", "1"), ("height", "1")]);
    }
    let inner = add(&mut m, sprites, "marker", &[("id", "m2")]);
    for _ in 0..40 {
        let attrs = [("fill", "context-fill"), ("width", "1"), ("height", "1")];
        add(&mut m, inner, "rect", &attrs);
    }
    let outer = add(&mut m, sprites, "marker", &[("id", "m1")]);
    for _ in 0..4 {
        let attrs = [
            ("d", "M0 0 L1 1"),
            ("fill", "context-fill"),
            ("marker-start", "url(#m2)"),
        ];
        add(&mut m, outer, "path", &attrs);
    }
    let attrs = [
        ("d", "M0 0 L1 1"),
        ("fill", "url(#p)"),
        ("marker-start", "url(#m1)"),
    ];
    let path = add(&mut m, svg, "path", &attrs);
    drop(m);
    doc.flush_style_and_layout(0.0);
    assert!(doc.capture_svg_element(path).is_err());
}

/// Context-painted text paints the marked element's pattern per character.
#[test]
fn context_painted_text_is_counted_per_character() {
    let (mut doc, svg) = page("");
    let mut m = doc.mutate();
    let html = m.doc.nodes[svg].parent.unwrap();
    let sprites = add(&mut m, html, "svg", &[("style", "display:none")]);
    let pattern = add(&mut m, sprites, "pattern", &[("id", "p")]);
    for _ in 0..20 {
        add(&mut m, pattern, "rect", &[("width", "1"), ("height", "1")]);
    }
    let marker = add(&mut m, sprites, "marker", &[("id", "m")]);
    let text = add(&mut m, marker, "text", &[("fill", "context-fill")]);
    let content = m.create_text_node("x".repeat(200));
    m.append_children(text, &[content]);
    let attrs = [
        ("d", "M0 0 L1 1"),
        ("fill", "url(#p)"),
        ("marker-start", "url(#m)"),
    ];
    let path = add(&mut m, svg, "path", &attrs);
    drop(m);
    doc.flush_style_and_layout(0.0);
    assert!(doc.capture_svg_element(path).is_err());
}

/// Decorations declared above text take the text's paint: its pattern is
/// charged for every decorated span.
#[test]
fn decorations_above_text_charge_the_text_paint() {
    let (mut doc, svg) = page("");
    let mut m = doc.mutate();
    let html = m.doc.nodes[svg].parent.unwrap();
    let sprites = add(&mut m, html, "svg", &[("style", "display:none")]);
    let pattern = add(&mut m, sprites, "pattern", &[("id", "p")]);
    for _ in 0..10 {
        add(&mut m, pattern, "rect", &[("width", "1"), ("height", "1")]);
    }
    let decorations = [("text-decoration", "underline overline line-through")];
    let group = add(&mut m, svg, "g", &decorations);
    let text = add(&mut m, group, "text", &[("fill", "url(#p)")]);
    for _ in 0..200 {
        let span = add(&mut m, text, "tspan", &[("fill", "red")]);
        let content = m.create_text_node("x");
        m.append_children(span, &[content]);
    }
    drop(m);
    doc.flush_style_and_layout(0.0);
    assert!(doc.capture_svg_element(group).is_err());
}

/// A context-painted element passes its context paint on to the markers it
/// inherits from an unstyled ancestor too.
#[test]
fn inherited_markers_of_context_paints_are_counted() {
    let (mut doc, svg) = page("");
    let mut m = doc.mutate();
    let html = m.doc.nodes[svg].parent.unwrap();
    let sprites = add(&mut m, html, "svg", &[("style", "display:none")]);
    let pattern = add(&mut m, sprites, "pattern", &[("id", "p")]);
    for _ in 0..50 {
        add(&mut m, pattern, "rect", &[("width", "1"), ("height", "1")]);
    }
    let inner = add(&mut m, sprites, "marker", &[("id", "m2")]);
    for _ in 0..40 {
        let attrs = [("fill", "context-fill"), ("width", "1"), ("height", "1")];
        add(&mut m, inner, "rect", &attrs);
    }
    let outer = add(&mut m, sprites, "marker", &[("id", "m1")]);
    let group = add(&mut m, outer, "g", &[("marker-start", "url(#m2)")]);
    for _ in 0..4 {
        let attrs = [("d", "M0 0 L1 1"), ("fill", "context-fill")];
        add(&mut m, group, "path", &attrs);
    }
    let attrs = [
        ("d", "M0 0 L1 1"),
        ("fill", "url(#p)"),
        ("marker-start", "url(#m1)"),
    ];
    let path = add(&mut m, svg, "path", &attrs);
    drop(m);
    doc.flush_style_and_layout(0.0);
    assert!(doc.capture_svg_element(path).is_err());
}
