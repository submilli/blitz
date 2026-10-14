//! Caret navigation for `Selection.modify()`.
//! <https://w3c.github.io/selection-api/#dom-selection-modify>
//!
//! The specification leaves movement to the user agent; these rules follow
//! Chrome on macOS, so callers must flush style and layout first. Movement
//! by line, paragraph and line boundary ports Chrome's editing model over
//! DOM positions and line boxes, and bounds its work (the `visible`
//! module). Document character steps walk visually distinct positions;
//! other document flow steps preserve visible selection ends around
//! navigation over inline caret stops. Text controls navigate only their
//! editor's flow. Each
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

impl Direction {
    /// Whether a step goes forward in the DOM. Left and right are visual:
    /// over right-to-left text, as `rtl` tells (asked only for them), they
    /// go backward and forward.
    pub(crate) fn is_forward(self, rtl: impl FnOnce() -> bool) -> bool {
        match self {
            Self::Forward => true,
            Self::Backward => false,
            Self::Right => !rtl(),
            Self::Left => rtl(),
        }
    }
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
    /// Whether the document selection is directional. Chrome's macOS
    /// selection is after extend steps, `extend()`, `setBaseAndExtent()`
    /// and `selectAllChildren()`, and not after move steps, `addRange()`
    /// or a script change of its Range. Extending a range that is not
    /// directional first puts its anchor at the end opposite the side the
    /// step moves toward (`SelectionModifier::PrepareToModifySelection`).
    /// A text control's selection is directional unless its
    /// `selectionDirection` is "none", whatever this says.
    pub directional: bool,
    /// What the previous result kept for this step; any other selection
    /// change discards it ([`CaretMemory::default`]).
    pub memory: CaretMemory,
}

/// A selection after movement.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Modified {
    /// The anchor of the selection Chrome keeps, which the next step starts
    /// from. It can be inside a shadow tree.
    pub anchor: Boundary,
    /// The focus of the selection Chrome keeps.
    pub focus: Boundary,
    /// The range the selection reports for the kept one, still in the kept
    /// ends' tree scope; `None` when the step leaves no selection, as
    /// Chrome's does when neither end has a visible position in the DOM
    /// tree (`anchor` and `focus` are then where the step started).
    pub reported: Option<Reported>,
    /// What to pass to the next step.
    pub memory: CaretMemory,
}

impl Modified {
    /// A selection that reports its kept ends as they are.
    pub(crate) fn reporting_kept(anchor: Boundary, focus: Boundary, memory: CaretMemory) -> Self {
        Self {
            anchor,
            focus,
            reported: Some(Reported { anchor, focus }),
            memory,
        }
    }

    /// No selection left after a step from `given` (anchor, focus).
    pub(crate) fn dropped(given: (Boundary, Boundary)) -> Self {
        Self {
            anchor: given.0,
            focus: given.1,
            reported: None,
            memory: CaretMemory::default(),
        }
    }
}

/// The boundary points a selection reports: its visible range in the DOM
/// tree. A range kept beside a shadow host reports the position Chrome's
/// DOM tree makes of it: an end kept before a block host reports as
/// `(<p>, 0)` for the paragraph after the host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reported {
    /// The reported anchor.
    pub anchor: Boundary,
    /// The reported focus.
    pub focus: Boundary,
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
