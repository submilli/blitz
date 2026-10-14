//! Generated content must produce a box tree that layout can round.
mod common;

use blitz_dom::{BaseDocument, NodeId};
use common::q;

const REDUCED: &str = "<style>#t::before{content:'G';display:contents}</style>\
                       <div id=t style='display:inline-table;font-size:40px'></div>";

/// Text belongs to a text layout, never among the box children that Taffy
/// rounds. Walk the actual box tree rather than every allocated DOM node.
fn assert_box_tree(doc: &BaseDocument) {
    let mut pending = vec![q(doc, "html")];
    let mut visited = std::collections::HashSet::<NodeId>::new();
    while let Some(id) = pending.pop() {
        assert!(visited.insert(id), "box {id:?} reached twice");
        let node = doc.get_node(id).unwrap();
        assert!(node.element_data().is_some(), "{}", node.node_debug_str());
        assert!(node.final_layout().size.width.is_finite());
        assert!(node.final_layout().size.height.is_finite());
        if let Some(children) = node.layout_children.borrow().as_ref() {
            pending.extend(children);
        }
    }
}

#[test]
fn generated_content_fuzz_page_lays_out() {
    let mut doc = common::laid_out(include_str!("fixtures/generated_content.html"));
    assert_box_tree(&doc);
    doc.resolve(0.0);
    assert_box_tree(&doc);
}

#[test]
fn a_contents_pseudo_of_a_table_has_no_box() {
    let doc = common::laid_out(REDUCED);
    assert_box_tree(&doc);
    let table = doc.get_node(q(&doc, "#t")).unwrap();
    let before = table.before().unwrap();
    let children = table.layout_children.borrow();
    let children = children.as_ref().unwrap();
    assert!(!children.contains(&before));
    assert_eq!(children.len(), 1);
    let wrapper = doc.get_node(children[0]).unwrap();
    assert!(wrapper.flags.is_inline_root());
    assert!(wrapper.final_layout().size.width > 0.0);
    assert!(wrapper.final_layout().size.height > 0.0);
    assert_eq!(
        wrapper
            .element_data()
            .unwrap()
            .inline_layout_data
            .as_ref()
            .unwrap()
            .text,
        "G"
    );
}

#[test]
fn contents_pseudos_keep_box_children_across_display_changes() {
    for pseudo in ["before", "after"] {
        for table_display in ["table", "inline-table"] {
            let html = format!(
                "<style>#t::{pseudo}{{content:'G';display:contents}}</style>\
                 <div id=t style='display:{table_display}'><div style='display:table-row'>\
                 <div id=cell style='display:table-cell;width:40px;height:20px'>cell</div>\
                 </div></div>"
            );
            let mut doc = common::laid_out(&html);
            let target = q(&doc, "#t");
            let attr = blitz_dom::QualName::new(None, blitz_dom::ns!(), "style".into());
            for display in [
                "block",
                "flex",
                "grid",
                table_display,
                "none",
                table_display,
            ] {
                doc.mutate()
                    .set_attribute(target, attr.clone(), format!("display:{display}"));
                doc.resolve(0.0);
                assert_box_tree(&doc);
                if display == table_display {
                    let cell = doc.get_node(q(&doc, "#cell")).unwrap();
                    assert_eq!(
                        (
                            cell.final_layout().size.width,
                            cell.final_layout().size.height
                        ),
                        (40.0, 20.0)
                    );
                }
            }
        }
    }
}

#[test]
fn large_table_spans_are_admitted_before_grid_allocation() {
    let cells = (0..12)
        .map(|i| format!("<div id=c{i} colspan=65535 style='display:table-cell'>x</div>"))
        .collect::<String>();
    let columns = "<div span=65535 style='display:table-column;width:1px'></div>".repeat(12);
    let mut doc = common::laid_out(&format!(
        "<div id=t style='display:table'><div style='display:table-column-group'>{columns}</div>\
         <div style='display:contents'><div id=row style='display:table-row'>{cells}</div></div></div>"
    ));
    assert_box_tree(&doc);
    let omitted = q(&doc, "#c11");
    assert_eq!(
        doc.get_node(omitted).unwrap().final_layout().size.width,
        0.0
    );
    let row = q(&doc, "#row");
    for i in 0..11 {
        let id = q(&doc, &format!("#c{i}"));
        doc.mutate().remove_node(id);
    }
    let attr = blitz_dom::QualName::new(None, blitz_dom::ns!(), "colspan".into());
    doc.mutate().set_attribute(omitted, attr, "0");
    doc.resolve(0.0);
    assert_box_tree(&doc);
    assert!(doc.get_node(omitted).unwrap().final_layout().size.width > 0.0);
    assert!(
        doc.get_node(row)
            .unwrap()
            .layout_children
            .borrow()
            .is_none()
    );
}

#[test]
fn oversized_decimal_spans_are_clamped_after_parsing() {
    let doc = common::laid_out(
        "<div style='display:table'><div style='display:table-column-group'>\
         <div span=65536 style='display:table-column;width:1px'></div></div>\
         <div style='display:table-row'><div id=first colspan=65536 rowspan=65536 \
         style='display:table-cell'>x</div><div id=next style='display:table-cell'>y</div>\
         </div></div>",
    );
    assert_box_tree(&doc);
    assert!(
        doc.get_node(q(&doc, "#first"))
            .unwrap()
            .final_layout()
            .size
            .width
            > 0.0
    );
    assert!(
        doc.get_node(q(&doc, "#next"))
            .unwrap()
            .final_layout()
            .size
            .width
            > 0.0
    );
}

#[test]
fn contents_wrapped_columns_keep_their_fixed_widths() {
    let doc = common::laid_out(
        "<div style='display:table;table-layout:fixed;width:200px;border-spacing:0'>\
         <div style='display:contents'><div style='display:contents'>\
         <div style='display:table-column-group'>\
         <div style='display:table-column;width:150px'></div>\
         <div style='display:table-column'></div></div></div></div>\
         <div style='display:table-row'><div id=wide style='display:table-cell'>a</div>\
         <div id=narrow style='display:table-cell'>b</div></div></div>",
    );
    assert_box_tree(&doc);
    let wide = doc
        .get_node(q(&doc, "#wide"))
        .unwrap()
        .final_layout()
        .size
        .width;
    let narrow = doc
        .get_node(q(&doc, "#narrow"))
        .unwrap()
        .final_layout()
        .size
        .width;
    assert_eq!((wide, narrow), (150.0, 50.0));
}

#[test]
fn generated_rows_keep_pseudo_style_and_table_group_order() {
    let doc = common::laid_out(
        "<style>#t::before,#t::after{content:'G';display:contents;font-size:80px}</style>\
         <div id=t style='display:table;font-size:10px'>\
         <div style='display:contents'><div style='display:table-header-group'><div style='display:table-row'>\
         <div id=header style='display:table-cell;height:10px'>h</div></div></div></div>\
         <div style='display:contents'><div id=direct style='display:table-cell;height:20px'>d</div></div>\
         <div style='display:table-row'><div id=body style='display:table-cell;height:10px'>b</div></div>\
         <div style='display:contents'><div style='display:table-footer-group'><div style='display:table-row'>\
         <div id=footer style='display:table-cell;height:10px'>f</div></div></div></div></div>",
    );
    assert_box_tree(&doc);
    let table = doc.get_node(q(&doc, "#t")).unwrap();
    let generated = table
        .layout_children
        .borrow()
        .as_ref()
        .unwrap()
        .iter()
        .copied()
        .filter(|id| doc.get_node(*id).unwrap().is_anonymous())
        .collect::<Vec<_>>();
    assert_eq!(generated.len(), 2);
    let before = doc.get_node(generated[0]).unwrap().final_layout();
    let after = doc.get_node(generated[1]).unwrap().final_layout();
    assert!(before.size.height >= 80.0);
    assert!(after.size.height >= 80.0);
    let y = |selector| {
        doc.get_node(q(&doc, selector))
            .unwrap()
            .final_layout()
            .location
            .y
    };
    assert!(y("#header") < before.location.y);
    assert!(before.location.y < y("#direct"));
    assert!(y("#direct") < y("#body"));
    assert!(y("#body") < after.location.y);
    assert!(after.location.y < y("#footer"));
}
