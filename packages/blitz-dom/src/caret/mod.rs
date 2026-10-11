//! Caret navigation for `Selection.modify()`.
//! <https://w3c.github.io/selection-api/#dom-selection-modify>
//!
//! The specification leaves movement to the user agent; these rules follow
//! Chrome on macOS, so callers must flush style and layout first. Movement
//! by line, paragraph and line boundary ports Chrome's editing model over
//! DOM positions and line boxes, and bounds its work (the `visible`
//! module). Other movement, and all movement in text controls, runs over
//! the caret stops of laid-out inline content (the `flow` module). Each
//! call rebuilds its view of the document and keeps no state.

mod align;
mod build;
mod content;
mod control;
mod document;
mod flow;
mod geometry;
mod navigate;
mod order;
mod segments;
mod visible;

pub use control::ControlModified;

use crate::NodeId;
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
    /// What the previous result kept for this step; any other selection
    /// change discards it ([`CaretMemory::default`]).
    pub memory: CaretMemory,
}

/// A selection after movement.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Modified {
    /// The canonical DOM position of the anchor's caret stop.
    pub anchor: Boundary,
    /// The canonical DOM position of the focus's caret stop.
    pub focus: Boundary,
    /// What to pass to the next step.
    pub memory: CaretMemory,
}

/// What consecutive `modify()` calls keep of a selection beyond its
/// boundary points, as Chrome's selection does.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CaretMemory {
    /// The horizontal position kept by consecutive line or paragraph
    /// movements.
    pub line_x: Option<f32>,
    /// The focus affinity: a focus at a soft wrap draws at the end of the
    /// earlier line.
    pub upstream: bool,
    /// How the anchor is anchored at its boundary point.
    pub anchor_type: AnchorType,
    /// How the focus is anchored at its boundary point.
    pub focus_type: AnchorType,
}

/// How a selection end is anchored, relative to its boundary point. Chrome's
/// selection keeps its positions between `modify()` calls, and a position
/// after a node can step differently from the same boundary point as an
/// offset: from after an `<hr>`, a line step up passes the `<hr>`, while
/// from the offset after it, it stays. The DOM reports only boundary
/// points, so a step returns the types for the next step to pass back. A
/// type naming a child that no longer sits beside the boundary point falls
/// back to the offset, as Chrome turns a position at a removed node into one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AnchorType {
    /// An offset in the boundary's node.
    #[default]
    Offset,
    /// Before this child, at the boundary's offset.
    Before(NodeId),
    /// After this child, before the boundary's offset.
    After(NodeId),
    /// After the last child of the boundary's node.
    AfterChildren,
}
