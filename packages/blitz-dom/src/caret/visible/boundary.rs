//! Line boundaries (Blink's `visible_units_line.cc`): the first and last
//! leaf of the line box a position's caret draws on.

use super::canonical;
use super::caret::{self, Caret, Placed, Side};
use super::context::Context;
use super::ifc::ItemKind;
use super::line::LineBox;
use super::position::{Anchor, Position};

/// `PositionForStartOfLine`: before the line's first leaf from the DOM.
pub(super) fn start_of_line_box(cx: &Context, line: &LineBox) -> Option<Placed> {
    let rtl = line.ifc.lines[line.index].rtl;
    let index = if rtl {
        last_leaf(line)
    } else {
        first_leaf(line)
    }?;
    let item = &line.ifc.items[index];
    match item.kind {
        ItemKind::Atomic(node) => Some(Placed::downstream(Position::before(node))),
        // Not a leaf, so never a line's edge.
        ItemKind::Float(_) => None,
        ItemKind::Text { .. } => {
            let offset = if rtl == item.rtl {
                item.content.start
            } else {
                item.content.end
            };
            at_text(cx, line, index, offset)
        }
    }
}

/// `PositionForEndOfLine`: after the line's last leaf from the DOM, or
/// before a `<br>` that is its only leaf.
pub(super) fn end_of_line_box(cx: &Context, line: &LineBox) -> Option<Placed> {
    let rtl = line.ifc.lines[line.index].rtl;
    let index = if rtl {
        first_leaf(line)
    } else {
        last_leaf(line)
    }?;
    let item = &line.ifc.items[index];
    let node = item.kind_node()?;
    if cx.tree.is_br(node) {
        return Some(Placed::downstream(Position::before(node)));
    }
    match item.kind {
        ItemKind::Atomic(node) => Some(Placed::downstream(Position::after(node))),
        // Not a leaf, so never a line's edge.
        ItemKind::Float(_) => None,
        ItemKind::Text { line_break, .. } => {
            let offset = if rtl == item.rtl && !line_break {
                item.content.end
            } else {
                item.content.start
            };
            at_text(cx, line, index, offset)
        }
    }
}

/// `MoveToFirstNonPseudoLeaf`: a forced break counts only when nothing
/// follows it on the line.
fn first_leaf(line: &LineBox) -> Option<usize> {
    let range = line.ifc.lines[line.index].items.clone();
    let last = range.end.checked_sub(1)?;
    range.into_iter().find(|&index| {
        let item = &line.ifc.items[index];
        let line_break = item.is_line_break();
        item.kind_node().is_some() && (!line_break || index == last)
    })
}

/// `MoveToLastNonPseudoLeaf`: the last leaf before a forced break, or the
/// break when nothing precedes it.
fn last_leaf(line: &LineBox) -> Option<usize> {
    let mut found = None;
    for index in line.ifc.lines[line.index].items.clone() {
        let item = &line.ifc.items[index];
        if item.kind_node().is_none() {
            continue;
        }
        if item.is_line_break() && found.is_some() {
            break;
        }
        found = Some(index);
    }
    found
}

/// `PositionForPointInText` at a line's edge leaf.
fn at_text(cx: &Context, line: &LineBox, item: usize, offset: usize) -> Option<Placed> {
    let caret = Caret {
        ifc: line.ifc.clone(),
        item,
        side: Side::AtText(offset),
    };
    super::bidi::adjust_for_hit_test(caret).to_position(cx)
}

/// `StartPositionForLine`.
fn start_position_for_line(cx: &Context, placed: Placed) -> Option<Placed> {
    position_for_line(cx, placed, start_of_line_box)
}

/// `EndPositionForLine`.
fn end_position_for_line(cx: &Context, placed: Placed) -> Option<Placed> {
    position_for_line(cx, placed, end_of_line_box)
}

/// An edge of the line box a position's caret draws on, or the start of a
/// block without line boxes.
fn position_for_line(
    cx: &Context,
    placed: Placed,
    edge: fn(&Context, &LineBox) -> Option<Placed>,
) -> Option<Placed> {
    let adjusted = caret::inline_adjusted(cx, placed);
    if let Some(adjusted) = adjusted.filter(|a| caret::ifc_of_position(cx, a.position).is_some()) {
        let caret = caret::inline_caret(cx, adjusted)?;
        return edge(cx, &LineBox::of_caret(&caret));
    }
    block_start(cx, placed)
}

/// A block without line boxes holds a position at its start.
fn block_start(cx: &Context, placed: Placed) -> Option<Placed> {
    let position = placed.position;
    let block = cx
        .object(position.node)
        .is_some_and(|o| o.is_block_flow_or_flex_grid());
    (block && position.editing_offset(&cx.tree) == 0).then_some(placed)
}

/// `StartOfLine`, used by visible positions.
pub(super) fn start_of_line(cx: &Context, placed: Placed) -> Option<Placed> {
    start_position_for_line(cx, placed)
}

/// `LogicalStartOfLine`.
pub(super) fn logical_start_of_line(cx: &Context, placed: Placed) -> Option<Placed> {
    let normalized = if needs_caret_normalization(cx, placed.position) {
        Placed {
            position: canonical::most_backward(cx, placed.position),
            ..placed
        }
    } else {
        placed
    };
    start_position_for_line(cx, normalized)
}

/// `LogicalEndOfLine`.
pub(super) fn logical_end_of_line(cx: &Context, placed: Placed) -> Option<Placed> {
    let normalized = if needs_caret_normalization(cx, placed.position) {
        Placed {
            position: canonical::most_forward(cx, placed.position),
            ..placed
        }
    } else {
        placed
    };
    end_position_for_line(cx, normalized)
}

/// A position inside a block flow, not in a Text node or beside a node.
fn needs_caret_normalization(cx: &Context, position: Position) -> bool {
    matches!(position.anchor, Anchor::Offset(_) | Anchor::AfterChildren)
        && !cx.tree.is_text(position.node)
        && cx.object(position.node).is_some_and(|o| o.is_block_flow())
}
