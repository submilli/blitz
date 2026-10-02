//! Remove derived references before native storage can release their targets.
use std::collections::HashSet;

use crate::layout::damage::ALL_DAMAGE;
use crate::node::{FlatParent, Node, NodeFlags, SpecialElementData};
use crate::{BaseDocument, NodeId};

pub(super) fn prune(doc: &mut BaseDocument, live: &HashSet<NodeId>) {
    for &id in live {
        let node = &mut doc.nodes[id];
        let changed = prune_node(node, live);
        if changed {
            node.clear_layout_cache();
            node.insert_damage(ALL_DAMAGE);
        }
    }
    doc.nodes.bump_geometry_generation();
}

fn prune_node(node: &mut Node, live: &HashSet<NodeId>) -> bool {
    node.manual_slottables.retain(|id| live.contains(id));
    node.slot_assignment.retain(|id| live.contains(id));
    if node.manual_slot.is_some_and(|id| !live.contains(&id)) {
        node.manual_slot = None;
    }
    let mut changed = false;
    if node
        .layout_parent
        .get()
        .is_some_and(|id| !live.contains(&id))
    {
        node.layout_parent.set(None);
        changed = true;
    }
    if node
        .oof_containing_block
        .get()
        .is_some_and(|id| !live.contains(&id))
    {
        node.oof_containing_block.set(None);
        changed = true;
    }
    if let FlatParent::Node(id) = node.flat_parent.get()
        && !live.contains(&id)
    {
        node.flat_parent.set(FlatParent::Dom);
        changed = true;
    }
    if node.is_anonymous() && node.children.iter().any(|id| !live.contains(id)) {
        node.children.retain(|id| live.contains(id));
        changed = true;
    }
    for children in [
        node.layout_children.get_mut(),
        node.paint_children.get_mut(),
        &mut node.composed_children,
    ]
    .into_iter()
    .flatten()
    {
        let before = children.len();
        children.retain(|id| live.contains(id));
        changed |= before != children.len();
    }
    let hoisted = node.hoisted_children.get_mut();
    let before = hoisted.len();
    hoisted.retain(|id| live.contains(id));
    changed |= before != hoisted.len();
    let contributions = node.sc_contribution_cache.get_mut();
    let before = contributions.len();
    contributions.retain(|entry| live.contains(&entry.node_id));
    changed |= before != contributions.len();
    if let Some(context) = &mut node.stacking_context {
        let before = context.children.len();
        context
            .children
            .retain(|entry| live.contains(&entry.node_id));
        if before != context.children.len() {
            context.sort();
            changed = true;
        }
    }
    changed | prune_text_and_table(node, live)
}

fn prune_text_and_table(node: &mut Node, live: &HashSet<NodeId>) -> bool {
    let Some(element) = node.element_data_mut() else {
        return false;
    };
    let mut cleared_flags = NodeFlags::empty();
    if element.inline_layout_data.as_ref().is_some_and(|text| {
        text.layout
            .styles()
            .iter()
            .any(|style| !live.contains(&style.brush.id))
            || text
                .layout
                .inline_boxes()
                .iter()
                .any(|item| !live.contains(&NodeId::from_u64(item.id)))
    }) {
        element.inline_layout_data = None;
        cleared_flags.insert(NodeFlags::IS_INLINE_ROOT);
    }
    if let SpecialElementData::TableRoot(table) = &element.special_data
        && table.references_missing_node(live)
    {
        element.special_data = SpecialElementData::None;
        cleared_flags.insert(NodeFlags::IS_TABLE_ROOT);
    }
    node.flags.remove(cleared_flags);
    !cleared_flags.is_empty()
}
