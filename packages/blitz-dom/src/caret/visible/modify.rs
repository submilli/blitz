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
use super::shadow;
use super::visible_position;
use crate::caret::{
    Alter, AnchorType, CaretMemory, Direction, Granularity, Modified, ModifyRequest,
};
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
    let mut anchor = Position::from_kept(&cx.tree, anchor, request.memory.anchor_type);
    let mut focus = Position::from_kept(&cx.tree, focus, request.memory.focus_type);
    let mut line_x = request.memory.line_x;
    let line = request.granularity == Granularity::Line;
    if line && request.alter == Alter::Move && anchor == focus {
        // Line steps are logical (see below).
        let forward = is_forward(request.direction, true);
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
    let forward = is_forward(request.direction, ltr);
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
    report(cx, anchor, focus, affinity, vertical.then_some(x))
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

/// The selection as the DOM reports it: visible again with its affinity,
/// as parent-anchored boundary points.
fn report(
    cx: &Context,
    anchor: Position,
    focus: Position,
    affinity: Affinity,
    line_x: Option<f32>,
) -> Option<Modified> {
    let reported = Selection::visible(cx, anchor, focus, affinity).unwrap_or(Selection {
        anchor,
        focus,
        affinity,
    });
    if cx.exhausted() {
        return None;
    }
    let anchor_first = reported.anchor_first(cx);
    let reported_anchor = reported.anchor.to_boundary(&cx.tree)?;
    let reported_focus = reported.focus.to_boundary(&cx.tree)?;
    let (anchor, mut focus) =
        shadow::avoid_crossing(&cx.tree, reported_anchor, reported_focus, anchor_first);
    if focus != reported_focus {
        // The adjusted end is made visible in the DOM tree, where the host
        // it moved beside has only its light children.
        let dom = cx.dom();
        let position = Position::from_boundary(&dom.tree, focus);
        if let Some(visible) = visible_position::create(&dom, Placed::downstream(position)) {
            focus = visible.position.to_boundary(&dom.tree)?;
        }
        if dom.exhausted() {
            return None;
        }
    }
    // An end the adjustment moved is anchored by its offset.
    let anchor_type = |position: Position, reported: Boundary, boundary: Boundary| {
        if boundary == reported {
            position.anchor_type(&cx.tree, boundary)
        } else {
            AnchorType::Offset
        }
    };
    Some(Modified {
        anchor,
        focus,
        memory: CaretMemory {
            line_x,
            upstream: affinity == Affinity::Upstream,
            anchor_type: anchor_type(reported.anchor, reported_anchor, anchor),
            focus_type: anchor_type(reported.focus, reported_focus, focus),
        },
    })
}

fn is_forward(direction: Direction, ltr: bool) -> bool {
    match direction {
        Direction::Forward => true,
        Direction::Backward => false,
        Direction::Right => ltr,
        Direction::Left => !ltr,
    }
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
    Some(Modified {
        anchor: anchor_boundary,
        focus: focus_boundary,
        memory: CaretMemory {
            line_x: Some(line_x),
            upstream: false,
            anchor_type: anchor.anchor_type(&cx.tree, anchor_boundary),
            focus_type: focus.anchor_type(&cx.tree, focus_boundary),
        },
    })
}

/// The direction of the block around a position
/// (`DirectionOfEnclosingBlockOf`).
fn direction_of(cx: &Context, position: Position) -> TextDirection {
    let node = position.container(&cx.tree).unwrap_or(position.node);
    canonical::enclosing_block_flow_element(cx, node)
        .and_then(|block| cx.tree.node(block).primary_styles())
        .map_or(TextDirection::Ltr, |style| style.clone_direction())
}
