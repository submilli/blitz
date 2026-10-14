use blitz_traits::node_id::NodeId;
use style::values::specified::box_::DisplayInside;

use crate::BaseDocument;
use crate::layout::box_depth::{
    omit_subtree_boxes, past_box_depth, release_flattened_box_from_document,
};
use crate::layout::damage::{CONSTRUCT_BOX, CONSTRUCT_DESCENDENT, CONSTRUCT_FC};

pub(super) fn flatten_table_root_items(
    doc: &mut BaseDocument,
    node_id: NodeId,
    depth: usize,
    items: &mut Vec<(NodeId, usize)>,
) {
    if past_box_depth(depth) {
        omit_subtree_boxes(&mut doc.nodes, node_id);
        return;
    }
    let display = doc.nodes[node_id]
        .primary_styles()
        .map(|style| style.clone_display());
    if display.is_some_and(|display| display.inside() == DisplayInside::Contents) {
        release_flattened_box_from_document(doc, node_id);
        let children = doc.nodes[node_id].children.to_vec();
        for child_id in children {
            doc.nodes[child_id].remove_damage(CONSTRUCT_DESCENDENT | CONSTRUCT_FC | CONSTRUCT_BOX);
            flatten_table_root_items(doc, child_id, depth + 1, items);
        }
    } else {
        items.push((node_id, depth));
    }
}

/// Header row groups precede ordinary rows; footer row groups follow them.
pub(super) fn table_root_order(doc: &BaseDocument, node_id: NodeId) -> u8 {
    let display = doc.nodes[node_id]
        .primary_styles()
        .map(|style| style.clone_display());
    match display.map(|display| display.inside()) {
        Some(DisplayInside::TableHeaderGroup) => 0,
        Some(DisplayInside::TableFooterGroup) => 2,
        _ => 1,
    }
}
