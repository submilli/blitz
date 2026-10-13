//! Orienting a selection that is not directional before extending it
//! (Blink's `SelectionModifier::PrepareToModifySelection`).

use super::caret;
use super::context::Context;
use super::modify::direction_of;
use super::position::Position;
use super::selection::Selection;
use crate::caret::{Alter, CaretMemory, ModifyRequest};
use crate::ranges::Boundary;
use style::computed_values::direction::T as TextDirection;

/// The selection a step starts from. Extending a range that is not
/// directional grows it at the side the step moves toward: the anchor
/// becomes the start for a forward step and the end for a backward one,
/// with each end keeping its anchor type. Any other selection is as
/// given.
pub(super) fn prepare(
    cx: &Context,
    anchor: Boundary,
    focus: Boundary,
    request: ModifyRequest,
) -> (Boundary, Boundary, ModifyRequest) {
    let given = (anchor, focus, request);
    if !applies(anchor, focus, request) {
        return given;
    }
    let memory = request.memory;
    let selection = Selection::visible(
        cx,
        Position::from_kept(&cx.tree, anchor, memory.anchor_type),
        Position::from_kept(&cx.tree, focus, memory.focus_type),
        caret::Affinity::Downstream,
    );
    let Some(selection) = selection.filter(|selection| !selection.is_caret()) else {
        return given;
    };
    let anchor_start = request
        .direction
        .is_forward(|| direction_of_selection(cx, &selection) == TextDirection::Rtl);
    if cx.exhausted() || selection.anchor_first(cx) == anchor_start {
        return given;
    }
    let memory = CaretMemory {
        upstream: false,
        anchor_type: memory.focus_type,
        focus_type: memory.anchor_type,
        ..memory
    };
    (focus, anchor, ModifyRequest { memory, ..request })
}

/// Whether a step may orient the selection: an extension of distinct
/// boundary points that are not directional.
pub(super) fn applies(anchor: Boundary, focus: Boundary, request: ModifyRequest) -> bool {
    request.alter == Alter::Extend && !request.directional && anchor != focus
}

/// `DirectionOfSelection`: the resolved direction of the text both ends
/// draw in when they agree, else the direction of the focus's block. A
/// left-to-right word in a right-to-left paragraph is left to right.
fn direction_of_selection(cx: &Context, selection: &Selection) -> TextDirection {
    let at = |end: Position| {
        caret::caret_of(cx, selection.visible_end(cx, end)).map(|caret| caret.rtl())
    };
    match (at(selection.start(cx)), at(selection.end(cx))) {
        (Some(false), Some(false)) => TextDirection::Ltr,
        (Some(true), Some(true)) => TextDirection::Rtl,
        _ => direction_of(cx, selection.focus),
    }
}
