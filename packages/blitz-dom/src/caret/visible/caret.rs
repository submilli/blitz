//! Where a position's caret draws: the line item it resolves to in its
//! inline formatting context (Blink's `inline_caret_position.cc` and
//! `inline_box_position.cc`), and its horizontal position (`caret_rect.cc`).

use super::bidi;
use super::canonical::{self, has_rendered_descendants_with_height};
use super::context::{Context, Kind};
use super::ifc::{Ifc, ItemKind};
use super::position::{Anchor, Position};
use crate::NodeId;
use crate::caret::geometry::{content_origin, content_width};
use std::rc::Rc;

/// The caret's width, by which a caret after a box draws inside it.
const CARET_WIDTH: f32 = 1.0;

/// `TextAffinity`: at a soft wrap, upstream draws at the end of the
/// earlier line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Affinity {
    Downstream,
    Upstream,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Placed {
    pub(super) position: Position,
    pub(super) affinity: Affinity,
}

impl Placed {
    pub(super) fn downstream(position: Position) -> Self {
        Self {
            position,
            affinity: Affinity::Downstream,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Side {
    AtText(usize),
    BeforeBox,
    AfterBox,
}

/// A caret resolved to one item of a formatting context.
#[derive(Clone)]
pub(super) struct Caret {
    pub(super) ifc: Rc<Ifc>,
    pub(super) item: usize,
    pub(super) side: Side,
}

impl Caret {
    pub(super) fn line(&self) -> usize {
        self.ifc.items[self.item].line
    }

    /// Whether the item it draws in is laid out right to left.
    pub(super) fn rtl(&self) -> bool {
        self.ifc.items[self.item].rtl
    }

    /// `ToPositionInDOMTreeWithAffinity`.
    pub(super) fn to_position(&self, cx: &Context) -> Option<Placed> {
        let item = &self.ifc.items[self.item];
        match (self.side, item.kind) {
            (Side::BeforeBox, ItemKind::Atomic(node)) => {
                Some(Placed::downstream(Position::before(node)))
            }
            (Side::AfterBox, ItemKind::Atomic(node)) => Some(Placed {
                position: Position::after(node),
                affinity: Affinity::Upstream,
            }),
            (Side::AtText(offset), _) => {
                let upstream = offset == item.content.end;
                let found = if upstream {
                    self.ifc.first_position(offset)
                } else {
                    self.ifc.last_position(offset)
                };
                let (node, offset) = found?;
                let position = if cx.tree.is_text(node) {
                    Position::offset(node, offset)
                } else if offset > 0 {
                    Position::after(node)
                } else {
                    Position::before(node)
                };
                Some(Placed {
                    position,
                    affinity: if upstream {
                        Affinity::Upstream
                    } else {
                        Affinity::Downstream
                    },
                })
            }
            _ => None,
        }
    }

    /// The caret's horizontal position in the document.
    pub(super) fn x(&self) -> f32 {
        let item = &self.ifc.items[self.item];
        let width = CARET_WIDTH;
        match self.side {
            Side::BeforeBox if item.rtl => item.right - width,
            Side::BeforeBox => item.left,
            Side::AfterBox if item.rtl => item.left,
            Side::AfterBox => item.right - width,
            Side::AtText(offset) => {
                let boundary = item
                    .carets
                    .get(offset.saturating_sub(item.content.start))
                    .copied()
                    .unwrap_or(item.left);
                // A bar caret is centred on the boundary, except at a break.
                let line_break = item.is_line_break();
                let x = if line_break {
                    boundary
                } else {
                    boundary - width / 2.0
                };
                clamp_and_round(&self.ifc.lines[item.line], x, width)
            }
        }
    }
}

/// Keep a caret inside its line box (and the block, past the line's end
/// edge), on a pixel of the block (`ClampAndRound` in `caret_rect.cc`).
fn clamp_and_round(line: &super::ifc::Line, x: f32, width: f32) -> f32 {
    let origin = line.container_left;
    let (low, high) = if line.align_right {
        (line.left.min(origin), line.right - width)
    } else {
        (line.left, line.right.max(line.container_right) - width)
    };
    let (low, high) = ((low - origin).ceil(), (high - origin).floor());
    // Page geometry that overflows to infinity gives NaN bounds, which
    // `clamp` rejects with a panic.
    if low.is_nan() || high.is_nan() || low >= high {
        let edge = origin + high;
        return if edge.is_finite() { edge } else { x };
    }
    origin + (x - origin).clamp(low, high).round()
}

/// `ComputeInlineAdjustedPosition`: a position in a Text node, or beside an
/// inline box; a position in a block moves into its inline content.
pub(super) fn inline_adjusted(cx: &Context, placed: Placed) -> Option<Placed> {
    adjust(cx, placed, 0)
}

/// Blink bounds the block adjustment's recursion.
const MAX_ADJUSTMENT_DEPTH: usize = 256;

fn adjust(cx: &Context, placed: Placed, depth: usize) -> Option<Placed> {
    let position = placed.position;
    let object = cx.object(position.node)?;
    if object.kind == Kind::Text {
        return Some(placed);
    }
    if position.is_before_anchor() || position.is_after_anchor() {
        if cx.ifc_of(position.node).is_some() {
            return object.is_inline().then_some(placed);
        }
        if object.is_atomic() {
            return Some(placed);
        }
    }
    let editable_children =
        cx.tree.is_element(position.node) && !cx.tree.editing_ignores_content(position.node);
    if object.is_block_flow()
        && editable_children
        && has_rendered_descendants_with_height(cx, position.node)
    {
        return adjust_block_flow(cx, position, depth);
    }
    matches!(object.kind, Kind::Atomic { .. } | Kind::Br).then_some(placed)
}

fn adjust_block_flow(cx: &Context, position: Position, depth: usize) -> Option<Placed> {
    if depth >= MAX_ADJUSTMENT_DEPTH {
        return None;
    }
    let upstream = |position| Placed {
        position,
        affinity: Affinity::Upstream,
    };
    let downstream = repeat(cx, position, canonical::most_forward);
    if downstream != position && cx.has_layout(downstream.node) {
        return adjust(cx, upstream(downstream), depth + 1);
    }
    let backward = repeat(cx, position, canonical::most_backward);
    if backward == position || !cx.has_layout(backward.node) {
        return None;
    }
    adjust(cx, upstream(backward), depth + 1)
}

/// Apply a canonicalization step until it settles.
fn repeat(cx: &Context, position: Position, step: fn(&Context, Position) -> Position) -> Position {
    let mut current = position;
    // Each step moves monotonically through the tree, so it settles; the
    // bound only guards against a cycle.
    for _ in 0..MAX_ADJUSTMENT_DEPTH {
        let next = step(cx, current);
        if next.equivalent(current, &cx.tree) {
            return next;
        }
        current = next;
    }
    current
}

/// `ComputeInlineCaretPosition`: the item a position's caret resolves to.
pub(super) fn inline_caret(cx: &Context, placed: Placed) -> Option<Caret> {
    let position = placed.position;
    let object = cx.object(position.node)?;
    let ifc = cx.ifc_of(position.node)?;
    if object.kind == Kind::Inline {
        return match position.anchor {
            Anchor::Before => beside_inline(cx, ifc, position.node, false),
            Anchor::After => beside_inline(cx, ifc, position.node, true),
            _ => None,
        };
    }
    if !accepts(cx, position) {
        return None;
    }
    let (node, offset) = Ifc::node_offset(&cx.tree, position);
    let content = ifc.content_offset(node, offset).unwrap_or(0);
    let layout_text = (position.is_offset_in_anchor() && cx.tree.is_text(node)).then_some(node);
    let zero_width_space = 0x200B;
    let content = if placed.affinity == Affinity::Upstream
        && content > 0
        && ifc.text().get(content - 1) == Some(&zero_width_space)
    {
        content - 1
    } else {
        content
    };
    resolve(&ifc, content, placed.affinity, layout_text)
}

/// Whether the offset mapping takes a position (`AcceptsPosition`): one in
/// a Text node, or beside text or an atomic inline.
pub(super) fn accepts(cx: &Context, position: Position) -> bool {
    if cx.tree.is_text(position.node) {
        return true;
    }
    if !position.is_before_anchor() && !position.is_after_anchor() {
        return false;
    }
    cx.object(position.node)
        .is_some_and(|o| o.is_text() || o.is_atomic())
}

/// The formatting context holding a position (`NGInlineFormattingContextOf`).
pub(super) fn ifc_of_position(cx: &Context, position: Position) -> Option<Rc<Ifc>> {
    if !accepts(cx, position) {
        return None;
    }
    cx.ifc_of(position.node)
}

/// The caret before or after an inline box: at its first (or last) item.
fn beside_inline(cx: &Context, ifc: Rc<Ifc>, span: NodeId, after: bool) -> Option<Caret> {
    let index = culled_inline_edge(cx, &ifc, span, after)?;
    let item = &ifc.items[index];
    let side = match item.kind {
        ItemKind::Text { .. } if after => Side::AtText(item.content.end),
        ItemKind::Text { .. } => Side::AtText(item.content.start),
        ItemKind::Atomic(_) | ItemKind::Float(_) if after => Side::AfterBox,
        ItemKind::Atomic(_) | ItemKind::Float(_) => Side::BeforeBox,
    };
    Some(bidi::adjust_for_resolution(Caret {
        ifc: ifc.clone(),
        item: index,
        side,
    }))
}

/// The first (or last) item that a descendant of an inline box draws
/// (`MoveToIncludingCulledInline`): inline boxes take no items of their
/// own, so this walks the box's descendants.
fn culled_inline_edge(cx: &Context, ifc: &Ifc, span: NodeId, after: bool) -> Option<usize> {
    let last = cx.tree.last_within_or_self(span);
    let mut found: Option<usize> = None;
    let mut node = Some(span);
    while let Some(current) = node {
        if !cx.spend() {
            return None;
        }
        if let Some((first, last)) = ifc.items_of(current) {
            found = Some(match found {
                Some(index) if after => index.max(last),
                Some(index) => index.min(first),
                None if after => last,
                None => first,
            });
        }
        if current == last {
            break;
        }
        node = cx.tree.next_in_preorder(current, false);
    }
    found
}

enum Resolution {
    Failed,
    Candidate(Side),
    Resolved(Side),
}

/// Resolve a content offset to an item, preferring the item of
/// `layout_text` when the position is inside a Text node.
fn resolve(
    ifc: &Rc<Ifc>,
    offset: usize,
    affinity: Affinity,
    layout_text: Option<NodeId>,
) -> Option<Caret> {
    let start = layout_text
        .and_then(|text| ifc.items_of(text))
        .map_or(0, |(first, _)| first);
    let mut candidate: Option<Caret> = None;
    for index in start..ifc.items.len() {
        let caret = |side| Caret {
            ifc: ifc.clone(),
            item: index,
            side,
        };
        match try_resolve(ifc, index, offset, affinity) {
            Resolution::Failed => {}
            Resolution::Resolved(side) => {
                let found = caret(side);
                let own = layout_text.is_none_or(|text| ifc.items[index].kind_node() == Some(text));
                if own {
                    return Some(bidi::adjust_for_resolution(found));
                }
                candidate = Some(found);
            }
            Resolution::Candidate(side) => {
                let found = caret(side);
                candidate = Some(match candidate {
                    Some(current) if !is_upstream_after_line_break(&current) => current,
                    _ => found,
                });
            }
        }
    }
    candidate.map(bidi::adjust_for_resolution)
}

fn try_resolve(ifc: &Ifc, index: usize, offset: usize, affinity: Affinity) -> Resolution {
    let item = &ifc.items[index];
    let (start, end) = (item.content.start, item.content.end);
    match item.kind {
        ItemKind::Text { line_break, .. } => {
            if offset < start || offset > end {
                return Resolution::Failed;
            }
            if offset > start && offset < end {
                return Resolution::Resolved(Side::AtText(offset));
            }
            if offset == start && can_resolve_before(ifc, index, affinity) {
                return Resolution::Resolved(Side::AtText(offset));
            }
            if offset == end && !line_break && can_resolve_after(ifc, index, affinity) {
                return Resolution::Resolved(Side::AtText(offset));
            }
            Resolution::Candidate(Side::AtText(offset))
        }
        ItemKind::Atomic(_) => {
            if offset == start {
                if can_resolve_before(ifc, index, affinity) {
                    return Resolution::Resolved(Side::BeforeBox);
                }
                return Resolution::Candidate(Side::BeforeBox);
            }
            if offset == end {
                if can_resolve_after(ifc, index, affinity) {
                    return Resolution::Resolved(Side::AfterBox);
                }
                return Resolution::Candidate(Side::AfterBox);
            }
            Resolution::Failed
        }
        ItemKind::Float(_) => Resolution::Failed,
    }
}

/// A caret before an item resolves there unless, upstream, the item opens
/// a line after a soft wrap.
fn can_resolve_before(ifc: &Ifc, index: usize, affinity: Affinity) -> bool {
    if affinity == Affinity::Downstream {
        return true;
    }
    let line = ifc.items[index].line;
    if ifc.lines[line].items.start != index {
        return true;
    }
    line.checked_sub(1)
        .is_none_or(|previous| !ifc.lines[previous].soft_wrap)
}

/// A caret after an item resolves there unless, downstream, the item ends
/// a line at a soft wrap.
fn can_resolve_after(ifc: &Ifc, index: usize, affinity: Affinity) -> bool {
    if affinity == Affinity::Upstream {
        return true;
    }
    let line = &ifc.lines[ifc.items[index].line];
    if line.items.end != index + 1 {
        return true;
    }
    !line.soft_wrap
}

fn is_upstream_after_line_break(caret: &Caret) -> bool {
    let item = &caret.ifc.items[caret.item];
    item.is_line_break() && caret.side == Side::AtText(item.content.end)
}

/// The caret of a position, as `LocalCaretRectOfPosition` finds it: through
/// its inline adjustment, or `None` for a block-level position.
pub(super) fn caret_of(cx: &Context, placed: Placed) -> Option<Caret> {
    let adjusted = inline_adjusted(cx, placed)?;
    inline_caret(cx, for_caret_rect(cx, adjusted))
}

/// `AdjustForInlineCaretPosition`.
fn for_caret_rect(cx: &Context, placed: Placed) -> Placed {
    let position = placed.position;
    let with = |position| Placed { position, ..placed };
    match position.anchor {
        Anchor::After | Anchor::Before => placed,
        Anchor::AfterChildren => with(Position::after(position.node)),
        Anchor::Offset(offset) => {
            if cx.tree.is_text(position.node) || offset > 0 {
                return placed;
            }
            match cx.object(position.node) {
                Some(object) if !object.is_block_flow() => with(Position::before(position.node)),
                _ => placed,
            }
        }
    }
}

/// The horizontal position a line step keeps
/// (`LineDirectionPointForBlockDirectionNavigationOf`).
pub(super) fn line_direction_point(cx: &Context, placed: Placed) -> f32 {
    if let Some(caret) = caret_of(cx, placed) {
        return caret.x();
    }
    box_caret_x(cx, placed.position).unwrap_or(0.0)
}

/// A caret drawn by a box rather than by inline content
/// (`LayoutBox::LocalCaretRect`): at its content start, or, past offset 0,
/// at its content end. A box whose content editing ignores draws at its
/// border edges.
fn box_caret_x(cx: &Context, position: Position) -> Option<f32> {
    let node = position.node;
    cx.object(node)?;
    let size = cx.size(node)?;
    let data = cx.tree.node(node);
    let mut left = data.unrounded_absolute_position(0.0, 0.0).x;
    let mut width = size.width;
    if !cx.tree.editing_ignores_content(node) && !cx.tree.is_table(node) {
        left = content_origin(data).0;
        width = content_width(data);
    }
    if position.editing_offset(&cx.tree) > 0 {
        left += width - CARET_WIDTH;
    }
    Some(left)
}
