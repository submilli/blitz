//! Selections across tree scopes in the DOM tree: keeping a range inside
//! one scope (`AdjustSelectionToAvoidCrossingShadowBoundaries` in Blink's
//! `selection_adjuster.cc`, DOM-tree version) and ordering positions in
//! different scopes (`ComparePositions` in `compare_positions.cc`).

use super::position::Position;
use super::tree::Tree;
use crate::NodeId;
use std::cmp::Ordering;
use std::collections::HashSet;

/// The focus kept in the anchor's tree scope: beside its ancestor in that
/// scope, or the anchor itself when the scope has no place for it.
/// `anchor_first` says which end is the range's start.
pub(super) fn avoid_crossing(
    tree: &Tree,
    anchor: Position,
    focus: Position,
    anchor_first: bool,
) -> Position {
    // Blink compares the anchor nodes' scopes, not the containers'.
    if tree.scope_root(anchor.node) == tree.scope_root(focus.node) {
        return focus;
    }
    let Some(anchor_container) = anchor.container(tree) else {
        return anchor;
    };
    beside_ancestor_in_scope(tree, focus, anchor_container, anchor_first).unwrap_or(anchor)
}

/// `AdjustPositionForEnd` (`for_end`) or `AdjustPositionForStart`: beside
/// the ancestor of `position`'s container in the scope of `other`. At a
/// range's end, after the ancestor if it holds `other` and before it
/// otherwise; at the start, the reverse. Without such an ancestor, past the
/// scope's last child (at the end) or before its first.
fn beside_ancestor_in_scope(
    tree: &Tree,
    position: Position,
    other: NodeId,
    for_end: bool,
) -> Option<Position> {
    let root = tree.scope_root(other);
    let Some(ancestor) = ancestor_in_scope(tree, position.container(tree)?, root) else {
        let children = tree.dom_children(root);
        return if for_end {
            children.last().map(|&last| Position::after(last))
        } else {
            children.first().map(|&first| Position::before(first))
        };
    };
    let holds_other = tree.is_dom_inclusive_descendant(other, ancestor);
    Some(if holds_other == for_end {
        Position::after(ancestor)
    } else {
        Position::before(ancestor)
    })
}

/// Tree order of positions in different tree scopes of the DOM tree. Each
/// is taken to its ancestor in the innermost scope holding both, and a
/// position inside that ancestor (in its shadow tree) compares as just
/// after the ancestor's start. `None` when no scope holds both.
pub(super) fn order_across_scopes(tree: &Tree, a: Position, b: Position) -> Option<Ordering> {
    let (a_container, b_container) = (a.container(tree)?, b.container(tree)?);
    let root = common_scope(tree, a_container, b_container)?;
    // Each position in the common scope, with its container there and
    // whether it was inside that container's shadow tree.
    let in_scope = |position: Position, container: NodeId| {
        let ancestor = ancestor_in_scope(tree, container, root)?;
        Some(if ancestor == container {
            (position, container, false)
        } else {
            (Position::offset(ancestor, 0), ancestor, true)
        })
    };
    let (a, a_container, a_inside) = in_scope(a, a_container)?;
    let (b, b_container, b_inside) = in_scope(b, b_container)?;
    let order = a.compare(b, tree)?;
    if order != Ordering::Equal || a_container != b_container {
        return Some(order);
    }
    Some(match (a_inside, b_inside) {
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        _ => Ordering::Equal,
    })
}

/// The root of the innermost tree scope holding both nodes
/// (`CommonAncestorTreeScope`).
fn common_scope(tree: &Tree, a: NodeId, b: NodeId) -> Option<NodeId> {
    let roots_of = |node: NodeId| {
        let mut roots = vec![tree.scope_root(node)];
        while let Some(host) = roots.last().and_then(|&root| tree.shadow_host(root)) {
            roots.push(tree.scope_root(host));
        }
        roots
    };
    let b_roots: HashSet<NodeId> = roots_of(b).into_iter().collect();
    roots_of(a).into_iter().find(|root| b_roots.contains(root))
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
