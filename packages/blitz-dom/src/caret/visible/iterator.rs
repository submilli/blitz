//! Blink's `SlowPositionIterator` (Chrome does not enable the fast one):
//! every position in flat-tree order, stepping through Text nodes by
//! grapheme cluster. Form controls hold their own selection, so their
//! children are not entered.

use super::context::Context;
use super::position::Position;
use crate::NodeId;

#[derive(Clone)]
pub(super) struct PositionIterator {
    anchor: Option<NodeId>,
    /// The child after the position, when the position is between children.
    child: Option<NodeId>,
    offset_in_anchor: usize,
    depth: usize,
    /// Each ancestor level's child offset, `None` until it is needed.
    offsets: Vec<Option<usize>>,
}

impl PositionIterator {
    pub(super) fn new(cx: &Context, position: Position) -> Self {
        let tree = &cx.tree;
        let anchor = position.node;
        let offset = position.editing_offset(tree);
        let child = tree.child_at(anchor, offset);
        let mut offsets = Vec::new();
        let mut node = selectable_parent(cx, anchor);
        while let Some(current) = node {
            offsets.push(None);
            node = selectable_parent(cx, current);
        }
        let depth = offsets.len();
        if child.is_some() {
            offsets.push(Some(offset));
        }
        Self {
            anchor: Some(anchor),
            child,
            offset_in_anchor: if child.is_some() { 0 } else { offset },
            depth,
            offsets,
        }
    }

    pub(super) fn node(&self) -> Option<NodeId> {
        self.anchor
    }

    pub(super) fn offset_in_text_node(&self, cx: &Context) -> Option<usize> {
        let anchor = self.anchor?;
        (cx.tree.is_text(anchor) && self.child.is_none()).then_some(self.offset_in_anchor)
    }

    fn level(&mut self, depth: usize) -> &mut Option<usize> {
        if self.offsets.len() <= depth {
            self.offsets.resize(depth + 1, None);
        }
        &mut self.offsets[depth]
    }

    /// `DeprecatedComputePosition`.
    pub(super) fn deprecated_position(&self, cx: &Context) -> Option<Position> {
        let tree = &cx.tree;
        let anchor = self.anchor?;
        if let Some(child) = self.child {
            let parent = tree.parent(child)?;
            if tree.editing_ignores_content(parent) {
                return Some(Position::before(anchor));
            }
            let offset = self.offsets.get(self.depth).copied().flatten()?;
            return Some(Position::offset(anchor, offset));
        }
        if tree.has_children(anchor) {
            return Some(Position::last_in_or_after_node(tree, anchor));
        }
        Some(Position::editing_position_of(
            tree,
            anchor,
            self.offset_in_anchor,
        ))
    }

    /// `ComputePosition`.
    pub(super) fn position(&self, cx: &Context) -> Option<Position> {
        let tree = &cx.tree;
        let anchor = self.anchor?;
        if self.child.is_some() {
            let offset = self.offsets.get(self.depth).copied().flatten()?;
            return Some(Position::offset(anchor, offset));
        }
        if traverses_children(cx, anchor) {
            return Some(Position::last_in_or_after_node(tree, anchor));
        }
        if tree.is_text(anchor) {
            return Some(Position::offset(anchor, self.offset_in_anchor));
        }
        Some(if self.offset_in_anchor > 0 {
            Position::after(anchor)
        } else {
            Position::before(anchor)
        })
    }

    pub(super) fn increment(&mut self, cx: &Context) {
        let tree = &cx.tree;
        let Some(anchor) = self.anchor else {
            return;
        };
        if !cx.spend() {
            // Out of budget: the walk ends here.
            self.anchor = None;
            return;
        }
        if let Some(child) = self.child {
            // Into the child, before its first child.
            self.anchor = Some(child);
            self.child = if traverses_children(cx, child) {
                tree.first_child(child)
            } else {
                None
            };
            self.offset_in_anchor = 0;
            self.depth += 1;
            *self.level(self.depth) = Some(0);
            return;
        }
        if cx.has_layout(anchor)
            && !traverses_children(cx, anchor)
            && self.offset_in_anchor < last_offset(cx, anchor)
        {
            // Within a leaf.
            self.offset_in_anchor = cx.next_grapheme_boundary(anchor, self.offset_in_anchor);
            return;
        }
        // Out of the anchor, to the point after it in its parent.
        self.child = Some(anchor);
        self.anchor = selectable_parent(cx, anchor);
        if self.anchor.is_none() {
            return;
        }
        self.depth = self.depth.saturating_sub(1);
        let index = tree.index(anchor);
        let level = self.level(self.depth);
        let offset = match *level {
            None => index + 1,
            Some(offset) => offset + 1,
        };
        *level = Some(offset);
        self.child = tree.next_sibling(anchor);
        self.offset_in_anchor = offset;
    }

    pub(super) fn decrement(&mut self, cx: &Context) {
        let tree = &cx.tree;
        let Some(anchor) = self.anchor else {
            return;
        };
        if !cx.spend() {
            self.anchor = None;
            return;
        }
        if let Some(child) = self.child {
            if let Some(previous) = tree.previous_sibling(child) {
                // Into the end of the previous sibling.
                self.anchor = Some(previous);
                self.child = None;
                self.offset_in_anchor = if traverses_children(cx, previous) {
                    0
                } else {
                    last_offset(cx, previous)
                };
                let index = tree.index(child);
                let level = self.level(self.depth);
                *level = Some(match *level {
                    None => index,
                    Some(offset) => offset.saturating_sub(1),
                });
                self.depth += 1;
                let offset = self.offset_in_anchor;
                *self.level(self.depth) = Some(offset);
                return;
            }
            // Before the first child: to the point before the anchor.
            self.child = tree.parent(child);
            self.anchor = self.child.and_then(|c| selectable_parent(cx, c));
            if self.anchor.is_none() {
                return;
            }
            self.offset_in_anchor = 0;
            self.depth = self.depth.saturating_sub(1);
            if let Some(child) = self.child {
                let index = tree.index(child);
                let level = self.level(self.depth);
                if level.is_none() {
                    *level = Some(index);
                }
            }
            return;
        }
        if traverses_children(cx, anchor) {
            // The end of a container: into its last child.
            let Some(last) = tree.last_child(anchor) else {
                return;
            };
            self.anchor = Some(last);
            self.offset_in_anchor = if traverses_children(cx, last) {
                0
            } else {
                last_offset(cx, last)
            };
            *self.level(self.depth) = None;
            self.depth += 1;
            return;
        }
        if self.offset_in_anchor > 0 && cx.has_layout(anchor) {
            self.offset_in_anchor = cx.previous_grapheme_boundary(anchor, self.offset_in_anchor);
            return;
        }
        // The start of a leaf: to the point before it.
        self.child = Some(anchor);
        self.anchor = selectable_parent(cx, anchor);
        if self.anchor.is_none() {
            return;
        }
        self.depth = self.depth.saturating_sub(1);
        if self.level(self.depth).is_some() {
            return;
        }
        let index = tree.index(anchor);
        self.offset_in_anchor = index;
        *self.level(self.depth) = Some(index);
    }

    pub(super) fn at_start(&self, cx: &Context) -> bool {
        let tree = &cx.tree;
        let Some(anchor) = self.anchor else {
            return true;
        };
        if tree.parent(anchor).is_some() {
            return false;
        }
        (!tree.has_children(anchor) && self.offset_in_anchor == 0)
            || self
                .child
                .is_some_and(|child| tree.previous_sibling(child).is_none())
    }

    pub(super) fn at_end(&self, cx: &Context) -> bool {
        let tree = &cx.tree;
        let Some(anchor) = self.anchor else {
            return true;
        };
        if self.child.is_some() {
            return false;
        }
        tree.parent(anchor).is_none()
            && (tree.has_children(anchor)
                || self.offset_in_anchor >= tree.last_offset_for_editing(anchor))
    }

    pub(super) fn at_start_of_node(&self, cx: &Context) -> bool {
        let tree = &cx.tree;
        let Some(anchor) = self.anchor else {
            return true;
        };
        match self.child {
            None => !traverses_children(cx, anchor) && self.offset_in_anchor == 0,
            Some(child) => tree.previous_sibling(child).is_none(),
        }
    }

    pub(super) fn at_end_of_node(&self, cx: &Context) -> bool {
        let tree = &cx.tree;
        let Some(anchor) = self.anchor else {
            return true;
        };
        if self.child.is_some() {
            return false;
        }
        tree.has_children(anchor) || self.offset_in_anchor >= tree.last_offset_for_editing(anchor)
    }
}

fn traverses_children(cx: &Context, node: NodeId) -> bool {
    cx.tree.has_children(node) && !cx.tree.is_user_select_contain(node)
}

/// `LastOffsetForPositionIterator`.
fn last_offset(cx: &Context, node: NodeId) -> usize {
    if cx.tree.is_user_select_contain(node) {
        1
    } else {
        cx.tree.last_offset_for_editing(node)
    }
}

/// The parent, unless it holds its own selection.
fn selectable_parent(cx: &Context, node: NodeId) -> Option<NodeId> {
    cx.tree
        .parent(node)
        .filter(|&parent| !cx.tree.is_user_select_contain(parent))
}
