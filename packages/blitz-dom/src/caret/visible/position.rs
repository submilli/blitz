//! DOM positions as Blink's editing represents them (`PositionTemplate` in
//! the flat tree): an anchor node with an offset inside it, or the point
//! before, after, or after the children of it.

use super::tree::Tree;
use crate::NodeId;
use crate::caret::AnchorType;
use crate::ranges::Boundary;
use std::cmp::Ordering;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Anchor {
    /// An offset in the anchor: UTF-16 units of a Text node, children of
    /// any other node.
    Offset(usize),
    Before,
    After,
    AfterChildren,
}

/// A position. Positions are equal when their anchor and kind are; two
/// kinds can describe the same boundary point (see [`Position::equivalent`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct Position {
    pub(super) node: NodeId,
    pub(super) anchor: Anchor,
}

impl Position {
    pub(super) fn offset(node: NodeId, offset: usize) -> Self {
        Self {
            node,
            anchor: Anchor::Offset(offset),
        }
    }

    pub(super) fn before(node: NodeId) -> Self {
        Self {
            node,
            anchor: Anchor::Before,
        }
    }

    pub(super) fn after(node: NodeId) -> Self {
        Self {
            node,
            anchor: Anchor::After,
        }
    }

    pub(super) fn is_before_anchor(self) -> bool {
        self.anchor == Anchor::Before
    }

    pub(super) fn is_after_anchor(self) -> bool {
        self.anchor == Anchor::After
    }

    pub(super) fn is_offset_in_anchor(self) -> bool {
        matches!(self.anchor, Anchor::Offset(_))
    }

    /// A DOM boundary point as a flat-tree position (`ToPositionInFlatTree`):
    /// an offset among a host's light children becomes the slotted child's
    /// place, or the host's end when the child is not slotted.
    pub(super) fn from_boundary(tree: &Tree, boundary: Boundary) -> Self {
        let Boundary { node, offset } = boundary;
        if tree.is_character_data(node) {
            return Self::offset(node, offset);
        }
        let node_in_flat_tree = if tree.is_dom() {
            node
        } else {
            tree.shadow_host(node).unwrap_or(node)
        };
        if offset == 0 {
            return Self::first_in_node(node_in_flat_tree);
        }
        let Some(&child) = tree.dom_children(node).get(offset) else {
            return Self::last_in_node(tree, node_in_flat_tree);
        };
        match tree.parent(child) {
            Some(parent) => Self::offset(parent, tree.index(child)),
            None => Self::last_in_node(tree, node_in_flat_tree),
        }
    }

    /// A boundary point as the position a previous step reported with
    /// `kept`; a type that no longer describes the boundary point falls
    /// back to [`Position::from_boundary`].
    pub(super) fn from_kept(tree: &Tree, boundary: Boundary, kept: AnchorType) -> Self {
        let children = tree.dom_children(boundary.node);
        let beside = |index: Option<usize>, child: NodeId| {
            index.and_then(|index| children.get(index)) == Some(&child)
        };
        let kept_position = match kept {
            AnchorType::Offset => None,
            AnchorType::Before(child) => {
                beside(Some(boundary.offset), child).then(|| Self::before(child))
            }
            AnchorType::After(child) => {
                beside(boundary.offset.checked_sub(1), child).then(|| Self::after(child))
            }
            AnchorType::AfterChildren => Some(Self {
                node: boundary.node,
                anchor: Anchor::AfterChildren,
            }),
        };
        kept_position
            .filter(|position| position.to_boundary(tree) == Some(boundary))
            .unwrap_or_else(|| Self::from_boundary(tree, boundary))
    }

    /// The anchor type that rebuilds this position from `boundary`, the
    /// boundary point reported for it.
    pub(super) fn anchor_type(self, tree: &Tree, boundary: Boundary) -> AnchorType {
        let kept = match self.anchor {
            Anchor::Offset(_) => return AnchorType::Offset,
            Anchor::Before => AnchorType::Before(self.node),
            Anchor::After => AnchorType::After(self.node),
            Anchor::AfterChildren => AnchorType::AfterChildren,
        };
        if Self::from_kept(tree, boundary, kept) == self {
            kept
        } else {
            AnchorType::Offset
        }
    }

    /// `EditingPositionOf`: an offset in a node, or before or after a node
    /// whose content editing ignores.
    pub(super) fn editing_position_of(tree: &Tree, node: NodeId, offset: usize) -> Self {
        if tree.is_text(node) || !tree.editing_ignores_content(node) {
            return Self::offset(node, offset);
        }
        if offset == 0 {
            Self::before(node)
        } else {
            Self::after(node)
        }
    }

    pub(super) fn first_in_node(node: NodeId) -> Self {
        Self::offset(node, 0)
    }

    pub(super) fn last_in_node(tree: &Tree, node: NodeId) -> Self {
        if tree.is_text(node) {
            return Self::offset(node, tree.text_length(node));
        }
        Self {
            node,
            anchor: Anchor::AfterChildren,
        }
    }

    pub(super) fn first_in_or_before_node(tree: &Tree, node: NodeId) -> Self {
        if tree.editing_ignores_content(node) {
            Self::before(node)
        } else {
            Self::first_in_node(node)
        }
    }

    pub(super) fn last_in_or_after_node(tree: &Tree, node: NodeId) -> Self {
        if tree.editing_ignores_content(node) && tree.parent(node).is_some() {
            Self::after(node)
        } else {
            Self::last_in_node(tree, node)
        }
    }

    pub(super) fn container(self, tree: &Tree) -> Option<NodeId> {
        match self.anchor {
            Anchor::Offset(_) | Anchor::AfterChildren => Some(self.node),
            Anchor::Before | Anchor::After => tree.parent(self.node),
        }
    }

    pub(super) fn offset_in_container(self, tree: &Tree) -> usize {
        match self.anchor {
            Anchor::AfterChildren => last_offset_in_node(tree, self.node),
            Anchor::Offset(offset) => offset.min(last_offset_in_node(tree, self.node)),
            Anchor::Before => tree.index(self.node),
            Anchor::After => tree.index(self.node) + 1,
        }
    }

    /// `ComputeEditingOffset`.
    pub(super) fn editing_offset(self, tree: &Tree) -> usize {
        match self.anchor {
            Anchor::After | Anchor::AfterChildren => tree.last_offset_for_editing(self.node),
            Anchor::Offset(offset) => offset,
            Anchor::Before => 0,
        }
    }

    pub(super) fn to_offset_in_anchor(self, tree: &Tree) -> Option<Self> {
        Some(Self::offset(
            self.container(tree)?,
            self.offset_in_container(tree),
        ))
    }

    /// `IsEquivalent`: the same boundary point.
    pub(super) fn equivalent(self, other: Self, tree: &Tree) -> bool {
        if self.anchor_kind() == other.anchor_kind() {
            return self == other;
        }
        self.to_offset_in_anchor(tree) == other.to_offset_in_anchor(tree)
    }

    fn anchor_kind(self) -> u8 {
        match self.anchor {
            Anchor::Offset(_) => 0,
            Anchor::Before => 1,
            Anchor::After => 2,
            Anchor::AfterChildren => 3,
        }
    }

    pub(super) fn node_before(self, tree: &Tree) -> Option<NodeId> {
        match self.anchor {
            Anchor::AfterChildren => tree.last_child(self.node),
            Anchor::Offset(offset) => tree.child_at(self.node, offset.checked_sub(1)?),
            Anchor::Before => tree.previous_sibling(self.node),
            Anchor::After => Some(self.node),
        }
    }

    pub(super) fn at_first_editing_position_for_node(self, tree: &Tree) -> bool {
        match self.anchor {
            Anchor::Offset(offset) => offset == 0,
            Anchor::Before => true,
            Anchor::After | Anchor::AfterChildren => tree.last_offset_for_editing(self.node) == 0,
        }
    }

    pub(super) fn at_last_editing_position_for_node(self, tree: &Tree) -> bool {
        match self.anchor {
            Anchor::After | Anchor::AfterChildren => true,
            Anchor::Offset(offset) => offset >= tree.last_offset_for_editing(self.node),
            Anchor::Before => tree.last_offset_for_editing(self.node) == 0,
        }
    }

    /// The DOM boundary point a selection reports for a position
    /// (`ParentAnchoredEquivalent`, then `ToPositionInDomTree`). Like Blink,
    /// the point before a node whose content editing does not ignore is
    /// reported as offset 0 inside it.
    pub(super) fn to_boundary(self, tree: &Tree) -> Option<Boundary> {
        let opaque = |node| {
            tree.editing_ignores_content(node)
                || tree.is_table(node)
                || tree.is_select_row_element(node)
        };
        let offset = match self.anchor {
            Anchor::Offset(offset) => offset,
            _ => 0,
        };
        let after = matches!(self.anchor, Anchor::After | Anchor::AfterChildren);
        if offset == 0 && !after {
            if let Some(parent) = tree.parent(self.node).filter(|_| opaque(self.node)) {
                return dom_boundary(tree, parent, tree.index(self.node));
            }
            return Some(Boundary {
                node: self.node,
                offset: 0,
            });
        }
        let at_end = after || offset == tree.count_children(self.node);
        if !tree.is_text(self.node) && at_end && opaque(self.node) {
            if let Some(parent) = tree.parent(self.node) {
                return dom_boundary(tree, parent, tree.index(self.node) + 1);
            }
        }
        dom_boundary(tree, self.container(tree)?, self.offset_in_container(tree))
    }

    /// Tree order of the boundary points, `None` for disconnected nodes.
    pub(super) fn compare(self, other: Self, tree: &Tree) -> Option<Ordering> {
        let a = (self.container(tree)?, self.offset_in_container(tree));
        let b = (other.container(tree)?, other.offset_in_container(tree));
        compare_points(tree, a, b)
    }
}

fn last_offset_in_node(tree: &Tree, node: NodeId) -> usize {
    if tree.is_text(node) {
        tree.text_length(node)
    } else {
        tree.count_children(node)
    }
}

/// Order of two boundary points (container, offset) in the tree walked.
fn compare_points(
    tree: &Tree,
    (a, a_offset): (NodeId, usize),
    (b, b_offset): (NodeId, usize),
) -> Option<Ordering> {
    if a == b {
        return Some(a_offset.cmp(&b_offset));
    }
    let a_path = ancestors(tree, a);
    let b_path = ancestors(tree, b);
    if a_path.last() != b_path.last() {
        return None;
    }
    // Walk down from the root while the chains agree.
    let common = a_path
        .iter()
        .rev()
        .zip(b_path.iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let child_under = |path: &[NodeId]| path.len().checked_sub(common + 1).map(|i| path[i]);
    match (child_under(&a_path), child_under(&b_path)) {
        // `b` is inside `a`: compare `a_offset` with the child holding `b`.
        (None, Some(b_child)) => Some(if a_offset <= tree.index(b_child) {
            Ordering::Less
        } else {
            Ordering::Greater
        }),
        (Some(a_child), None) => Some(if tree.index(a_child) < b_offset {
            Ordering::Less
        } else {
            Ordering::Greater
        }),
        (Some(a_child), Some(b_child)) => Some(tree.index(a_child).cmp(&tree.index(b_child))),
        (None, None) => Some(a_offset.cmp(&b_offset)),
    }
}

/// A node and its ancestors in the tree walked, innermost first.
fn ancestors(tree: &Tree, node: NodeId) -> Vec<NodeId> {
    let mut path = vec![node];
    let mut current = node;
    while let Some(parent) = tree.parent(current) {
        path.push(parent);
        current = parent;
    }
    path
}

/// A flat-tree offset in `node` as a DOM boundary point: the place of the
/// flat child at the offset in its DOM parent, or the end of `node`'s DOM
/// children.
fn dom_boundary(tree: &Tree, node: NodeId, offset: usize) -> Option<Boundary> {
    if tree.is_character_data(node) {
        return Some(Boundary { node, offset });
    }
    if let Some(child) = tree.child_at(node, offset) {
        let (node, offset) = tree.dom_index(child)?;
        return Some(Boundary { node, offset });
    }
    Some(Boundary {
        node,
        offset: tree.dom_children(node).len(),
    })
}
