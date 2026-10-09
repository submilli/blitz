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
    /// Nothing on its line: another float or an embedded box beside it.
    Stays,
    /// Nothing before it on its line, which it opens after a forced break
    /// or at the root's start. It stays at line boundaries, and vertical
    /// steps start above its line.
    LineStart,
    /// The end of the preceding flow of the float's root.
    Previous,
    /// The start of the following flow of the float's root.
    Next,
    /// The start of the content of the box on `line` that follows the
    /// float and ends at `end`, or else of the root's content after it.
    Into { end: Key, line: u32 },
    /// Nothing after it in its root. It stays at line boundaries, and
    /// vertical steps start below its line.
    LineEnd,
    /// What the stop after the float stands for: the preceding content
    /// ends in collapsible white space hanging at a soft wrap.
    SameAsAfter,
}

/// What the stops before and after a float stand for.
#[derive(Clone, Copy, Debug)]
pub(super) struct FloatStops {
    pub(super) before: Beside,
    pub(super) after: Beside,
    /// A forced break precedes the float. After a generated one, the float
    /// sits on the line after the content its before stop stands for; a
    /// DOM break opens the line anyway.
    pub(super) after_break: bool,
    /// The float, or the run of floats it ends, directly follows the
    /// content of the preceding flow on its line.
    pub(super) follows_content: bool,
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
    /// A box on `line` whose content forms flows of its own, ending at
    /// `end`.
    Box { end: Option<Key>, line: u32 },
    /// A float, by the index of its flow among the root's flows.
    Float(usize),
}

/// What the stop before a float stands for. `line_start` tells whether the
/// float opens its line; `previous` is the flow it closed, when that held
/// content, and `collapsible_end` whether the flow ends in collapsible
/// white space.
pub(super) fn before_stop(
    line_start: bool,
    last: Walked,
    previous: Option<&Flow>,
    collapsible_end: bool,
) -> Beside {
    if line_start || previous.is_some_and(Flow::ends_with_break) {
        return Beside::LineStart;
    }
    // A forced break that leaves the float off a line start is generated
    // content's, which Chrome passes over.
    match (last, previous) {
        (Walked::Inline | Walked::Break, Some(previous))
            if collapsible_end && previous.ends_with_hanging_space() =>
        {
            Beside::SameAsAfter
        }
        (Walked::Inline | Walked::Break, Some(_)) => Beside::Previous,
        _ => Beside::Stays,
    }
}

/// What the stop after a float stands for once `next` follows it. A forced
/// break leaves it with the content before the float, or before the run of
/// floats it ends, when they share a line.
pub(super) fn after_stop(next: Walked, stops: FloatStops) -> Beside {
    match next {
        Walked::Inline => Beside::Next,
        Walked::Box {
            end: Some(end),
            line,
        } => Beside::Into { end, line },
        Walked::Break if stops.follows_content && !stops.after_break => Beside::Previous,
        Walked::Break | Walked::Box { end: None, .. } | Walked::Float(_) => Beside::Stays,
    }
}

/// The inline caret a float's stop stands for, as Chrome canonicalizes it;
/// `None` when it stays, or for a stop that is not a float's.
pub(super) fn stand_in(flows: &dyn Flows, caret: Caret) -> Option<Caret> {
    let flow = flows.flow(caret.flow)?;
    // Another float's stop stands for nothing on a line.
    let next = || {
        let index = caret.flow.checked_add(1)?;
        Some((index, flows.flow(index).filter(|next| !next.is_float())?))
    };
    match flow.float_stops()?.side(caret.index > 0) {
        Beside::Next => {
            let (index, next) = next().filter(|(_, next)| next.root() == flow.root())?;
            Some(Caret::at(index, next.first_stop()?))
        }
        Beside::Into { end, .. } => {
            let inside = |next: &Flow| next.first_key().is_some_and(|key| key < end);
            let (index, next) =
                next().filter(|(_, next)| next.root() == flow.root() || inside(next))?;
            Some(Caret::at(index, next.first_stop()?))
        }
        Beside::Previous => {
            // Back over the run of floats the stop's float ends.
            let (index, previous) = (0..caret.flow)
                .rev()
                .map_while(|index| Some((index, flows.flow(index)?)))
                .find(|(_, previous)| !previous.is_float())?;
            (previous.root() == flow.root()).then_some(Caret {
                flow: index,
                index: previous.len(),
                upstream: true,
            })
        }
        Beside::Stays | Beside::LineStart | Beside::LineEnd | Beside::SameAsAfter => None,
    }
}
