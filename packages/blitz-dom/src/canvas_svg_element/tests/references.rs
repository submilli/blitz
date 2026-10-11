//! Copies of `<use>` targets and paint servers, and the IDs they resolve.

use super::*;

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

/// A `<use>` may instance a sprite below `display: none`, which has no
/// computed styles: it enters with its attributes, as Chrome renders it.
#[test]
fn sprites_below_display_none_are_instanced() {
    let (mut doc, svg) = page("");
    let mut m = doc.mutate();
    let sprites = add(&mut m, svg, "svg", &[("style", "display:none")]);
    let symbol = add(&mut m, sprites, "symbol", &[("id", "icon")]);
    add(
        &mut m,
        symbol,
        "rect",
        &[("width", "2"), ("height", "2"), ("fill", "rgb(0,0,255)")],
    );
    let group = add(&mut m, svg, "g", &[]);
    add(
        &mut m,
        group,
        "use",
        &[("href", "#icon"), ("width", "2"), ("height", "2")],
    );
    drop(m);
    let element = capture(&mut doc, group).unwrap();
    let paths = paths(&element);
    assert_eq!(paths.len(), 1);
    assert_eq!(fill_color(&paths[0]), Some((0, 0, 255)));
}

/// Only local `url()`s name copies: another document's `#id` draws the
/// fallback, as in Chrome.
#[test]
fn external_paint_servers_are_not_local() {
    let (mut doc, svg) = page("");
    let mut m = doc.mutate();
    add(&mut m, svg, "linearGradient", &[("id", "g")]);
    let fill = [
        ("fill", "url(https://other.test/a.svg#g) rgb(255,0,0)"),
        ("width", "2"),
        ("height", "2"),
    ];
    let rect = add(&mut m, svg, "rect", &fill);
    drop(m);
    let element = capture(&mut doc, rect).unwrap();
    assert_eq!(fill_color(&paths(&element)[0]), Some((255, 0, 0)));
}

/// IDs are looked up as needed, so a reference early in a document larger
/// than one lookup's budget still resolves.
#[test]
fn references_resolve_in_large_documents() {
    let (mut doc, svg) = page("");
    let mut m = doc.mutate();
    let group = add(&mut m, svg, "g", &[]);
    add(&mut m, group, "use", &[("href", "#early")]);
    add(
        &mut m,
        svg,
        "rect",
        &[("id", "early"), ("width", "1"), ("height", "1")],
    );
    let filler = add(&mut m, svg, "g", &[]);
    for _ in 0..6000 {
        add(&mut m, filler, "g", &[]);
    }
    drop(m);
    let element = capture(&mut doc, group).unwrap();
    assert_eq!(paths(&element).len(), 1);
}

/// IDs resolve through the document's index, wherever they are and whether
/// or not they exist.
#[test]
fn late_and_missing_references_resolve_in_large_documents() {
    let (mut doc, svg) = page("");
    let mut m = doc.mutate();
    let group = add(&mut m, svg, "g", &[]);
    add(&mut m, group, "use", &[("href", "#late")]);
    let fallback = [
        ("fill", "url(#missing) rgb(255,0,0)"),
        ("width", "1"),
        ("height", "1"),
    ];
    add(&mut m, group, "rect", &fallback);
    let filler = add(&mut m, svg, "g", &[]);
    for _ in 0..6000 {
        add(&mut m, filler, "g", &[]);
    }
    add(
        &mut m,
        svg,
        "rect",
        &[("id", "late"), ("width", "1"), ("height", "1")],
    );
    drop(m);
    let element = capture(&mut doc, group).unwrap();
    let paths = paths(&element);
    assert_eq!(paths.len(), 2);
    assert_eq!(fill_color(&paths[1]), Some((255, 0, 0)));
}

/// Embedded images keep their `data:` URLs, however long: the markup bound
/// limits them.
#[test]
fn long_image_urls_are_kept() {
    let (mut doc, svg) = page("");
    let mut m = doc.mutate();
    let group = add(&mut m, svg, "g", &[]);
    let href = format!("data:image/png;base64,{}", "A".repeat(10_000));
    add(
        &mut m,
        group,
        "image",
        &[("href", &href), ("width", "1"), ("height", "1")],
    );
    drop(m);
    assert!(painted(&mut doc, group).contains(&href));
}

/// An unstyled sprite's paint reference falls back as a paint does.
#[test]
fn unstyled_references_fall_back() {
    let (mut doc, svg) = page("");
    let mut m = doc.mutate();
    let sprites = add(&mut m, svg, "svg", &[("style", "display:none")]);
    let symbol = add(&mut m, sprites, "symbol", &[("id", "icon")]);
    let fill = [
        ("fill", "url(#missing) rgb(255,0,0)"),
        ("width", "2"),
        ("height", "2"),
    ];
    add(&mut m, symbol, "rect", &fill);
    let group = add(&mut m, svg, "g", &[]);
    add(
        &mut m,
        group,
        "use",
        &[("href", "#icon"), ("width", "2"), ("height", "2")],
    );
    drop(m);
    let element = capture(&mut doc, group).unwrap();
    assert_eq!(fill_color(&paths(&element)[0]), Some((255, 0, 0)));
}

/// IDs inside a shadow tree resolve there, which the document's ID index
/// leaves out.
#[test]
fn shadow_tree_references_resolve() {
    let (mut doc, svg) = page("");
    let mut m = doc.mutate();
    let html = m.doc.nodes[svg].parent.unwrap();
    let host = m.create_element(qual_name!("div", html), vec![]);
    m.append_children(html, &[host]);
    let root = m
        .attach_shadow(host, crate::shadow::ShadowRootInit::default())
        .unwrap();
    let inner = m.create_element(qual_name!("svg", svg), vec![]);
    m.append_children(root, &[inner]);
    let gradient = add(&mut m, inner, "linearGradient", &[("id", "g")]);
    add(&mut m, gradient, "stop", &[("stop-color", "lime")]);
    add(
        &mut m,
        gradient,
        "stop",
        &[("offset", "1"), ("stop-color", "blue")],
    );
    add(
        &mut m,
        inner,
        "rect",
        &[("id", "r"), ("width", "1"), ("height", "1")],
    );
    let group = add(&mut m, inner, "g", &[]);
    add(&mut m, group, "use", &[("href", "#r")]);
    add(
        &mut m,
        group,
        "rect",
        &[("fill", "url(#g)"), ("width", "1"), ("height", "1")],
    );
    drop(m);
    let element = capture(&mut doc, group).unwrap();
    let paths = paths(&element);
    assert_eq!(paths.len(), 2);
    let paint = paths[1].fill().unwrap().paint().clone();
    assert!(matches!(paint, usvg::Paint::LinearGradient(_)));
}

/// Duplicate IDs resolve to the first in tree order, as Chrome's do.
#[test]
fn duplicate_ids_take_the_first_in_tree_order() {
    let (mut doc, svg) = page("");
    let mut m = doc.mutate();
    let group = add(&mut m, svg, "g", &[]);
    add(&mut m, group, "use", &[("href", "#dup")]);
    let first = add(&mut m, svg, "g", &[]);
    let size = [("width", "1"), ("height", "1")];
    // The later rect gets its ID first, so tree order decides, not insertion.
    let later = [("id", "dup"), ("fill", "rgb(255,0,0)"), size[0], size[1]];
    add(&mut m, svg, "rect", &later);
    let earlier = [("id", "dup"), ("fill", "rgb(0,0,255)"), size[0], size[1]];
    add(&mut m, first, "rect", &earlier);
    drop(m);
    let element = capture(&mut doc, group).unwrap();
    assert_eq!(fill_color(&paths(&element)[0]), Some((0, 0, 255)));
}

/// A reference the plan cannot read never reaches the parser, which would
/// resolve it itself: like a missing one, it paints nothing.
#[test]
fn unread_references_are_left_out() {
    let (mut doc, svg) = page("");
    let mut m = doc.mutate();
    let sprites = add(&mut m, svg, "svg", &[("style", "display:none")]);
    let symbol = add(&mut m, sprites, "symbol", &[("id", "icon")]);
    let padded = format!("url(#c1){}", " ".repeat(5000));
    add(
        &mut m,
        symbol,
        "path",
        &[
            ("d", "M0 0 L1 1 L2 0"),
            ("marker-mid", &padded),
            ("fill", "url(#c1"),
        ],
    );
    let group = add(&mut m, svg, "g", &[]);
    add(&mut m, group, "use", &[("href", "#icon")]);
    drop(m);
    let markup = painted(&mut doc, group);
    let path = "<path d=\"M0 0 L1 1 L2 0\" marker-mid=\"none\" fill=\"none\">";
    assert!(markup.contains(path), "{markup}");
}

/// An unstyled paint's fallback is rewritten from the color it parses as:
/// a second reference in its place paints nothing rather than naming a copy.
#[test]
fn unstyled_fallbacks_cannot_name_copies() {
    let (mut doc, svg) = page("");
    let mut m = doc.mutate();
    let sprites = add(&mut m, svg, "svg", &[("style", "display:none")]);
    let pattern = add(&mut m, sprites, "pattern", &[("id", "p")]);
    add(&mut m, pattern, "rect", &[("width", "1"), ("height", "1")]);
    let symbol = add(&mut m, sprites, "symbol", &[("id", "icon")]);
    add(
        &mut m,
        symbol,
        "rect",
        &[("fill", "url(#p)"), ("width", "1"), ("height", "1")],
    );
    let named = [
        ("fill", "url(#nope) url(#c2)"),
        ("width", "1"),
        ("height", "1"),
    ];
    add(&mut m, symbol, "rect", &named);
    let colored = [("fill", "url(#nope) #00f"), ("width", "1"), ("height", "1")];
    add(&mut m, symbol, "rect", &colored);
    let group = add(&mut m, svg, "g", &[]);
    add(&mut m, group, "use", &[("href", "#icon")]);
    drop(m);
    let markup = painted(&mut doc, group);
    assert_eq!(markup.matches("url(#c2)").count(), 1, "{markup}");
    assert!(markup.contains("fill=\"none\""), "{markup}");
    assert!(markup.contains("fill=\"rgba(0,0,255,1)\""), "{markup}");
}
