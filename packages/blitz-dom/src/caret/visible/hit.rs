//! Hit testing at a point, as Blink's `PositionForPoint` family does it for
//! a line's leaves, atomic inlines, floats and block containers.

use super::boundary;
use super::caret::{Caret, Placed, Side};
use super::context::Context;
use super::ifc::{Ifc, ItemKind};
use super::line::LineBox;
use super::position::Position;
use crate::NodeId;
use std::rc::Rc;

/// Nested boxes a hit test enters; page markup controls the depth, so it
/// is bounded.
const MAX_HIT_DEPTH: usize = 64;

/// `PositionForPointInChild`: the position at `x` inside one item.
pub(super) fn point_in_child(
    cx: &Context,
    ifc: &Rc<Ifc>,
    index: usize,
    x: f32,
    y: f32,
    depth: usize,
) -> Option<Placed> {
    let item = &ifc.items[index];
    match item.kind {
        ItemKind::Text { node: None, .. } => None,
        ItemKind::Text { .. } => point_in_text(cx, ifc, index, x),
        ItemKind::Atomic(node) => point_in_box(cx, node, x, y, depth + 1),
        ItemKind::Float(node) => point_in_float(cx, node, x, y, depth + 1),
    }
}

/// `PositionForPointInText`: the caret boundary nearest `x`.
fn point_in_text(cx: &Context, ifc: &Rc<Ifc>, index: usize, x: f32) -> Option<Placed> {
    let item = &ifc.items[index];
    // The nearest boundary; at a glyph's middle, the one before it.
    let mut best = (item.content.start, f32::INFINITY);
    for (step, &caret_x) in item.carets.iter().enumerate() {
        let distance = (caret_x - x).abs();
        if distance < best.1 {
            best = (item.content.start + step, distance);
        }
    }
    let caret = Caret {
        ifc: ifc.clone(),
        item: index,
        side: Side::AtText(best.0),
    };
    super::bidi::adjust_for_hit_test(caret).to_position(cx)
}

/// `LayoutObject::PositionForPoint` of an atomic inline with a point in
/// document coordinates: outside the box, the position before or after it
/// (the inline direction decides first); inside, the position in its
/// content.
fn point_in_box(cx: &Context, node: NodeId, x: f32, y: f32, depth: usize) -> Option<Placed> {
    let size = cx.size(node)?;
    let origin = cx.tree.node(node).unrounded_absolute_position(0.0, 0.0);
    let outside = if x < origin.x {
        Some(true)
    } else if x >= origin.x + size.width {
        Some(false)
    } else if y < origin.y {
        Some(true)
    } else if y >= origin.y + size.height {
        Some(false)
    } else {
        None
    };
    if let Some(before) = outside {
        let position = if before {
            Position::first_in_or_before_node(&cx.tree, node)
        } else {
            Position::last_in_or_after_node(&cx.tree, node)
        };
        return Some(Placed::downstream(position));
    }
    point_in_block(cx, node, x, y, depth)
}

/// `PhysicalBoxFragment::PositionForPoint` of a block container: in its
/// inline content, or in the first block child that ends below the point
/// (else the last), descending until inline content.
fn point_in_block(cx: &Context, node: NodeId, x: f32, y: f32, depth: usize) -> Option<Placed> {
    if depth > MAX_HIT_DEPTH {
        return None;
    }
    if let Some(ifc) = cx.own_ifc(node) {
        return point_in_context(cx, &ifc, x, y, depth);
    }
    let children: Vec<NodeId> = match cx.tree.node(node).layout_children.borrow().as_deref() {
        Some(children) => children.to_vec(),
        None => return Some(Placed::downstream(Position::first_in_node(node))),
    };
    let mut last = None;
    for child in children {
        if !is_hit_test_candidate(cx, child) {
            continue;
        }
        let bottom = cx.tree.node(child).unrounded_absolute_position(0.0, 0.0).y + cx.height(child);
        if y < bottom {
            return point_in_block(cx, child, x, y, depth + 1);
        }
        last = Some(child);
    }
    match last {
        Some(child) => point_in_block(cx, child, x, y, depth + 1),
        None => Some(Placed::downstream(Position::first_in_or_before_node(
            &cx.tree, node,
        ))),
    }
}

/// `LayoutObject::PositionForPoint` of a float: a replaced element's
/// position before or after it, by the side of its middle the point is on;
/// a block's position in its content.
fn point_in_float(cx: &Context, node: NodeId, x: f32, y: f32, depth: usize) -> Option<Placed> {
    if cx.object(node).is_some_and(|o| o.block_container) {
        return point_in_block(cx, node, x, y, depth);
    }
    let size = cx.size(node)?;
    let origin = cx.tree.node(node).unrounded_absolute_position(0.0, 0.0);
    let before = if y < origin.y {
        true
    } else if y >= origin.y + size.height {
        false
    } else {
        x <= origin.x + size.width / 2.0
    };
    Some(Placed::downstream(if before {
        Position::before(node)
    } else {
        Position::after(node)
    }))
}

/// A visible, in-flow box with height.
fn is_hit_test_candidate(cx: &Context, node: NodeId) -> bool {
    let data = cx.tree.node(node);
    let laid_out = data.is_anonymous() || cx.object(node).is_some_and(|o| o.visible);
    laid_out && cx.height(node) > 0.0 && !crate::caret::order::is_out_of_flow(data)
}

/// `PositionForPointInInlineFormattingContext`: in the line under `y`, or at
/// the start of the first line above or the end of the last line below.
fn point_in_context(cx: &Context, ifc: &Rc<Ifc>, x: f32, y: f32, depth: usize) -> Option<Placed> {
    let mut before = None;
    let mut after = None;
    for (index, line) in ifc.lines.iter().enumerate() {
        if y < line.top {
            before.get_or_insert(index);
            continue;
        }
        if y >= line.bottom {
            after = Some(index);
            continue;
        }
        if let Some(found) = point_in_line(cx, ifc, index, x, y, depth) {
            return Some(found);
        }
    }
    let line = |index| LineBox::new(ifc.clone(), index);
    if let Some(index) = before {
        if let Some(found) = boundary::start_of_line_box(cx, &line(index)) {
            return Some(Placed::downstream(found.position));
        }
    }
    let index = after?;
    boundary::end_of_line_box(cx, &line(index)).map(|found| Placed::downstream(found.position))
}

/// `PositionForPointInInlineBox` over a line: the child under `x`, else the
/// closest one after it, else the closest one before it.
fn point_in_line(
    cx: &Context,
    ifc: &Rc<Ifc>,
    line: usize,
    x: f32,
    y: f32,
    depth: usize,
) -> Option<Placed> {
    let range = ifc.lines[line].items.clone();
    let mut closest_after: Option<usize> = None;
    let mut closest_before: Option<usize> = None;
    for index in range {
        let item = &ifc.items[index];
        if matches!(item.kind, ItemKind::Text { node: None, .. }) {
            continue;
        }
        // A float is entered only under the point, never as the closest.
        let float = matches!(item.kind, ItemKind::Float(_));
        if float && (x < item.left || x >= item.right) {
            continue;
        }
        if x < item.left {
            if closest_after.is_none_or(|best| item.left < ifc.items[best].left) {
                closest_after = Some(index);
            }
            continue;
        }
        if x >= item.right {
            if closest_before.is_none_or(|best| item.right > ifc.items[best].right) {
                closest_before = Some(index);
            }
            continue;
        }
        if let Some(found) = point_in_child(cx, ifc, index, x, y, depth) {
            return Some(found);
        }
    }
    closest_after
        .and_then(|index| point_in_child(cx, ifc, index, x, y, depth))
        .or_else(|| closest_before.and_then(|index| point_in_child(cx, ifc, index, x, y, depth)))
}
