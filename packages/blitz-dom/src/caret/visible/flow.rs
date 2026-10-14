//! The visible-position boundary around movement over inline caret stops.

use super::canonical;
use super::caret::{Affinity, Placed};
use super::context::Context;
use super::iterator::PositionIterator;
use super::modify;
use super::paragraph;
use super::position::Position;
use super::prepare;
use super::selection::Selection;
use super::visible_position;
use crate::caret::{Alter, Granularity, Modified, ModifyRequest};
use crate::{BaseDocument, ranges::Boundary};

/// Canonicalize the input and output, keeping a fixed end's position kind.
/// Flow stops describe where text renders; they do not replace a selection's
/// anchor (`SelectionModifier::Modify` and `ComputeVisibleSelectionInDomTree`).
pub(in crate::caret) fn modify_selection(
    doc: &BaseDocument,
    anchor: Boundary,
    focus: Boundary,
    request: ModifyRequest,
    step: impl FnOnce(Boundary, Boundary, ModifyRequest) -> Option<Modified>,
) -> Option<Modified> {
    let given = (anchor, focus);
    let cx = Context::new(doc);
    let (anchor, focus, request) = prepare::prepare(&cx, anchor, focus, request);
    let affinity = if request.memory.upstream {
        Affinity::Upstream
    } else {
        Affinity::Downstream
    };
    let selection = Selection::visible(
        &cx,
        Position::from_kept(&cx.tree, anchor, request.memory.anchor_type),
        Position::from_kept(&cx.tree, focus, request.memory.focus_type),
        affinity,
    )?;
    let (a, f) = (
        selection.anchor.to_boundary(&cx.tree)?,
        selection.focus.to_boundary(&cx.tree)?,
    );
    // An opaque block has visible positions but no inline caret stops.
    // Start there, with its kept anchor kind, rather than clamping to a
    // neighbouring flow and taking an extra step inside it.
    let forward = request.direction.is_forward(|| {
        modify::direction_of(&cx, selection.focus) == style::computed_values::direction::T::Rtl
    });
    if request.granularity == Granularity::Character
        && request.alter == Alter::Move
        && !selection.is_caret()
    {
        let target = if forward {
            selection.end(&cx)
        } else {
            selection.start(&cx)
        };
        let target = selection.visible_end(&cx, target);
        return modify::report(
            &cx,
            target.position,
            target.position,
            target.affinity,
            None,
            given,
        );
    }
    let from = if matches!(
        request.granularity,
        Granularity::SentenceBoundary
            | Granularity::ParagraphBoundary
            | Granularity::DocumentBoundary
    ) {
        selection
            .visible_end(
                &cx,
                if forward {
                    selection.end(&cx)
                } else {
                    selection.start(&cx)
                },
            )
            .position
    } else {
        selection.visible_focus(&cx).position
    };
    let origin = if request.granularity == Granularity::Character
        || (!cx.tree.is_text(from.node) && cx.ifc_of(from.node).is_none())
    {
        outside_flow(&cx, from, request)
    } else {
        Origin::Flow
    };
    let moved = match origin {
        Origin::Target(target) => {
            let (anchor, focus) = match request.alter {
                Alter::Move => (target, target),
                Alter::Extend => modify::extend(&cx, &selection, target, request, forward),
            };
            return modify::report(&cx, anchor, focus, Affinity::Downstream, None, given);
        }
        Origin::None => return None,
        Origin::Text(edge) => {
            let edge = edge.to_boundary(&cx.tree)?;
            let moved = step(
                edge,
                edge,
                ModifyRequest {
                    alter: Alter::Move,
                    ..request
                },
            )?;
            let target = Position::from_boundary(&cx.tree, moved.focus);
            let (anchor, focus) = match request.alter {
                Alter::Move => (target, target),
                Alter::Extend => modify::extend(&cx, &selection, target, request, forward),
            };
            return modify::report(&cx, anchor, focus, Affinity::Downstream, None, given);
        }
        Origin::Flow => step(a, f, request)?,
    };
    let position = |boundary| {
        if boundary == a {
            selection.anchor
        } else if boundary == f {
            selection.focus
        } else {
            Position::from_boundary(&cx.tree, boundary)
        }
    };
    let affinity = if moved.memory.upstream {
        Affinity::Upstream
    } else {
        Affinity::Downstream
    };
    modify::report(
        &cx,
        position(moved.anchor),
        position(moved.focus),
        affinity,
        moved.memory.line_x,
        (anchor, focus),
    )
}

/// Steps from a visible position outside inline content. Word movement first
/// reaches the adjacent text edge; sentence movement backward reaches its
/// sentence start. Boundaries use that text, or the opaque block's own ends.
enum Origin {
    Flow,
    Target(Position),
    Text(Position),
    None,
}

fn outside_flow(cx: &Context, from: Position, request: ModifyRequest) -> Origin {
    let forward = request
        .direction
        .is_forward(|| modify::direction_of(cx, from) == style::computed_values::direction::T::Rtl);
    match request.granularity {
        Granularity::Character => {
            let placed = distinct_candidate(cx, from, forward)
                .and_then(|target| visible_position::create(cx, Placed::downstream(target)));
            match placed {
                Some(placed) if forward || placed.position != from => {
                    Origin::Target(placed.position)
                }
                _ => Origin::None,
            }
        }
        Granularity::Word | Granularity::Sentence | Granularity::SentenceBoundary => {
            let Some((edge, separator)) = adjacent_text(cx, from, forward) else {
                return Origin::None;
            };
            if separator
                && (request.granularity == Granularity::Word
                    || (forward && request.granularity == Granularity::Sentence))
            {
                Origin::Target(edge)
            } else {
                Origin::Text(edge)
            }
        }
        Granularity::ParagraphBoundary if cx.tree.editing_ignores_content(from.node) => {
            if forward {
                Origin::Target(Position::after(from.node))
            } else {
                paragraph::start_of_paragraph(cx, from).map_or(Origin::None, Origin::Target)
            }
        }
        _ => Origin::Flow,
    }
}

/// Blink's Next/PreviousVisuallyDistinctCandidate: skip positions rendering
/// with either side of the starting position, but retain its iterator kind.
fn distinct_candidate(cx: &Context, from: Position, forward: bool) -> Option<Position> {
    let upstream = canonical::most_backward(cx, from);
    let downstream = canonical::most_forward(cx, from);
    scan(cx, from, forward, |candidate| {
        canonical::is_candidate(cx, candidate)
            && canonical::most_backward(cx, candidate) != upstream
            && canonical::most_forward(cx, candidate) != downstream
    })
}

fn adjacent_text(cx: &Context, from: Position, forward: bool) -> Option<(Position, bool)> {
    let mut separator = false;
    let edge = scan(cx, from, forward, |candidate| {
        if cx.tree.editing_ignores_content(candidate.node)
            && cx
                .object(candidate.node)
                .is_some_and(|object| object.block_container)
        {
            separator = true;
        }
        cx.tree.is_text(candidate.node) && canonical::is_candidate(cx, candidate)
    })?;
    Some((edge, separator))
}

/// The context's shared budget bounds every iterator and canonical walk.
fn scan(
    cx: &Context,
    from: Position,
    forward: bool,
    mut accepts: impl FnMut(Position) -> bool,
) -> Option<Position> {
    let mut iterator = PositionIterator::new(cx, from);
    loop {
        if forward {
            iterator.increment(cx);
        } else {
            iterator.decrement(cx);
        }
        if cx.exhausted() || (forward && iterator.at_end(cx)) || (!forward && iterator.at_start(cx))
        {
            return None;
        }
        let candidate = iterator.position(cx)?;
        if accepts(candidate) {
            return Some(candidate);
        }
    }
}
