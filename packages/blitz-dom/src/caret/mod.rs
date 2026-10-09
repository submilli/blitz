//! Caret navigation for `Selection.modify()`.
//! <https://w3c.github.io/selection-api/#dom-selection-modify>
//!
//! The specification leaves movement to the user agent; these rules follow
//! Chrome on macOS. Positions are the caret stops of laid-out inline content
//! (see [`flow`]), so callers must flush style and layout first. Each call
//! rebuilds its view of the document, in time linear in the document's nodes
//! and laid-out text, with no recursion and no retained state.

mod align;
mod build;
mod control;
mod document;
mod float;
mod flow;
mod geometry;
mod navigate;
mod order;
mod segments;

pub use control::ControlModified;

use crate::ranges::Boundary;

/// `modify()`'s `alter` argument: move the caret, or extend the selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alter {
    Move,
    Extend,
}

/// `modify()`'s `direction` argument. `Left` and `Right` follow the
/// paragraph's base direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Forward,
    Backward,
    Left,
    Right,
}

/// `modify()`'s `granularity` argument.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Granularity {
    Character,
    Word,
    Sentence,
    Line,
    Paragraph,
    LineBoundary,
    SentenceBoundary,
    ParagraphBoundary,
    DocumentBoundary,
}

/// One `modify()` call, after argument conversion.
#[derive(Clone, Copy, Debug)]
pub struct ModifyRequest {
    pub alter: Alter,
    pub direction: Direction,
    pub granularity: Granularity,
    /// The horizontal position kept by consecutive line or paragraph
    /// movements, from the previous result. Any other selection change
    /// discards it.
    pub line_x: Option<f32>,
    /// The previous result's focus affinity, kept the same way: a focus at a
    /// soft wrap draws at the end of the earlier line.
    pub upstream: bool,
}

/// A selection after movement.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Modified {
    /// The canonical DOM position of the anchor's caret stop.
    pub anchor: Boundary,
    /// The canonical DOM position of the focus's caret stop.
    pub focus: Boundary,
    /// The horizontal position to pass to the next line or paragraph step.
    pub line_x: Option<f32>,
    /// The focus affinity to pass to the next step.
    pub upstream: bool,
}
