//! Line and line-boundary movement as Chrome's editing computes it, ported
//! from Blink (`third_party/blink/renderer/core/editing`): over canonical
//! DOM positions ("visible positions"), the line boxes of inline
//! formatting contexts, and hit testing at a kept horizontal position.
//!
//! Floats and positioned boxes take no text content in their formatting
//! context, so the positions beside one map to where the content around it
//! is, and lines are found by walking the DOM's leaves where a context has
//! none. The rest of caret movement works over [`super::flow`]s.

mod bidi;
mod boundary;
mod canonical;
mod caret;
mod context;
mod hit;
mod ifc;
mod iterator;
mod line;
mod modify;
mod paragraph;
mod position;
mod prepare;
mod select;
mod selection;
mod shadow;
mod text;
mod tree;
mod visible_position;

use super::ModifyRequest;
use crate::BaseDocument;
use crate::ranges::Boundary;

/// Move or extend a document selection by line, paragraph or line
/// boundary; `None` leaves it as it is, and a result reporting no range
/// leaves no selection.
pub(super) fn modify_selection(
    doc: &BaseDocument,
    anchor: Boundary,
    focus: Boundary,
    request: ModifyRequest,
) -> Option<super::Modified> {
    let cx = context::Context::new(doc);
    let (anchor, focus, request) = prepare::prepare(&cx, anchor, focus, request);
    modify::modify(&cx, anchor, focus, request)
}

/// The selection a step over caret stops starts from: a range that is not
/// directional, oriented toward the side an extension moves to (see
/// [`ModifyRequest::directional`]).
pub(super) fn prepare_to_modify(
    doc: &BaseDocument,
    anchor: Boundary,
    focus: Boundary,
    request: ModifyRequest,
) -> (Boundary, Boundary, ModifyRequest) {
    // Most steps need no orienting; they skip building a context.
    if !prepare::applies(anchor, focus, request) {
        return (anchor, focus, request);
    }
    prepare::prepare(&context::Context::new(doc), anchor, focus, request)
}
