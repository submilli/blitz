//! Canonical positions (Blink's `visible_units.cc`): of the positions that
//! render a caret in the same place, editing works with the most backward
//! candidate. The document is not editable, so editing boundaries never
//! apply.

use super::context::{Context, Kind};
use super::iterator::PositionIterator;
use super::position::Position;
use super::text;
use crate::NodeId;
use std::cmp::Ordering;

/// The most backward position rendering at the same place
/// (`MostBackwardCaretPosition`).
pub(super) fn most_backward(cx: &Context, position: Position) -> Position {
    let start = position.node;
    let boundary = enclosing_visual_boundary(cx, start);
    let adjusted = adjust_for_backward_iteration(cx, position);
    let mut last_visible = PositionIterator::new(cx, adjusted);
    let mut current = last_visible.clone();
    while !current.at_start(cx) {
        let Some(node) = current.node() else {
            break;
        };
        match backward_step(cx, &mut current, &mut last_visible, node, start, boundary) {
            Step::Return(found) => return found,
            Step::Continue => {}
        }
        current.decrement(cx);
    }
    last_visible.deprecated_position(cx).unwrap_or(position)
}

/// One position of the backward walk.
fn backward_step(
    cx: &Context,
    current: &mut PositionIterator,
    last_visible: &mut PositionIterator,
    node: NodeId,
    start: NodeId,
    boundary: Option<NodeId>,
) -> Step {
    let last = |it: &PositionIterator| Step::from(it.deprecated_position(cx));
    if !can_have_caret_position(cx, node) {
        return Step::Continue;
    }
    if ends_are_visually_distinct(cx, node) && Some(node) != boundary {
        return last(last_visible);
    }
    let Some(object) = cx.object(node).filter(|o| o.visible) else {
        return Step::Continue;
    };
    if is_streamer(cx, current) {
        *last_visible = current.clone();
    }
    if ends_are_visually_distinct(cx, node) && current.at_start_of_node(cx) {
        return last(last_visible);
    }
    if cx.tree.editing_ignores_content(node) || is_display_inside_table(cx, node) {
        return Step::from(current.at_end_of_node(cx).then(|| Position::after(node)));
    }
    if !object.is_text() || !text::has_non_collapsed_text(cx, node) {
        return Step::Continue;
    }
    if node != start {
        return Step::Return(Position::offset(node, text::caret_max_offset(cx, node)));
    }
    let Some(offset) = current.offset_in_text_node(cx) else {
        return Step::Continue;
    };
    if text::is_after_non_collapsed_character(cx, node, offset) {
        return Step::from(current.position(cx));
    }
    Step::Continue
}

/// The most forward position rendering at the same place
/// (`MostForwardCaretPosition`).
pub(super) fn most_forward(cx: &Context, position: Position) -> Position {
    let start = position.node;
    let boundary = enclosing_visual_boundary(cx, start);
    let initial = if position.is_after_anchor() {
        Position::editing_position_of(&cx.tree, start, text::node_caret_max_offset(cx, start))
    } else {
        position
    };
    let mut last_visible = PositionIterator::new(cx, initial);
    let mut current = last_visible.clone();
    while !current.at_end(cx) {
        let Some(node) = current.node() else {
            break;
        };
        if cx.tree.is_body(node) && current.at_end_of_node(cx) {
            break;
        }
        match forward_step(cx, &mut current, &mut last_visible, node, start, boundary) {
            Step::Return(found) => return found,
            Step::Continue => {}
        }
        current.increment(cx);
    }
    last_visible.deprecated_position(cx).unwrap_or(position)
}

/// Whether a walk for a canonical position ends, and where.
enum Step {
    Return(Position),
    Continue,
}

impl From<Option<Position>> for Step {
    fn from(found: Option<Position>) -> Self {
        found.map_or(Self::Continue, Self::Return)
    }
}

fn forward_step(
    cx: &Context,
    current: &mut PositionIterator,
    last_visible: &mut PositionIterator,
    node: NodeId,
    start: NodeId,
    boundary: Option<NodeId>,
) -> Step {
    let last = |it: &PositionIterator| Step::from(it.deprecated_position(cx));
    if !can_have_caret_position(cx, node) {
        return Step::Continue;
    }
    if ends_are_visually_distinct(cx, node) && Some(node) != boundary {
        return last(last_visible);
    }
    if boundary.and_then(|b| cx.tree.parent(b)) == Some(node) {
        return last(last_visible);
    }
    let Some(object) = cx.object(node).filter(|o| o.visible) else {
        return Step::Continue;
    };
    if is_streamer(cx, current) {
        *last_visible = current.clone();
    }
    if cx.tree.editing_ignores_content(node) || is_display_inside_table(cx, node) {
        if current.at_start_of_node(cx) {
            return Step::Return(Position::editing_position_of(&cx.tree, node, 0));
        }
        return Step::Continue;
    }
    if !object.is_text() || !text::has_non_collapsed_text(cx, node) {
        return Step::Continue;
    }
    if node != start {
        return Step::Return(Position::offset(node, text::caret_min_offset(cx, node)));
    }
    let Some(offset) = current.offset_in_text_node(cx) else {
        return Step::Continue;
    };
    if text::is_before_non_collapsed_character(cx, node, offset) {
        if let Some(found) = current.position(cx) {
            return Step::Return(found);
        }
    }
    Step::Continue
}

/// Whether a caret can render at the position
/// (`IsVisuallyEquivalentCandidate`).
pub(super) fn is_candidate(cx: &Context, position: Position) -> bool {
    let tree = &cx.tree;
    let anchor = position.node;
    let Some(object) = cx.object(anchor).filter(|o| o.visible) else {
        return false;
    };
    let parent_selectable = || {
        tree.parent(anchor)
            .and_then(|parent| cx.object(parent))
            .is_some_and(|parent| parent.selectable)
    };
    if object.kind == Kind::Br {
        if position.is_after_anchor() || position.editing_offset(tree) != 0 {
            return false;
        }
        return parent_selectable();
    }
    if object.kind == Kind::Text {
        return object.selectable && in_rendered_text(cx, position);
    }
    if is_svg(cx, anchor) {
        return false;
    }
    if is_display_inside_table(cx, anchor) || tree.editing_ignores_content(anchor) {
        if !position.at_first_editing_position_for_node(tree)
            && !position.at_last_editing_position_for_node(tree)
        {
            return false;
        }
        return parent_selectable();
    }
    if tree.is_document_element(anchor) || tree.is_document(anchor) || !object.selectable {
        return false;
    }
    let block_like = object.is_block_flow_or_flex_grid();
    if block_like && (cx.height(anchor) > 0.0 || tree.is_body(anchor)) {
        // Only an empty block holds a caret, at its start.
        return !has_rendered_descendants_with_height(cx, anchor)
            && position.at_first_editing_position_for_node(tree);
    }
    false
}

/// `CanonicalPositionOf`.
pub(super) fn canonical(cx: &Context, position: Position) -> Option<Position> {
    let backward = most_backward(cx, position);
    if is_candidate(cx, backward) {
        return Some(backward);
    }
    let forward = most_forward(cx, position);
    if is_candidate(cx, forward) {
        return Some(forward);
    }
    // Neither stays in the block: search outward for a candidate.
    let next = next_candidate(cx, position).map(|c| canonicalize_candidate(cx, c));
    let previous = previous_candidate(cx, position).map(|c| canonicalize_candidate(cx, c));
    if cx.tree.is_document(position.node) {
        return next.or(previous);
    }
    let (Some(next), Some(previous)) = (next, previous) else {
        return next.or(previous);
    };
    let node = position.container(&cx.tree);
    let next_same = in_same_block(cx, node, next.node);
    let previous_same = in_same_block(cx, node, previous.node);
    if previous_same && !next_same {
        return Some(previous);
    }
    Some(next)
}

fn canonicalize_candidate(cx: &Context, candidate: Position) -> Position {
    let upstream = most_backward(cx, candidate);
    if is_candidate(cx, upstream) {
        upstream
    } else {
        candidate
    }
}

pub(super) fn next_candidate(cx: &Context, position: Position) -> Option<Position> {
    scan_for_candidate(cx, position, true)
}

pub(super) fn previous_candidate(cx: &Context, position: Position) -> Option<Position> {
    scan_for_candidate(cx, position, false)
}

/// The first candidate after (or before) a position, in iterator order.
/// Line steps scan from every leaf they pass, so a scan stops at a node
/// start an earlier scan passed, whose candidate is the same.
fn scan_for_candidate(cx: &Context, position: Position, forward: bool) -> Option<Position> {
    let mut it = PositionIterator::new(cx, position);
    let mut passed = Vec::new();
    let step = |it: &mut PositionIterator| {
        if forward {
            it.increment(cx)
        } else {
            it.decrement(cx)
        }
    };
    step(&mut it);
    let found = loop {
        let done = if forward {
            it.at_end(cx)
        } else {
            it.at_start(cx)
        };
        if done {
            break None;
        }
        if let Some(current) = it.position(cx) {
            if is_candidate(cx, current) {
                break Some(current);
            }
            if it.at_start_of_node(cx) {
                if let Some(found) = cx.remembered_candidate(current, forward) {
                    break found;
                }
                passed.push(current);
            }
        }
        step(&mut it);
    };
    cx.remember_candidate(passed, forward, found);
    found
}

/// Whether `new` is in the block flow element around `original`.
fn in_same_block(cx: &Context, original: Option<NodeId>, new: NodeId) -> bool {
    let Some(original) = original else {
        return false;
    };
    if cx.tree.is_inclusive_descendant(new, original) && new != original {
        return true;
    }
    enclosing_block_flow_element(cx, original) == enclosing_block_flow_element(cx, new)
}

/// `EnclosingBlockFlowElement`, over the DOM tree.
pub(super) fn enclosing_block_flow_element(cx: &Context, node: NodeId) -> Option<NodeId> {
    let is_block_flow =
        |id: NodeId| cx.tree.is_element(id) && cx.object(id).is_some_and(|o| o.is_block_flow());
    if is_block_flow(node) {
        return Some(node);
    }
    let mut current = cx.tree.node(node).parent;
    while let Some(id) = current {
        if is_block_flow(id) || cx.tree.is_body(id) {
            return Some(id);
        }
        current = cx.tree.node(id).parent;
    }
    None
}

fn adjust_for_backward_iteration(cx: &Context, position: Position) -> Position {
    if !position.is_after_anchor() {
        return position;
    }
    if cx.tree.is_user_select_contain(position.node) {
        return position.to_offset_in_anchor(&cx.tree).unwrap_or(position);
    }
    let offset = text::node_caret_max_offset(cx, position.node);
    Position::editing_position_of(&cx.tree, position.node, offset)
}

/// A position in a Text node, or before a node, which canonical positions
/// prefer (`IsStreamer`).
fn is_streamer(cx: &Context, it: &PositionIterator) -> bool {
    match it.node() {
        None => true,
        Some(node) => cx.tree.is_atomic(node) || it.at_start_of_node(cx),
    }
}

/// `EndsOfNodeAreVisuallyDistinctPositions`: block-level boxes, and
/// non-empty inline-level boxes with no content, have distinct sides.
pub(super) fn ends_are_visually_distinct(cx: &Context, node: NodeId) -> bool {
    let Some(object) = cx.object(node) else {
        return false;
    };
    if !object.is_inline() {
        return true;
    }
    if cx.tree.is_table(node) {
        return false;
    }
    let Some(size) = cx.size(node) else {
        return false;
    };
    object.is_atomic()
        && cx.tree.is_element(node)
        && !cx.tree.editing_ignores_content(node)
        && size.width > 0.0
        && size.height > 0.0
        && !has_rendered_descendants_with_height(cx, node)
}

fn enclosing_visual_boundary(cx: &Context, node: NodeId) -> Option<NodeId> {
    let mut current = Some(node);
    while let Some(id) = current {
        if ends_are_visually_distinct(cx, id) {
            return Some(id);
        }
        current = cx.tree.parent(id);
    }
    None
}

/// `HasRenderedNonAnonymousDescendantsWithHeight`.
pub(super) fn has_rendered_descendants_with_height(cx: &Context, node: NodeId) -> bool {
    cx.rendered_with_height(node, || scan_rendered_descendants(cx, node))
}

fn scan_rendered_descendants(cx: &Context, node: NodeId) -> bool {
    let mut pending: Vec<NodeId> = cx
        .tree
        .layout_children(node)
        .iter()
        .rev()
        .copied()
        .collect();
    while let Some(id) = pending.pop() {
        if !cx.spend() {
            return false;
        }
        let Some(object) = cx.object(id) else {
            continue;
        };
        match object.kind {
            Kind::Text | Kind::Br => {
                if text::has_non_collapsed_text(cx, id) {
                    return true;
                }
            }
            Kind::Inline => {
                if is_empty_inline(cx, id) && cx.height(id) > 0.0 {
                    return true;
                }
            }
            Kind::Atomic { .. } | Kind::Block { .. } => {
                if cx.height(id) > 0.0 {
                    return true;
                }
            }
        }
        pending.extend(cx.tree.layout_children(id).iter().rev().copied());
    }
    false
}

/// An inline box holding nothing but floats, positioned boxes, empty
/// inline boxes and collapsible white space.
///
/// Iterative, as page markup controls the nesting depth, and memoized for
/// every inline box it decides, as scans ask about each nested box in turn.
fn is_empty_inline(cx: &Context, node: NodeId) -> bool {
    if let Some(known) = cx.empty_inline(node) {
        return known;
    }
    // The inline boxes being searched, outermost first, with the index of
    // the next child to look at in each.
    let mut open = vec![(node, 0)];
    while let Some((current, next)) = open.last_mut() {
        let Some(&child) = cx.tree.layout_children(*current).get(*next) else {
            cx.remember_empty_inline(*current, true);
            open.pop();
            continue;
        };
        *next += 1;
        let Some(object) = cx.object(child) else {
            continue;
        };
        let empty = match object.kind {
            Kind::Block { .. } => true,
            Kind::Text => !text::has_non_collapsed_text(cx, child),
            Kind::Inline => match cx.empty_inline(child) {
                Some(known) => known,
                None => {
                    open.push((child, 0));
                    continue;
                }
            },
            Kind::Br | Kind::Atomic { .. } => false,
        };
        if !empty {
            // Content in a box is content in each box around it.
            for (inline, _) in open {
                cx.remember_empty_inline(inline, false);
            }
            return false;
        }
    }
    true
}

/// `InRenderedText`: a caret offset at a grapheme boundary of rendered text.
fn in_rendered_text(cx: &Context, position: Position) -> bool {
    let node = position.node;
    if !cx.tree.is_text(node) {
        return false;
    }
    let offset = position.editing_offset(&cx.tree);
    if !text::contains_caret_offset(cx, node, offset) {
        return false;
    }
    offset == text::caret_min_offset(cx, node)
        || offset == cx.next_grapheme_boundary(node, cx.previous_grapheme_boundary(node, offset))
}

fn can_have_caret_position(cx: &Context, node: NodeId) -> bool {
    !is_svg(cx, node)
}

/// A non-text SVG element, which holds no caret position.
fn is_svg(cx: &Context, node: NodeId) -> bool {
    use markup5ever::{local_name, ns};
    cx.tree.node(node).element_data().is_some_and(|element| {
        element.name.ns == ns!(svg)
            && !matches!(
                element.name.local,
                local_name!("text") | local_name!("foreignObject")
            )
    })
}

fn is_display_inside_table(cx: &Context, node: NodeId) -> bool {
    cx.tree.is_table(node) && cx.has_layout(node)
}

/// Tree order of two positions; `Equal` for disconnected ones.
pub(super) fn order(cx: &Context, a: Position, b: Position) -> Ordering {
    a.compare(b, &cx.tree).unwrap_or(Ordering::Equal)
}
