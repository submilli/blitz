//! Line boxes for vertical movement (Blink's `selection_modifier_line.cc`):
//! the line above or below a position, found in its formatting context or
//! else by walking the DOM's leaves, and the position nearest a horizontal
//! point on it.

use super::canonical;
use super::caret::{self, Caret, Placed};
use super::context::Context;
use super::hit;
use super::ifc::{Ifc, Item};
use super::position::Position;
use super::text;
use super::visible_position;
use crate::NodeId;
use std::rc::Rc;

/// One line box of a formatting context.
#[derive(Clone)]
pub(super) struct LineBox {
    pub(super) ifc: Rc<Ifc>,
    pub(super) index: usize,
}

impl LineBox {
    /// The line a position's caret draws on (`AbstractLineBox::CreateFor`).
    pub(super) fn of(cx: &Context, placed: Placed) -> Option<Self> {
        cx.object(placed.position.node)?;
        let adjusted = caret::inline_adjusted(cx, placed)?;
        Some(Self::of_caret(&caret::inline_caret(cx, adjusted)?))
    }

    pub(super) fn new(ifc: Rc<Ifc>, index: usize) -> Self {
        Self { ifc, index }
    }

    pub(super) fn of_caret(caret: &Caret) -> Self {
        Self::new(caret.ifc.clone(), caret.line())
    }

    fn previous(&self) -> Option<Self> {
        Some(Self {
            ifc: self.ifc.clone(),
            index: self.index.checked_sub(1)?,
        })
    }

    fn next(&self) -> Option<Self> {
        (self.index + 1 < self.ifc.lines.len()).then(|| Self {
            ifc: self.ifc.clone(),
            index: self.index + 1,
        })
    }

    pub(super) fn items(&self) -> &[Item] {
        self.ifc.line_items(self.index)
    }

    /// A line a caret can move to (`CanBeCaretContainer`): it has height,
    /// and a leaf from the DOM. Like Blink, the search for that leaf runs on
    /// from the line to the end of its context, so a line of generated
    /// content qualifies when a later line holds DOM content.
    fn can_hold_caret(&self) -> bool {
        let line = &self.ifc.lines[self.index];
        // A line without leaves (floats only, or white space that hangs)
        // is an empty line box.
        let empty = !self.items().iter().any(Item::is_leaf);
        let rest = self.ifc.items.get(line.items.start..).unwrap_or(&[]);
        !empty && line.bottom > line.top && rest.iter().any(|item| item.kind_node().is_some())
    }

    /// The position nearest `x` on the line (`AbstractLineBox::PositionForPoint`):
    /// in the leaf under it, or the closest one.
    fn position_for_point(&self, cx: &Context, x: f32) -> Option<Placed> {
        let base = self.ifc.lines[self.index].items.start;
        let mut closest: Option<(usize, f32)> = None;
        for (offset, item) in self.items().iter().enumerate() {
            if !item.is_leaf() {
                continue;
            }
            if x >= item.left && x < item.right {
                closest = Some((offset, 0.0));
                break;
            }
            let distance = if x < item.left {
                item.left - x
            } else {
                x - item.right + 1.0
            };
            if closest.is_none_or(|(_, best)| distance < best) {
                closest = Some((offset, distance));
            }
        }
        let index = base + closest?.0;
        let item = &self.ifc.items[index];
        let node = item.kind_node()?;
        if cx.tree.editing_ignores_content(node) {
            return Some(Placed::downstream(Position::before(node)));
        }
        let y = self.ifc.lines[self.index].top;
        hit::point_in_child(cx, &self.ifc, index, x, y, 0)
    }
}

/// The position nearest `x` on the line above (`PreviousLinePosition`).
pub(super) fn previous_line_position(cx: &Context, placed: Placed, x: f32) -> Option<Placed> {
    vertical(cx, placed, x, false)
}

/// The position nearest `x` on the line below (`NextLinePosition`).
pub(super) fn next_line_position(cx: &Context, placed: Placed, x: f32) -> Option<Placed> {
    vertical(cx, placed, x, true)
}

fn vertical(cx: &Context, placed: Placed, x: f32, down: bool) -> Option<Placed> {
    let position = placed.position;
    cx.object(position.node)?;
    let step = |line: &LineBox| if down { line.next() } else { line.previous() };
    let mut line = LineBox::of(cx, placed)
        .and_then(|line| step(&line))
        .filter(LineBox::can_hold_caret);
    // Chrome's `InlineBlockLineNavigation` (stable in 154): a step out of an
    // inline-block's first or last line goes to the line beside it in the
    // outer context.
    if line.is_none() {
        line = escape_inline_block(cx, position, down);
    }
    if line.is_none() {
        let candidate = if down {
            let child = cx
                .tree
                .child_at(position.node, position.editing_offset(&cx.tree));
            let start = child.unwrap_or_else(|| cx.tree.last_within_or_self(position.node));
            next_root_inline_box_candidate(cx, start, placed)
        } else {
            previous_root_inline_box_candidate(cx, position.node, placed)
        };
        if let Some(candidate) = candidate {
            let visible = visible_position::create(cx, Placed::downstream(candidate));
            line = visible.and_then(|v| LineBox::of(cx, v));
            if line.is_none() {
                return Some(Placed::downstream(candidate));
            }
        }
    }
    if let Some(found) = line.and_then(|line| line.position_for_point(cx, x)) {
        return Some(found);
    }
    // No line beyond: the start (or end) of the document's content.
    let root = cx.tree.document_element()?;
    Some(Placed::downstream(if down {
        Position::last_in_node(&cx.tree, root)
    } else {
        Position::first_in_node(root)
    }))
}

/// From an inline-block's first or last line, the line before or after
/// the inline-block in its outer context (`EscapeAtomicInlineAndFindLine`).
fn escape_inline_block(cx: &Context, position: Position, down: bool) -> Option<LineBox> {
    let ifc = caret::ifc_of_position(cx, position)?;
    let atomic = cx.ifc_owner(ifc.root);
    if !cx.tree.is_element(atomic) || cx.tree.node(atomic).is_anonymous() {
        return None;
    }
    let inline = cx.object(atomic).is_some_and(|object| object.is_atomic());
    if !inline {
        return None;
    }
    let outside = if down {
        Position::after(atomic)
    } else {
        Position::before(atomic)
    };
    let line = LineBox::of(cx, Placed::downstream(outside))?;
    let line = if down { line.next() } else { line.previous() }?;
    line.can_hold_caret().then_some(line)
}

/// The last candidate position on a line before the position's, by the
/// DOM's leaves (`PreviousRootInlineBoxCandidatePosition`).
fn previous_root_inline_box_candidate(
    cx: &Context,
    node: NodeId,
    placed: Placed,
) -> Option<Position> {
    let mut runner = previous_leaf(cx, node);
    while let Some(leaf) = runner {
        if !in_same_line_as_node(cx, leaf, placed) {
            break;
        }
        runner = previous_leaf(cx, leaf);
    }
    while let Some(leaf) = runner {
        if cx.tree.node(leaf).is_shadow_root() {
            return None;
        }
        let candidate = if cx.tree.is_br(leaf) {
            Position::before(leaf)
        } else {
            Position::editing_position_of(&cx.tree, leaf, text::node_caret_max_offset(cx, leaf))
        };
        if canonical::is_candidate(cx, candidate) {
            return Some(candidate);
        }
        runner = previous_leaf(cx, leaf);
    }
    None
}

/// The first candidate position on a line after the position's
/// (`NextRootInlineBoxCandidatePosition`).
fn next_root_inline_box_candidate(cx: &Context, node: NodeId, placed: Placed) -> Option<Position> {
    let mut runner = next_leaf(cx, node);
    while let Some(leaf) = runner {
        if !in_same_line_as_node(cx, leaf, placed) {
            break;
        }
        runner = next_leaf(cx, leaf);
    }
    while let Some(leaf) = runner {
        if cx.tree.node(leaf).is_shadow_root() {
            return None;
        }
        let candidate =
            Position::editing_position_of(&cx.tree, leaf, text::node_caret_min_offset(cx, leaf));
        if canonical::is_candidate(cx, candidate) {
            return Some(candidate);
        }
        runner = next_leaf(cx, leaf);
    }
    None
}

/// Whether a node's first position is on the same line as `placed`; a
/// node that does not render counts as on it.
fn in_same_line_as_node(cx: &Context, node: NodeId, placed: Placed) -> bool {
    if !cx.has_layout(node) {
        return true;
    }
    let first = Position::first_in_or_before_node(&cx.tree, node);
    visible_position::create(cx, Placed::downstream(first))
        .is_some_and(|visible| visible_position::in_same_line(cx, visible, placed))
}

/// The previous leaf in the flat tree, as if leaves formed one chain
/// (`PreviousAtomicLeafNode`).
fn previous_leaf(cx: &Context, start: NodeId) -> Option<NodeId> {
    let mut node = previous_considering_atomic(cx, start);
    while let Some(current) = node {
        if !cx.spend() {
            return None;
        }
        if cx.tree.is_atomic(current) {
            return Some(current);
        }
        node = previous_considering_atomic(cx, current);
    }
    None
}

fn previous_considering_atomic(cx: &Context, start: NodeId) -> Option<NodeId> {
    let Some(previous) = cx.tree.previous_sibling(start) else {
        return cx.tree.parent(start);
    };
    let mut node = previous;
    while !cx.tree.is_atomic(node) {
        node = cx.tree.last_child(node)?;
    }
    Some(node)
}

/// `NextAtomicLeafNode`.
fn next_leaf(cx: &Context, start: NodeId) -> Option<NodeId> {
    let mut node = next_considering_atomic(cx, start);
    while let Some(current) = node {
        if !cx.spend() {
            return None;
        }
        if cx.tree.is_atomic(current) {
            return Some(current);
        }
        node = next_considering_atomic(cx, current);
    }
    None
}

fn next_considering_atomic(cx: &Context, start: NodeId) -> Option<NodeId> {
    if !cx.tree.is_atomic(start) {
        if let Some(first) = cx.tree.first_child(start) {
            return Some(first);
        }
    }
    let mut node = start;
    loop {
        if let Some(next) = cx.tree.next_sibling(node) {
            return Some(next);
        }
        node = cx.tree.parent(node)?;
    }
}
