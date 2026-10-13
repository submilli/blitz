//! Visible selections (Blink's `CreateVisibleSelection`): both ends as
//! visible positions (or the one end that has one), kept in one tree scope
//! in the DOM tree, and a range narrowed to the smallest equivalent one.

use super::canonical::{self, most_backward, most_forward};
use super::caret::{Affinity, Placed};
use super::context::Context;
use super::position::Position;
use super::shadow;
use super::visible_position;
use std::cmp::Ordering;

#[derive(Clone, Copy, Debug)]
pub(super) struct Selection {
    pub(super) anchor: Position,
    pub(super) focus: Position,
    pub(super) affinity: Affinity,
}

impl Selection {
    /// The visible selection of a DOM selection. An end without a visible
    /// position collapses the selection to the other end
    /// (`CanonicalizeSelection`); `None` when neither has one. In the DOM
    /// tree a range is also kept inside its anchor's tree scope.
    pub(super) fn visible(
        cx: &Context,
        anchor: Position,
        focus: Position,
        affinity: Affinity,
    ) -> Option<Self> {
        let deep = |position| {
            visible_position::create(cx, Placed { position, affinity }).map(|v| v.position)
        };
        let (anchor, mut focus) = match (deep(anchor), deep(focus)) {
            (Some(anchor), Some(focus)) => (anchor, focus),
            (Some(end), None) | (None, Some(end)) => (end, end),
            (None, None) => return None,
        };
        if cx.tree.is_dom() {
            let canonical = Self {
                anchor,
                focus,
                affinity,
            };
            focus = shadow::avoid_crossing(&cx.tree, anchor, focus, canonical.anchor_first(cx));
        }
        Some(minimal(cx, anchor, focus, affinity))
    }

    pub(super) fn is_caret(&self) -> bool {
        self.anchor == self.focus
    }

    pub(super) fn anchor_first(&self, cx: &Context) -> bool {
        canonical::order(cx, self.anchor, self.focus) != Ordering::Greater
    }

    pub(super) fn start(&self, cx: &Context) -> Position {
        if self.anchor_first(cx) {
            self.anchor
        } else {
            self.focus
        }
    }

    pub(super) fn end(&self, cx: &Context) -> Position {
        if self.anchor_first(cx) {
            self.focus
        } else {
            self.anchor
        }
    }

    /// `VisibleStart` and `VisibleEnd`: an end as a visible position.
    pub(super) fn visible_end(&self, cx: &Context, end: Position) -> Placed {
        let placed = Placed {
            position: end,
            affinity: self.affinity,
        };
        visible_position::create(cx, placed).unwrap_or(placed)
    }

    /// `ComputeVisibleFocus`: at a soft wrap a range's focus draws on the
    /// line its direction leads to.
    pub(super) fn visible_focus(&self, cx: &Context) -> Placed {
        let focus = self.focus;
        let mut affinity = self.affinity;
        if !self.is_caret() && cx.has_layout(focus.node) {
            let downstream = Placed::downstream(focus);
            let upstream = Placed {
                position: focus,
                affinity: Affinity::Upstream,
            };
            if !visible_position::in_same_line(cx, downstream, upstream) {
                affinity = if self.anchor_first(cx) {
                    Affinity::Downstream
                } else {
                    Affinity::Upstream
                };
            }
        }
        let placed = Placed {
            position: focus,
            affinity,
        };
        visible_position::create(cx, placed).unwrap_or(placed)
    }
}

/// `SelectionTypeAdjuster`: a range whose ends render together collapses;
/// otherwise its start moves forward and its end backward over positions
/// rendering at the same place.
fn minimal(cx: &Context, anchor: Position, focus: Position, affinity: Affinity) -> Selection {
    let anchor_first = canonical::order(cx, anchor, focus) != Ordering::Greater;
    let (start, end) = if anchor_first {
        (anchor, focus)
    } else {
        (focus, anchor)
    };
    let caret = |position| Selection {
        anchor: position,
        focus: position,
        affinity,
    };
    let collapsed = canonical::order(cx, start, end) == Ordering::Equal;
    if collapsed || most_backward(cx, start) == most_backward(cx, end) {
        return caret(start);
    }
    let (mut first, mut last) = (most_forward(cx, start), most_backward(cx, end));
    if canonical::order(cx, first, last) == Ordering::Greater {
        (first, last) = (start, end);
    }
    let forward = anchor_first || canonical::order(cx, first, last) == Ordering::Equal;
    let (anchor, focus) = if forward {
        (first, last)
    } else {
        (last, first)
    };
    Selection {
        anchor,
        focus,
        affinity,
    }
}
