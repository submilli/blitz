//! What inline layout construction pushes to the text layout.

mod common;

use blitz_dom::BaseDocument;
use common::q;

fn laid_out(html: &str) -> BaseDocument {
    common::laid_out(&format!(
        "<!doctype html><html><head></head><body>{html}</body></html>"
    ))
}

fn layout_text(doc: &BaseDocument, selector: &str) -> String {
    let node = doc.get_node(q(doc, selector)).unwrap();
    node.element_data()
        .unwrap()
        .inline_layout_data
        .as_ref()
        .unwrap()
        .text
        .clone()
}

#[test]
fn pseudo_elements_of_nested_inline_boxes_are_generated() {
    let doc = laid_out(
        "<style>.x::after{content:'a'}.y::before{content:'q'}</style>\
         <p><span>zz<span class=x></span></span><b><i class=y>c</i></b>bb</p>",
    );
    assert_eq!(layout_text(&doc, "p"), "zzaqcbb");
}

#[test]
fn a_select_is_an_atomic_inline_box() {
    let doc = laid_out(
        "<p style='font-size:20px'>aa<select><option>o</option><optgroup><option>ppp</option></optgroup></select>bb</p>",
    );
    let p = doc.get_node(q(&doc, "p")).unwrap();
    let layout = p
        .element_data()
        .unwrap()
        .inline_layout_data
        .as_ref()
        .unwrap();
    let select = q(&doc, "select");
    let boxes: Vec<_> = layout.layout.inline_boxes().iter().map(|b| b.id).collect();
    assert_eq!(boxes, [select.as_u64()]);
    // Its own 13.333333px font, as wide as its longest label (the grouped
    // one, indented) and its button part, one line tall: Chrome 154 gives
    // 60x19 (`select_layout.rs` measures more).
    let size = doc.get_node(select).unwrap().final_layout().size;
    assert_eq!((size.width, size.height), (60.0, 19.0));
}

/// Set `#t`'s style to each of `styles` in turn, laying the page out after
/// each.
fn restyle(doc: &mut BaseDocument, styles: &[&str]) {
    let target = q(doc, "#t");
    for style in styles {
        let attr = blitz_dom::QualName::new(None, blitz_dom::ns!(), "style".into());
        doc.mutate().set_attribute(target, attr, *style);
        doc.resolve(0.0);
    }
}

#[test]
fn a_box_flattened_back_into_a_line_keeps_no_children_of_its_own() {
    // Each box had layout children, and the span anonymous blocks, as a flex
    // or grid container; flattened into its inline root it keeps none, so
    // every layout child's parent is its container again (layout checks
    // that in debug builds) and the anonymous blocks are freed.
    for (html, styles) in [
        (
            "<div><p>aa <span id=t>bb <b style='display:inline-block'>x <i>q</i></b></span> cc</p></div>",
            &["display:flex", ""][..],
        ),
        (
            "<p id=t>aa <span>bb</span> cc</p>",
            &["display:grid", "display:contents"][..],
        ),
    ] {
        let mut doc = laid_out(html);
        let nodes_before = doc.tree().len();
        restyle(&mut doc, styles);
        let target = doc.get_node(q(&doc, "#t")).unwrap();
        assert!(target.layout_children.borrow().is_none(), "{html}");
        assert!(target.anonymous_blocks.is_empty(), "{html}");
        assert!(doc.tree().len() <= nodes_before, "{html}");
    }
}

#[test]
fn a_contents_elements_pseudo_elements_join_its_line() {
    // Chrome 154 lays out "XX" and "YY" in the span's place: "cc" starts
    // 69.39px in, after "a", "XX", "bb" and "YY".
    let doc = laid_out(
        "<style>.c::before{content:'XX'}.c::after{content:'YY'}</style>\
         <div style='font-family:Arial;font-size:16px'><p style='margin:0'>a\
         <span class=c style='display:contents'>bb</span><i>cc</i></p></div>",
    );
    let div = doc.get_client_bounding_rect(q(&doc, "div")).unwrap();
    let i = doc.get_client_bounding_rect(q(&doc, "i")).unwrap();
    // Liberation Sans advances differ from Arial's by hundredths.
    assert!((i.x - div.x - 69.39).abs() < 0.1, "{}", i.x - div.x);
}

#[test]
fn a_contents_elements_pseudo_elements_leave_no_freed_parents() {
    // As a block holding a block, the span wraps its inline `::before` in an
    // anonymous block. Once the span is `display: contents` in a line, that
    // block is freed, and the pseudo-element's layout parent is the line's
    // root instead.
    let mut doc = laid_out(
        "<style>#t::before{content:'XX'}</style>\
         <div id=root>a<span id=t style='display:block'><b id=b style='display:block'>b</b></span>c</div>",
    );
    let (span, b) = (q(&doc, "#t"), q(&doc, "#b"));
    let style = blitz_dom::QualName::new(None, blitz_dom::ns!(), "style".into());
    let mut mutation = doc.mutate();
    mutation.set_attribute(span, style.clone(), "display:contents");
    mutation.set_attribute(b, style, "");
    drop(mutation);
    doc.resolve(0.0);
    let root = q(&doc, "#root");
    assert!(doc.get_node(root).unwrap().flags.is_inline_root());
    let before = doc
        .get_node(span)
        .unwrap()
        .before()
        .expect("the span has a ::before");
    assert_eq!(
        doc.get_node(before).unwrap().layout_parent.get(),
        Some(root)
    );
}

#[test]
fn block_pseudo_elements_in_a_line_stay_on_it_unlike_chrome() {
    // Chrome 154 gives a block-level `::before` its own line, so "cc" starts
    // a line below "a" (18px down) beside a paragraph's own block pseudo,
    // and four lines down (72px, after "a", "XX", "bb" and "YY") beside a
    // contents element's. Ours lays the pseudo out on the line (submilli
    // SUB-1684).
    for (html, chrome_y, ours) in [
        (
            "<style>.c::before{content:'XX';display:block}</style>\
             <div style='font-family:Arial;font-size:16px'><p class=c style='margin:0'>a<i>cc</i></p></div>",
            18.0,
            (30.90625, 4.0),
        ),
        (
            "<style>.c::before{content:'XX';display:block}.c::after{content:'YY';display:block}</style>\
             <div style='font-family:Arial;font-size:16px'><p style='margin:0'>a\
             <span class=c style='display:contents'>bb</span><i>cc</i></p></div>",
            72.0,
            (70.703125, 4.0),
        ),
    ] {
        let doc = laid_out(html);
        let div = doc.get_client_bounding_rect(q(&doc, "div")).unwrap();
        let i = doc.get_client_bounding_rect(q(&doc, "i")).unwrap();
        let at = (i.x - div.x, i.y - div.y);
        assert_ne!(at.1, chrome_y);
        assert_eq!(at, ours);
    }
}
