//! `Selection.modify()` by line, paragraph and line boundary, as Blink's
//! `SelectionModifier` does it with Chrome's macOS editing behaviour.

use super::boundary;
use super::canonical;
use super::caret::{self, Affinity, Placed};
use super::context::Context;
use super::line;
use super::paragraph;
use super::position::Position;
use super::select::{self, SelectStep};
use super::selection::Selection;
use super::visible_position;
use crate::caret::{Alter, CaretMemory, Direction, Granularity, Modified, ModifyRequest, Reported};
use crate::ranges::Boundary;
use style::computed_values::direction::T as TextDirection;

/// Move or extend; `None` leaves the selection as it is.
pub(super) fn modify(
    cx: &Context,
    anchor: Boundary,
    focus: Boundary,
    request: ModifyRequest,
) -> Option<Modified> {
    let affinity = if request.memory.upstream {
        Affinity::Upstream
    } else {
        Affinity::Downstream
    };
    let given = (anchor, focus);
    let mut anchor = Position::from_kept(&cx.tree, anchor, request.memory.anchor_type);
    let mut focus = Position::from_kept(&cx.tree, focus, request.memory.focus_type);
    let mut line_x = request.memory.line_x;
    let line = request.granularity == Granularity::Line;
    if line && request.alter == Alter::Move && anchor == focus {
        // Line steps are logical (see below).
        let forward = request.direction.is_forward(|| false);
        match select_line_step(cx, focus, forward, line_x) {
            Some(InSelect::Reported(modified)) => return modified,
            Some(InSelect::Left { from, x }) => (anchor, focus, line_x) = (from, from, Some(x)),
            None => {}
        }
    }
    let selection = Selection::visible(cx, anchor, focus, affinity)?;
    let vertical = matches!(
        request.granularity,
        Granularity::Line | Granularity::Paragraph
    );
    // Without `BidiVisualOrderCaretMovement` (off in Chrome 154), right and
    // left by line or paragraph are logical; a line boundary follows the
    // block's direction.
    let ltr = vertical || direction_of(cx, selection.focus) == TextDirection::Ltr;
    let forward = request.direction.is_forward(|| !ltr);
    let x = line_x.unwrap_or_else(|| {
        let focus = selection.visible_end(cx, selection.focus);
        caret::line_direction_point(cx, focus)
    });
    let start = selection.visible_end(cx, selection.start(cx));
    let end = selection.visible_end(cx, selection.end(cx));
    let target = if vertical {
        let from = match request.alter {
            Alter::Move if forward => end,
            Alter::Move => start,
            Alter::Extend => selection.visible_focus(cx),
        };
        if line && forward {
            let extend = request.alter == Alter::Extend;
            if let Some(position) = select::step_into(cx, from.position, extend) {
                let anchor = if extend { selection.anchor } else { position };
                return report_as_is(cx, anchor, position, x);
            }
        }
        vertical_step(cx, request.granularity, from, x, forward)
    } else {
        // A move left or right starts from the selection's start.
        let sideways = matches!(request.direction, Direction::Left | Direction::Right);
        let from = if forward && !(sideways && request.alter == Alter::Move) {
            end
        } else {
            start
        };
        if forward {
            boundary::logical_end_of_line(cx, from)
        } else {
            boundary::logical_start_of_line(cx, from)
        }
    };
    let position = visible_position::create(cx, target?)?;
    if cx.exhausted() {
        return None;
    }
    let (anchor, focus, affinity) = match request.alter {
        Alter::Move => (position.position, position.position, position.affinity),
        Alter::Extend => {
            let (anchor, focus) = extend(cx, &selection, position.position, request, forward);
            (anchor, focus, Affinity::Downstream)
        }
    };
    report(cx, anchor, focus, affinity, vertical.then_some(x), given)
}

fn vertical_step(
    cx: &Context,
    granularity: Granularity,
    from: Placed,
    x: f32,
    down: bool,
) -> Option<Placed> {
    match (granularity, down) {
        (Granularity::Paragraph, true) => paragraph::next_paragraph_position(cx, from, x),
        (Granularity::Paragraph, false) => paragraph::previous_paragraph_position(cx, from, x),
        (_, true) => line::next_line_position(cx, from, x),
        (_, false) => line::previous_line_position(cx, from, x),
    }
}

/// The selection after extending to `position`: a line or paragraph step
/// never carries the focus across the anchor, and a boundary step grows a
/// range at the side it moves toward.
fn extend(
    cx: &Context,
    selection: &Selection,
    position: Position,
    request: ModifyRequest,
    forward: bool,
) -> (Position, Position) {
    let mut position = position;
    let keeps_anchor_side = matches!(
        request.granularity,
        Granularity::Line | Granularity::Paragraph
    );
    if !selection.is_caret() && keeps_anchor_side {
        let extended = Selection {
            focus: position,
            ..*selection
        };
        if selection.anchor_first(cx) != extended.anchor_first(cx) {
            position = selection.anchor;
        }
    }
    if selection.is_caret() || request.granularity != Granularity::LineBoundary {
        return (selection.anchor, position);
    }
    match (forward, selection.anchor_first(cx)) {
        (true, true) | (false, false) => (selection.anchor, position),
        (true, false) | (false, true) => (position, selection.focus),
    }
}

/// The selection a step leaves. Chrome keeps the step's flat-tree result
/// as a visible selection in the DOM tree (`SelectionModifier::Selection`),
/// which keeps a range inside its anchor's tree scope, and the next step
/// starts from that. The selection reports the visible range of what it
/// keeps (`FirstEphemeralRangeOf` of `ComputeVisibleSelectionInDomTree`):
/// making the kept selection visible again canonicalizes an end the scope
/// adjustment left beside a host, such as before a block host, which then
/// reports as the start of the paragraph after it. A selection with no
/// visible position in the DOM tree is dropped; `given` is the selection
/// the step started from.
fn report(
    cx: &Context,
    anchor: Position,
    focus: Position,
    affinity: Affinity,
    line_x: Option<f32>,
    given: (Boundary, Boundary),
) -> Option<Modified> {
    let dom = cx.dom();
    let (anchor, focus) = (anchor.in_dom_tree(&cx.tree), focus.in_dom_tree(&cx.tree));
    let kept = Selection::visible(&dom, anchor, focus, affinity);
    let reported =
        kept.and_then(|kept| Selection::visible(&dom, kept.anchor, kept.focus, kept.affinity));
    if dom.exhausted() {
        return None;
    }
    let (Some(kept), Some(reported)) = (kept, reported) else {
        return Some(Modified::dropped(given));
    };
    let tree = &dom.tree;
    let (anchor, focus) = (
        kept.anchor.to_boundary(tree)?,
        kept.focus.to_boundary(tree)?,
    );
    Some(Modified {
        anchor,
        focus,
        reported: Some(Reported {
            anchor: reported.anchor.to_boundary(tree)?,
            focus: reported.focus.to_boundary(tree)?,
        }),
        memory: CaretMemory {
            line_x,
            upstream: kept.affinity == Affinity::Upstream,
            anchor_type: kept.anchor.anchor_type(tree, anchor),
            focus_type: kept.focus.anchor_type(tree, focus),
        },
    })
}

/// What a line step from a caret in a select does.
enum InSelect {
    /// It moves within the select, or stays: the selection to report.
    Reported(Option<Modified>),
    /// It leaves a list box: the step goes on from `from`, keeping `x`.
    Left { from: Position, x: f32 },
}

fn select_line_step(
    cx: &Context,
    caret: Position,
    forward: bool,
    line_x: Option<f32>,
) -> Option<InSelect> {
    let step = select::step_inside(cx, caret, forward)?;
    let x = line_x.unwrap_or_else(|| select::line_x(cx, caret));
    Some(match step {
        SelectStep::To(position) => InSelect::Reported(report_as_is(cx, position, position, x)),
        SelectStep::Leave(select) => InSelect::Left {
            from: Position::after(select),
            x,
        },
    })
}

/// The selection as given, without making it visible again: a step at a
/// select reports positions in its content, which have no visible
/// position of their own.
fn report_as_is(cx: &Context, anchor: Position, focus: Position, line_x: f32) -> Option<Modified> {
    if cx.exhausted() {
        return None;
    }
    let anchor_boundary = anchor.to_boundary(&cx.tree)?;
    let focus_boundary = focus.to_boundary(&cx.tree)?;
    let memory = CaretMemory {
        line_x: Some(line_x),
        upstream: false,
        anchor_type: anchor.anchor_type(&cx.tree, anchor_boundary),
        focus_type: focus.anchor_type(&cx.tree, focus_boundary),
    };
    Some(Modified::reporting_kept(
        anchor_boundary,
        focus_boundary,
        memory,
    ))
}

/// The direction of the block around a position
/// (`DirectionOfEnclosingBlockOf`).
pub(super) fn direction_of(cx: &Context, position: Position) -> TextDirection {
    let node = position.container(&cx.tree).unwrap_or(position.node);
    canonical::enclosing_block_flow_element(cx, node)
        .and_then(|block| cx.tree.node(block).primary_styles())
        .map_or(TextDirection::Ltr, |style| style.clone_direction())
}
