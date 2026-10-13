//! Visible positions (Blink's `VisiblePosition::Create`) and whether two
//! positions are on one line (`InSameLine`).

use super::boundary;
use super::canonical;
use super::caret::{self, Affinity, Placed};
use super::context::Context;
use super::position::{Anchor, Position};
use crate::NodeId;
use std::cmp::Ordering;
use std::rc::Rc;
use style::values::computed::Display;
use style::values::specified::box_::{DisplayInside, DisplayOutside};

/// The canonical position for a position and affinity: downstream unless
/// the caret differs upstream, at a soft wrap.
pub(super) fn create(cx: &Context, placed: Placed) -> Option<Placed> {
    let deep = canonical::canonical(cx, placed.position)?;
    let downstream = Placed::downstream(deep);
    if placed.affinity == Affinity::Downstream {
        if placed.position == deep {
            return Some(downstream);
        }
        // A canonical position on an earlier line of the same context
        // gives way to the start of the position's own line.
        let earlier = canonical::order(cx, deep, placed.position) == Ordering::Less;
        if earlier && in_different_lines_of_same_context(cx, placed, downstream) {
            if let Some(start) = boundary::start_of_line(cx, placed) {
                return Some(start);
            }
        }
        return Some(downstream);
    }
    let upstream = Placed {
        position: deep,
        affinity: Affinity::Upstream,
    };
    if in_same_line(cx, downstream, upstream) {
        Some(downstream)
    } else {
        Some(upstream)
    }
}

fn in_different_lines_of_same_context(cx: &Context, a: Placed, b: Placed) -> bool {
    if a == b || !cx.has_layout(a.position.node) || !cx.has_layout(b.position.node) {
        return false;
    }
    if in_same_line(cx, a, b) {
        return false;
    }
    match (
        caret::ifc_of_position(cx, a.position),
        caret::ifc_of_position(cx, b.position),
    ) {
        (Some(first), Some(second)) => Rc::ptr_eq(&first, &second),
        _ => false,
    }
}

/// `InSameLine`: in one line box of one formatting context; a position in
/// an inline-block shares its line with the content of its outer block.
pub(super) fn in_same_line(cx: &Context, a: Placed, b: Placed) -> bool {
    let first = caret::ifc_of_position(cx, a.position);
    let second = caret::ifc_of_position(cx, b.position);
    if first.is_some() || second.is_some() {
        let inline_block = |ifc: &Option<Rc<super::ifc::Ifc>>| {
            ifc.as_ref()
                .is_some_and(|ifc| is_inline_block(cx, cx.ifc_owner(ifc.root)))
        };
        if inline_block(&first) || inline_block(&second) {
            return forward_inline_contents(cx, a.position)
                == forward_inline_contents(cx, b.position);
        }
        let same_context = match (&first, &second) {
            (Some(x), Some(y)) => Rc::ptr_eq(x, y),
            _ => false,
        };
        return same_context && in_same_line_box(cx, a, b);
    }
    // Neither is in inline content: compare where their lines start.
    let start_a = boundary::start_of_line(cx, a).map(|p| p.position);
    let start_b = boundary::start_of_line(cx, b).map(|p| p.position);
    if start_a == start_b {
        return true;
    }
    let canonical_a = start_a.and_then(|p| canonical::canonical(cx, p));
    canonical_a.is_some()
        && (canonical_a == start_b
            || canonical_a == start_b.and_then(|p| canonical::canonical(cx, p)))
}

/// `InSameNGLineBox`, from each position's own caret.
fn in_same_line_box(cx: &Context, a: Placed, b: Placed) -> bool {
    let Some(first) = caret::inline_caret(cx, a) else {
        return false;
    };
    let Some(second) = caret::inline_caret(cx, b) else {
        return false;
    };
    Rc::ptr_eq(&first.ifc, &second.ifc) && first.line() == second.line()
}

/// An inline-level block container: an inline-block, or an inline button,
/// which is laid out as one.
fn is_inline_block(cx: &Context, node: NodeId) -> bool {
    let data = cx.tree.node(node);
    let display = data.display_style().unwrap_or(Display::inline());
    display.outside() == DisplayOutside::Inline
        && (display.inside() == DisplayInside::FlowRoot || data.flags.is_button_root())
}

/// The outermost block whose inline content holds the node at or after a
/// position (`FindForwardInlineContents`).
fn forward_inline_contents(cx: &Context, position: Position) -> Option<NodeId> {
    let mut node = range_first_node(cx, position);
    while let Some(current) = node {
        if !cx.spend() {
            return None;
        }
        if let Some(contents) = inline_contents_of(cx, current) {
            return Some(contents);
        }
        node = cx
            .tree
            .next_in_preorder(current, cx.tree.is_user_select_contain(current));
    }
    None
}

/// `NodeAsRangeFirstNode`.
fn range_first_node(cx: &Context, position: Position) -> Option<NodeId> {
    let tree = &cx.tree;
    let position = match position.anchor {
        Anchor::Offset(_) => position,
        _ => position.to_offset_in_anchor(tree)?,
    };
    let Anchor::Offset(offset) = position.anchor else {
        return None;
    };
    if tree.is_text(position.node) {
        return Some(position.node);
    }
    if let Some(child) = tree.child_at(position.node, offset) {
        return Some(child);
    }
    if offset == 0 {
        return Some(position.node);
    }
    cx.tree.next_in_preorder(position.node, true)
}

/// The block whose content box holds a node's box: the formatting
/// context it is laid out in, or its nearest block ancestor.
fn containing_block(cx: &Context, node: NodeId) -> Option<NodeId> {
    if let Some(ifc) = cx.ifc_of(node) {
        return Some(cx.ifc_owner(ifc.root));
    }
    let mut current = cx.tree.parent(node);
    while let Some(id) = current {
        if cx.object(id).is_some_and(|o| o.block_container) {
            return Some(id);
        }
        current = cx.tree.parent(id);
    }
    None
}

/// `ComputeInlineContentsAsBlockFlow`: the block flow whose inline content
/// holds a node, through nested inline-level blocks to the outermost.
fn inline_contents_of(cx: &Context, node: NodeId) -> Option<NodeId> {
    let object = cx.object(node)?;
    let mut block = if object.block_container {
        node
    } else {
        containing_block(cx, node)?
    };
    // Only a block whose children are inline holds inline contents.
    cx.own_ifc(block)?;
    while let Some(outer) = cx
        .object(block)
        .filter(|o| o.is_atomic())
        .and_then(|_| cx.ifc_of(block))
    {
        block = cx.ifc_owner(outer.root);
    }
    Some(block)
}
