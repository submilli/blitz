//! Paragraph movement (Blink's `SelectionModifier::NextParagraphPosition`
//! and `position_units_paragraph.cc`): line steps until the caret leaves
//! the paragraph it started in.

use super::caret::Placed;
use super::context::{Context, Kind};
use super::line;
use super::position::Position;
use super::text;
use super::visible_position;
use crate::NodeId;
use std::collections::HashSet;
use style::computed_values::white_space_collapse::T as WhiteSpace;

/// Line steps a paragraph step takes at most.
const MAX_LINES: usize = 100_000;

/// The position nearest `x` on the first line of the next paragraph.
pub(super) fn next_paragraph_position(cx: &Context, passed: Placed, x: f32) -> Option<Placed> {
    paragraph_step(cx, passed, x, true)
}

/// The position nearest `x` on the last line of the previous paragraph.
pub(super) fn previous_paragraph_position(cx: &Context, passed: Placed, x: f32) -> Option<Placed> {
    paragraph_step(cx, passed, x, false)
}

fn paragraph_step(cx: &Context, passed: Placed, x: f32, down: bool) -> Option<Placed> {
    let start = start_of_visible_paragraph(cx, passed);
    let mut position = passed;
    // Blink steps until the caret leaves the paragraph. A line step that
    // comes back to a position it left would repeat forever (Blink's line
    // steps do not cycle; ours can where layout differs, as at a button's
    // inner content), so a repeated position (including the one
    // the step started from) ends the step there.
    let mut visited = HashSet::from([passed.position]);
    for _ in 0..MAX_LINES {
        let stepped = if down {
            line::next_line_position(cx, position, x)
        } else {
            line::previous_line_position(cx, position, x)
        };
        let Some(next) = stepped.and_then(|p| visible_position::create(cx, p)) else {
            break;
        };
        if !visited.insert(next.position) {
            break;
        }
        position = next;
        if start_of_visible_paragraph(cx, position) != start {
            break;
        }
    }
    Some(position)
}

/// `StartOfParagraph` of a visible position, as a visible position.
fn start_of_visible_paragraph(cx: &Context, placed: Placed) -> Option<Position> {
    let start = start_of_paragraph(cx, placed.position)?;
    visible_position::create(cx, Placed::downstream(start)).map(|v| v.position)
}

/// `StartOfParagraphAlgorithm`: back through the enclosing block from the
/// position to the nearest break, block boundary or preserved newline.
pub(super) fn start_of_paragraph(cx: &Context, position: Position) -> Option<Position> {
    let tree = &cx.tree;
    let start = position.node;
    if is_non_inline_table_image_or_hr(cx, start) {
        return Some(Position::before(start));
    }
    let block = enclosing_block(cx, Position::first_in_or_before_node(tree, start));
    let mut candidate = position;
    let offset = position.editing_offset(tree);
    let mut walker = Walker {
        cx,
        start,
        block,
        current: Some(start),
        position,
    };
    while let Some(node) = walker.current {
        let Some(object) = cx.object(node).filter(|o| o.visible) else {
            walker.previous();
            continue;
        };
        if object.kind == Kind::Br || is_enclosing_block(cx, node) {
            break;
        }
        if object.kind == Kind::Text && text::has_non_collapsed_text(cx, node) {
            if preserves_breaks(cx, node) {
                if let Some(found) = last_newline(cx, node, (node == start).then_some(offset)) {
                    return Some(Position::offset(node, found + 1));
                }
            }
            candidate = Position::offset(node, 0);
            walker.previous();
        } else if tree.editing_ignores_content(node) || tree.is_table(node) {
            if object.selectable {
                candidate = Position::before(node);
            }
            walker.current = walker.previous_skipping_children(node);
        } else {
            walker.previous();
        }
    }
    Some(candidate)
}

/// The backward walk of `StartOfParagraphAlgorithm`, within the block.
struct Walker<'a, 'b> {
    cx: &'a Context<'b>,
    start: NodeId,
    block: Option<NodeId>,
    current: Option<NodeId>,
    position: Position,
}

impl Walker<'_, '_> {
    fn previous(&mut self) {
        let Some(current) = self.current else {
            return;
        };
        if !self.cx.spend() {
            self.current = None;
            return;
        }
        if current == self.start {
            // The first step takes the position's own offset into account.
            match self.position.node_before(&self.cx.tree) {
                None => {
                    self.current = self.previous_skipping_children(current);
                    return;
                }
                Some(before) if before != current => {
                    self.current = Some(before);
                    return;
                }
                Some(_) => {}
            }
        }
        self.current = previous_post_order(self.cx, current, self.block);
    }

    fn previous_skipping_children(&self, node: NodeId) -> Option<NodeId> {
        let mut current = Some(node);
        while let Some(id) = current {
            if Some(id) == self.block {
                return None;
            }
            if let Some(previous) = self.cx.tree.previous_sibling(id) {
                return Some(previous);
            }
            current = self.cx.tree.parent(id);
        }
        None
    }
}

/// `FlatTreeTraversal::PreviousPostOrder`, staying within `stay_within`.
fn previous_post_order(
    cx: &Context,
    current: NodeId,
    stay_within: Option<NodeId>,
) -> Option<NodeId> {
    if let Some(last) = cx.tree.last_child(current) {
        return Some(last);
    }
    let mut node = current;
    loop {
        if Some(node) == stay_within {
            return None;
        }
        if let Some(previous) = cx.tree.previous_sibling(node) {
            return Some(previous);
        }
        node = cx.tree.parent(node)?;
    }
}

/// `EnclosingBlock`: the nearest ancestor of the position's anchor (or the
/// anchor) that is not inline.
fn enclosing_block(cx: &Context, position: Position) -> Option<NodeId> {
    let mut node = Some(position.node);
    while let Some(id) = node {
        if cx.tree.is_element(id) && is_enclosing_block(cx, id) {
            return Some(id);
        }
        node = cx.tree.parent(id);
    }
    None
}

fn is_enclosing_block(cx: &Context, node: NodeId) -> bool {
    cx.object(node).is_some_and(|o| !o.is_inline())
}

fn is_non_inline_table_image_or_hr(cx: &Context, node: NodeId) -> bool {
    use markup5ever::local_name;
    let block_level = cx.object(node).is_some_and(|o| !o.is_inline());
    block_level
        && cx.tree.node(node).element_data().is_some_and(|e| {
            matches!(
                e.name.local,
                local_name!("table") | local_name!("img") | local_name!("hr")
            )
        })
}

/// Whether a Text node's style keeps line feeds as forced breaks.
fn preserves_breaks(cx: &Context, node: NodeId) -> bool {
    let Some(parent) = cx.tree.parent(node) else {
        return false;
    };
    cx.tree.node(parent).primary_styles().is_some_and(|style| {
        matches!(
            style.clone_white_space_collapse(),
            WhiteSpace::Preserve | WhiteSpace::PreserveBreaks | WhiteSpace::BreakSpaces
        )
    })
}

/// The offset of the last line feed in a Text node, before `limit` when
/// given.
fn last_newline(cx: &Context, node: NodeId, limit: Option<usize>) -> Option<usize> {
    let units = cx.tree.text_units(node);
    let end = limit.map_or(units.len(), |limit| limit.min(units.len()));
    units[..end]
        .iter()
        .rposition(|&unit| unit == u16::from(b'\n'))
}
