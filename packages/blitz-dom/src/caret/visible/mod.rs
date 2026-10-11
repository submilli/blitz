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
mod selection;
mod shadow;
mod text;
mod tree;
mod visible_position;

use crate::BaseDocument;
use crate::ranges::Boundary;

/// Move or extend a document selection by line, paragraph or line
/// boundary; `None` leaves it as it is.
pub(super) fn modify_selection(
    doc: &BaseDocument,
    anchor: Boundary,
    focus: Boundary,
    request: super::ModifyRequest,
) -> Option<super::Modified> {
    modify::modify(&context::Context::new(doc), anchor, focus, request)
}
