//! Caret stops beside floats.
//!
//! A floated or positioned replaced element is a flow of one atom with a
//! stop on each side. Chrome puts no float in a line box: a line step or
//! line boundary from one of its stops moves from the inline content beside
//! the float, and where there is none the stop stays where it is. The
//! builder records which content each stop stands for, since only it sees
//! what precedes and follows the float in tree order.

use super::navigate::{Caret, Flows};

/// The content one of a float's stops stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Beside {
    /// Nothing on its line: another float, a forced break, an embedded box
    /// before it, or the end of the root.
    Stays,
    /// Nothing before it on its line, which it opens after a forced break
    /// or at the root's start. Vertical steps start from the line before.
    LineStart,
    /// The end of the preceding flow, on the same line.
    Previous,
    /// The start of the following flow: the content after the float, or
    /// an embedded box's content.
    Next,
    /// What the stop after the float stands for: the preceding content ends
    /// in white space hanging at a soft wrap.
    AsAfter,
}

/// What the stops before and after a float stand for.
#[derive(Clone, Copy, Debug)]
pub(super) struct FloatStops {
    pub(super) before: Beside,
    pub(super) after: Beside,
}

impl FloatStops {
    /// The content a stop stands for, `after` the float or before it.
    pub(super) fn side(self, after: bool) -> Beside {
        match (after, self.before) {
            (true, _) | (false, Beside::AsAfter) => self.after,
            (false, before) => before,
        }
    }
}

/// The inline caret a float's stop stands for, as Chrome canonicalizes it;
/// `None` when it stays, or for a stop that is not a float's.
pub(super) fn stand_in(flows: &dyn Flows, caret: Caret) -> Option<Caret> {
    let flow = flows.flow(caret.flow)?;
    match flow.float_stops()?.side(caret.index > 0) {
        Beside::Next => {
            let index = caret.flow.checked_add(1)?;
            Some(Caret::at(index, flows.flow(index)?.first_stop()?))
        }
        Beside::Previous => {
            let index = caret.flow.checked_sub(1)?;
            let previous = flows.flow(index).filter(|p| p.root() == flow.root())?;
            Some(Caret {
                flow: index,
                index: previous.len(),
                upstream: true,
            })
        }
        Beside::Stays | Beside::LineStart | Beside::AsAfter => None,
    }
}
