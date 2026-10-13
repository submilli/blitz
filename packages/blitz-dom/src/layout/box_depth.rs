//! How deep boxes nest. Layout, paint and the caret engine recurse once per
//! box level, and construction once per DOM level below a box, so trees
//! nested past [`MAX_BOX_DEPTH`] get no boxes there rather than overflowing
//! the native stack.

use blitz_traits::node_id::NodeId;
use style::selector_parser::RestyleDamage;

use super::damage::{CONSTRUCT_BOX, CONSTRUCT_DESCENDENT, CONSTRUCT_FC};
use crate::BaseDocument;
use crate::node::{Node, NodeFlags};

/// The deepest level, the root box at 0, at which content gets boxes: the
/// box tree's depth, and separately how far construction follows
/// `display: contents` elements and inline content through the DOM below a
/// box. The two add up on the native stack. As if `display: none`, deeper
/// content gets no boxes.
///
/// The guest's 8 MiB held about 2,000 nested blocks and 1,200 nested
/// buttons (two boxes each), so this keeps half of it in reserve. Chrome 155
/// lays out up to about 1,500 nested blocks, but its renderer crashes on
/// 2,000, on 500 nested buttons and on 400 inline-blocks. The parser keeps
/// each tree's markup within 512 levels (`blitz-html`), but script, and
/// declarative shadow trees nested in each other, build deeper flat trees.
///
/// A subdocument is painted within its parent's paint, so its depth starts
/// at its iframe's (`BaseDocument::box_depth_base`): a frame chain shares
/// the limit.
pub(crate) const MAX_BOX_DEPTH: usize = 1024;

/// Whether content `depth` levels below a box gets no boxes.
pub(crate) fn past_box_depth(depth: usize) -> bool {
    depth > MAX_BOX_DEPTH
}

/// How many boxes contain `node_id`'s, up to its document's root; at most
/// just past [`MAX_BOX_DEPTH`].
pub(crate) fn box_depth(nodes: &crate::NodeTree, node_id: NodeId) -> usize {
    let mut depth = 0;
    let mut current = nodes.get(node_id).and_then(|node| node.layout_parent.get());
    while let Some(parent) = current {
        depth += 1;
        if past_box_depth(depth) {
            break;
        }
        current = nodes.get(parent).and_then(|node| node.layout_parent.get());
    }
    depth
}

/// Construct `node_id`, a box at [`MAX_BOX_DEPTH`], with no boxes below it.
/// `rebuild`: an ancestor was rebuilt for the depth limit. Returns whether
/// its boxes changed, which damage does not tell the layouts cached above.
pub(crate) fn construct_at_box_depth(
    doc: &mut BaseDocument,
    node_id: NodeId,
    mut damage: RestyleDamage,
    rebuild: bool,
) -> bool {
    // A capped box is flagged with an empty child list; a node omitted
    // itself is flagged with no child list at all.
    let node = &doc.nodes[node_id];
    let capped = node.flags.contains(NodeFlags::HAS_OMITTED_BOXES)
        && node
            .layout_children
            .borrow()
            .as_ref()
            .is_some_and(|children| children.is_empty());
    let changed = !capped || rebuild || damage.intersects(CONSTRUCT_FC | CONSTRUCT_BOX);
    if changed {
        omit_boxes_below(doc, node_id);
    }
    damage.remove(CONSTRUCT_DESCENDENT | CONSTRUCT_FC | CONSTRUCT_BOX);
    doc.nodes[node_id].set_damage(damage);
    changed
}

/// Give `node_id`'s descendants no boxes.
fn omit_boxes_below(doc: &mut BaseDocument, node_id: NodeId) {
    for anonymous_block in std::mem::take(&mut doc.nodes[node_id].anonymous_blocks) {
        doc.deallocate_anonymous_block(anonymous_block);
    }
    let node = &mut doc.nodes[node_id];
    node.flags.reset_construction_flags();
    node.flags.insert(NodeFlags::HAS_OMITTED_BOXES);
    node.invalidate_layout_cache();
    if let Some(element_data) = node.element_data_mut() {
        element_data.take_inline_layout();
    }
    let mut descendants: Vec<NodeId> = node.flat_children().to_vec();
    descendants.extend(node.before());
    descendants.extend(node.after());
    *node.layout_children.borrow_mut() = Some(Default::default());
    for descendant in descendants {
        omit_subtree_boxes(&mut doc.nodes, descendant);
    }
}

/// Give `root` and its descendants no boxes. Each is flagged, so that
/// constructing its box again, within the limit, rebuilds its subtree too.
pub(super) fn omit_subtree_boxes(nodes: &mut crate::NodeTree, root: NodeId) {
    for_each_in_subtree(nodes, root, |node| {
        clear_layout(node);
        node.flags.insert(NodeFlags::HAS_OMITTED_BOXES);
        *node.layout_children.borrow_mut() = None;
    });
}

/// A reconstruction may have freed these nodes' former anonymous parents.
/// Omitted boxes must expose empty geometry and retain no positioning links.
pub(super) fn clear_omitted_layout(nodes: &mut crate::NodeTree, root: NodeId) {
    for_each_in_subtree(nodes, root, clear_layout);
}

fn clear_layout(node: &mut Node) {
    node.layout_parent.set(None);
    node.oof_containing_block.set(None);
    node.flags.remove(NodeFlags::IS_INLINE_ROOT);
    node.hoisted_children.get_mut().clear();
    node.sc_contribution_cache.get_mut().clear();
    node.stacking_context = None;
    if let Some(layout) = node.try_layout_data_mut() {
        *layout = Default::default();
    }
}

/// Visit `root` and its flat-tree descendants and pseudo-elements, with an
/// explicit stack: they can nest as deep as the node limit.
fn for_each_in_subtree(
    nodes: &mut crate::NodeTree,
    root: NodeId,
    mut visit: impl FnMut(&mut Node),
) {
    let mut pending = vec![root];
    while let Some(id) = pending.pop() {
        let node = &mut nodes[id];
        pending.extend(node.flat_children().iter().copied());
        pending.extend(node.before());
        pending.extend(node.after());
        visit(node);
    }
}
