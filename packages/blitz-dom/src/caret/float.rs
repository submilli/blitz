//! Caret stops beside floats.
//!
//! A floated or positioned replaced element is a flow of one atom with a
//! stop on each side. Chrome puts no float in a line box: a line step or
//! line boundary from one of its stops moves from the inline content beside
//! the float, and where there is none the stop stays where it is. The flow
//! builder walks content in tree order, so it decides here what each stop
//! stands for as it passes the float and the content after it.

use super::flow::Flow;
use super::navigate::{Caret, Flows};
use super::order::Key;

/// The content one of a float's stops stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Beside {
    /// Nothing on its line: another float or an embedded box before it,
    /// or nothing after it.
    Stays,
    /// Nothing before it on its line, which it opens after a forced break
    /// or at the root's start. It stays at line boundaries, and vertical
    /// steps start above its line.
    LineStart,
    /// The end of the preceding flow of the float's root.
    Previous,
    /// The start of the following flow of the float's root.
    Next,
    /// The start of the content of the box that follows the float, which
    /// ends at this key, or else of the root's content after the box.
    Into(Key),
    /// What the stop after the float stands for: the preceding content
    /// ends in white space hanging at a soft wrap.
    SameAsAfter,
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
            (true, _) | (false, Beside::SameAsAfter) => self.after,
            (false, before) => before,
        }
    }
}

/// Content the flow builder walks past.
#[derive(Clone, Copy, Debug)]
pub(super) enum Walked {
    /// Laid-out text, rendered or not, or an atom.
    Inline,
    /// A forced break.
    Break,
    /// A box whose content forms flows of its own, ending at this key.
    Box(Option<Key>),
    /// A float, by the index of its flow among the root's flows.
    Float(usize),
}

/// What the stop before a float stands for. `previous` is the flow that
/// the float closed, when it held content; `line_start` tells whether any
/// content precedes the float on its line.
pub(super) fn before_stop(line_start: bool, last: Walked, previous: Option<&Flow>) -> Beside {
    if line_start {
        return Beside::LineStart;
    }
    match (last, previous) {
        (Walked::Inline, Some(previous)) if previous.ends_with_hanging_space() => {
            Beside::SameAsAfter
        }
        (Walked::Inline, Some(_)) => Beside::Previous,
        _ => Beside::Stays,
    }
}

/// What the stop after a float stands for once `next` follows it. A forced
/// break leaves it with the content before the float.
pub(super) fn after_stop(next: Walked, before: Beside) -> Beside {
    match next {
        Walked::Inline => Beside::Next,
        Walked::Box(Some(end)) => Beside::Into(end),
        Walked::Break if before == Beside::Previous => Beside::Previous,
        Walked::Break | Walked::Box(None) | Walked::Float(_) => Beside::Stays,
    }
}

/// The inline caret a float's stop stands for, as Chrome canonicalizes it;
/// `None` when it stays, or for a stop that is not a float's.
pub(super) fn stand_in(flows: &dyn Flows, caret: Caret) -> Option<Caret> {
    let flow = flows.flow(caret.flow)?;
    let next = || {
        let index = caret.flow.checked_add(1)?;
        Some((index, flows.flow(index)?))
    };
    match flow.float_stops()?.side(caret.index > 0) {
        Beside::Next => {
            let (index, next) = next().filter(|(_, next)| next.root() == flow.root())?;
            Some(Caret::at(index, next.first_stop()?))
        }
        Beside::Into(end) => {
            let inside = |next: &Flow| next.first_key().is_some_and(|key| key < end);
            let (index, next) =
                next().filter(|(_, next)| next.root() == flow.root() || inside(next))?;
            Some(Caret::at(index, next.first_stop()?))
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
        Beside::Stays | Beside::LineStart | Beside::SameAsAfter => None,
    }
}
