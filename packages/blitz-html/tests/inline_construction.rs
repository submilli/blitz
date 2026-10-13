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
