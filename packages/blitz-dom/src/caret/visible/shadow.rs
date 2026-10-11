//! Keeping a range selection inside one tree scope
//! (`AdjustSelectionToAvoidCrossingShadowBoundaries` in Blink's
//! `selection_adjuster.cc`, DOM-tree version): an end in another scope
//! moves beside its ancestor in the other end's scope.

use super::tree::Tree;
use crate::NodeId;
use crate::ranges::Boundary;

/// The selection's boundary points with the focus's end kept in the
/// anchor's scope; `anchor_first` says which end is the start.
pub(super) fn avoid_crossing(
    tree: &Tree,
    anchor: Boundary,
    focus: Boundary,
    anchor_first: bool,
) -> (Boundary, Boundary) {
    if tree.scope_root(anchor.node) == tree.scope_root(focus.node) {
        return (anchor, focus);
    }
    let adjusted = beside_ancestor_in_scope(tree, focus.node, anchor.node, anchor_first);
    // Without a place in the anchor's scope, the selection collapses to it.
    (anchor, adjusted.unwrap_or(anchor))
}

/// `AdjustPositionForEnd` (`for_end`) or `AdjustPositionForStart`: beside
/// the ancestor of `node` in the scope of `other`. At a range's end, after
/// the ancestor if it holds `other` and before it otherwise; at the start,
/// the reverse.
fn beside_ancestor_in_scope(
    tree: &Tree,
    node: NodeId,
    other: NodeId,
    for_end: bool,
) -> Option<Boundary> {
    let root = tree.scope_root(other);
    let Some(ancestor) = ancestor_in_scope(tree, node, root) else {
        let children = tree.dom_children(root);
        let offset = if for_end { children.len() } else { 0 };
        return Some(Boundary { node: root, offset });
    };
    let (parent, index) = tree.dom_index(ancestor)?;
    let holds_other = tree.is_dom_inclusive_descendant(other, ancestor);
    let after = holds_other == for_end;
    Some(Boundary {
        node: parent,
        offset: if after { index + 1 } else { index },
    })
}

/// `TreeScope::AncestorInThisScope`: `node` or the shadow-including
/// ancestor of it that is in the scope rooted at `root`.
fn ancestor_in_scope(tree: &Tree, node: NodeId, root: NodeId) -> Option<NodeId> {
    let mut current = node;
    loop {
        let own_root = tree.scope_root(current);
        if own_root == root {
            return Some(current);
        }
        current = tree.shadow_host(own_root)?;
    }
}
