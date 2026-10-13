//! Line steps at a `<select>`, as Chrome 154 (macOS) takes them through the
//! control's user-agent shadow tree.
//!
//! A step forward from just before a select lands in its content: in a
//! drop-down's label, which the selection reports as the point before the
//! select, or on a list box's first row, reported before the row's element.
//! Each list box row is a line of its own, so steps then go from row to
//! row and leave the list box past its first or last. Steps that reach a
//! select any other way pass it as an atomic inline.

use super::context::Context;
use super::position::{Anchor, Position};
use crate::NodeId;
use crate::caret::geometry::{content_origin, content_width};
use crate::node::SelectContent;
use crate::select::{ListBoxRow, SelectDisplay};
use std::cmp::Ordering;
use style::computed_values::direction::T as Direction;

/// Where a line step at a select goes.
pub(super) enum SelectStep {
    /// To this position, reported as it is.
    To(Position),
    /// Out of a list box: the step goes on from after it.
    Leave(NodeId),
}

/// A step from a caret inside a select: from a list box row to the next or
/// previous one, or out of the list box; a caret between a drop-down's
/// options has no line to move to and stays.
pub(super) fn step_inside(cx: &Context, caret: Position, forward: bool) -> Option<SelectStep> {
    let (select, display) = enclosing_select(cx, caret)?;
    // The point before a select's first child is the point before the
    // select, where the step enters it instead.
    if caret == Position::offset(select, 0) {
        return None;
    }
    let at_end = caret.anchor == Anchor::AfterChildren
        || caret == Position::offset(select, cx.tree.dom_children(select).len());
    if caret.node == select && at_end {
        return None;
    }
    let SelectDisplay::ListBox { .. } = display else {
        return Some(SelectStep::To(caret));
    };
    let rows = line_rows(cx, select);
    let row = row_at(cx, &rows, caret)?;
    let next = if forward {
        rows.get(row + 1)
    } else {
        row.checked_sub(1).and_then(|row| rows.get(row))
    };
    Some(match (next, forward) {
        (Some(next), _) => SelectStep::To(Position::before(next.element)),
        (None, true) => SelectStep::Leave(select),
        (None, false) => SelectStep::To(Position::before(select)),
    })
}

/// A forward step from just before a select (`start`, made visible) enters
/// it: a drop-down's label, reported before the select, or a list box's
/// first row. An extension stays before the select, as Chrome keeps a
/// selection's end out of the control's shadow tree.
pub(super) fn step_into(cx: &Context, start: Position, extend: bool) -> Option<Position> {
    let select = select_after(cx, start)?;
    let display = cx.doc().select_display(select)?;
    let before = Position::before(select);
    match display {
        SelectDisplay::DropDown => {
            let selected = cx.doc().selected_option(select)?;
            let shows_label = !cx.doc().option_display_label(selected).is_empty();
            shows_label.then_some(before)
        }
        SelectDisplay::ListBox { .. } => {
            let first = line_rows(cx, select).first().copied()?;
            Some(if extend {
                before
            } else {
                Position::before(first.element)
            })
        }
    }
}

/// The horizontal position a step from a caret in a select keeps when no
/// earlier step set one: in a list box row's text, or else the select's
/// content start.
pub(super) fn line_x(cx: &Context, caret: Position) -> f32 {
    let Some((select, _)) = enclosing_select(cx, caret) else {
        return 0.0;
    };
    let select_node = cx.tree.node(select);
    let in_row = row_text_x(cx, select, select_node, caret);
    content_origin(select_node).0 + in_row.unwrap_or(0.0)
}

/// From a list box's content left to a caret on a row: the start of the
/// row's label, and in its text, the label's advance up to the caret's
/// offset. The offset counts the label's characters, which are the text's
/// when it has no white space to collapse.
fn row_text_x(
    cx: &Context,
    select: NodeId,
    select_node: &crate::Node,
    caret: Position,
) -> Option<f32> {
    let rows = line_rows(cx, select);
    let row = rows[row_at(cx, &rows, caret)?];
    let layout = select_node.element_data()?.select_layout.as_deref()?;
    let SelectContent::ListBox(list_box) = &layout.content else {
        return None;
    };
    let laid_out = list_box
        .rows
        .iter()
        .find(|laid_out| laid_out.element == row.element)?;
    let text = &laid_out.label;
    let rtl = select_node
        .primary_styles()
        .is_some_and(|style| style.clone_direction() == Direction::Rtl);
    let start = if rtl {
        content_width(select_node) - laid_out.inset - text.full_width() / text.scale()
    } else {
        laid_out.inset
    };
    let (Anchor::Offset(offset), true) = (caret.anchor, cx.tree.is_text(caret.node)) else {
        return Some(start);
    };
    let label = cx.doc().list_box_row_label(row);
    let byte = label
        .char_indices()
        .nth(offset)
        .map_or(label.len(), |(byte, _)| byte);
    let cursor = parley::Cursor::from_byte_index(text, byte, parley::Affinity::Downstream);
    Some(start + cursor.geometry(text, 0.0).x0 as f32 / text.scale())
}

/// The rendered select a position is inside, with how it presents.
fn enclosing_select(cx: &Context, position: Position) -> Option<(NodeId, SelectDisplay)> {
    let mut node = match position.anchor {
        Anchor::Offset(_) | Anchor::AfterChildren => position.node,
        Anchor::Before | Anchor::After => cx.tree.parent(position.node)?,
    };
    loop {
        if !cx.spend() {
            return None;
        }
        if let Some(display) = rendered_select(cx, node) {
            return Some((node, display));
        }
        node = cx.tree.parent(node)?;
    }
}

/// The select right after a position: the position before it, or the
/// offset before it in its parent.
fn select_after(cx: &Context, position: Position) -> Option<NodeId> {
    let node = match position.anchor {
        Anchor::Before => position.node,
        Anchor::Offset(offset) if !cx.tree.is_character_data(position.node) => {
            cx.tree.child_at(position.node, offset)?
        }
        _ => return None,
    };
    rendered_select(cx, node).map(|_| node)
}

fn rendered_select(cx: &Context, node: NodeId) -> Option<SelectDisplay> {
    let has_content = cx
        .tree
        .node(node)
        .element_data()
        .is_some_and(|element| element.is_laid_out_select());
    if !has_content || !cx.has_layout(node) {
        return None;
    }
    cx.doc().select_display(node)
}

/// A list box's rows that hold a caret: those with a label, as an empty
/// row has no text for a caret.
fn line_rows(cx: &Context, select: NodeId) -> Vec<ListBoxRow> {
    let doc = cx.doc();
    let mut rows = Vec::new();
    for row in doc.list_box_rows(select) {
        if !cx.spend() {
            break;
        }
        if !doc.list_box_row_label(row).is_empty() {
            rows.push(row);
        }
    }
    rows
}

/// The row a caret in a list box is on: the last row starting at or
/// before it, so a caret inside a row's element, or before it, is on it.
fn row_at(cx: &Context, rows: &[ListBoxRow], caret: Position) -> Option<usize> {
    rows.iter().rposition(|row| {
        if !cx.spend() {
            return false;
        }
        let start = Position::before(row.element);
        matches!(
            start.compare(caret, &cx.tree),
            Some(Ordering::Less | Ordering::Equal)
        )
    })
}
