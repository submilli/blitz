//! Bidi adjustment of caret positions at the boundaries of bidi runs
//! (Blink's `bidi_adjustment.cc`). A caret at the start or end of a run of
//! the other direction than its paragraph draws at the side of the whole
//! run that the logical movement reaches.
//!
//! A line's leaves are in visual order, left to right. Parley reports a
//! run's direction, not its embedding level, so a leaf in the paragraph's
//! direction takes the paragraph's level and any other leaf the next one.

use super::caret::{Caret, Side};
use super::ifc::{Ifc, ItemKind};
use std::rc::Rc;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Edge {
    Left,
    Right,
}

/// A leaf and one of its sides (`AbstractInlineBoxAndSideAffinity`).
#[derive(Clone, Copy)]
struct BoxSide {
    item: usize,
    edge: Edge,
}

/// Traversal toward the right, or toward the left.
#[derive(Clone, Copy)]
struct Strategy {
    right: bool,
}

/// The caret a position resolved to, after `AdjustForInlineCaretPositionResolution`.
pub(super) fn adjust_for_resolution(caret: Caret) -> Caret {
    adjust(caret, |run, side| run.resolution(side))
}

/// A hit-test result, after `AdjustForHitTest`.
pub(super) fn adjust_for_hit_test(caret: Caret) -> Caret {
    adjust(caret, |run, side| run.hit_test(side))
}

fn adjust(caret: Caret, adjuster: impl Fn(&Runs, BoxSide) -> BoxSide) -> Caret {
    let item = &caret.ifc.items[caret.item];
    let at_start = match caret.side {
        Side::BeforeBox => true,
        Side::AfterBox => false,
        Side::AtText(offset) if offset == item.content.start => true,
        Side::AtText(offset) if offset == item.content.end => false,
        // Inside a leaf a caret is at no bidi boundary.
        Side::AtText(_) => return caret,
    };
    let edge = if at_start != item.rtl {
        Edge::Left
    } else {
        Edge::Right
    };
    let runs = Runs { ifc: &caret.ifc };
    let adjusted = adjuster(
        &runs,
        BoxSide {
            item: caret.item,
            edge,
        },
    );
    runs.caret(&caret.ifc, adjusted)
}

/// A line's leaves and their levels.
struct Runs<'a> {
    ifc: &'a Ifc,
}

impl Runs<'_> {
    fn level(&self, item: usize) -> u8 {
        let item = &self.ifc.items[item];
        let paragraph_rtl = self.ifc.lines[item.line].rtl;
        let base = u8::from(paragraph_rtl);
        if item.rtl == paragraph_rtl {
            base
        } else {
            base + 1
        }
    }

    fn rtl(&self, item: usize) -> bool {
        self.ifc.items[item].rtl
    }

    fn paragraph_rtl(&self, item: usize) -> bool {
        self.ifc.lines[self.ifc.items[item].line].rtl
    }

    /// The next leaf to the left or right on the item's line.
    fn neighbour(&self, item: usize, right: bool, ignore_breaks: bool) -> Option<usize> {
        let range = self.ifc.lines[self.ifc.items[item].line].items.clone();
        let is_candidate = |index: &usize| {
            let candidate = &self.ifc.items[*index];
            candidate.is_leaf() && !(ignore_breaks && candidate.is_line_break())
        };
        if right {
            (item + 1..range.end).find(is_candidate)
        } else {
            (range.start..item).rev().find(is_candidate)
        }
    }

    fn caret(&self, ifc: &Rc<Ifc>, side: BoxSide) -> Caret {
        let item = &ifc.items[side.item];
        let at_start = (side.edge == Edge::Left) != item.rtl;
        let side_of = match item.kind {
            ItemKind::Text { .. } if at_start => Side::AtText(item.content.start),
            ItemKind::Text { .. } => Side::AtText(item.content.end),
            _ if at_start => Side::BeforeBox,
            _ => Side::AfterBox,
        };
        Caret {
            ifc: ifc.clone(),
            item: side.item,
            side: side_of,
        }
    }

    /// `InlineCaretPositionResolutionAdjuster`: traverses right from a
    /// caret at a leaf's left side, left from its right side.
    fn resolution(&self, unadjusted: BoxSide) -> BoxSide {
        let s = Strategy {
            right: unadjusted.edge == Edge::Left,
        };
        let item = unadjusted.item;
        if self.rtl(item) == self.paragraph_rtl(item) {
            return self.primary_resolution(s, item);
        }
        let level = self.level(item);
        let backward = self.neighbour(item, !s.right, true);
        if backward.is_none_or(|b| self.level(b) < level) {
            // The backward side of a secondary run: the forward side of the
            // entire run.
            let found = self.entire_run_boundary(s, item, level, true);
            return s.forward_side(found);
        }
        if backward.is_some_and(|b| self.level(b) <= level) {
            return s.backward_side(item);
        }
        // The forward side of a tertiary run: the backward side of it.
        let found = self.run_boundary(s.reverse(), item, level);
        s.backward_side(found)
    }

    fn primary_resolution(&self, s: Strategy, item: usize) -> BoxSide {
        if self.starts_different_direction(s, item) {
            return s.backward_side(item);
        }
        let Some(backward) = self.neighbour(item, !s.right, false) else {
            return s.backward_side(item);
        };
        let level = self.level(backward);
        let forward = self.find_run(s, item, level);
        if forward.is_some_and(|f| self.level(f) == level) {
            return s.backward_side(item);
        }
        let found = self.entire_run_boundary(s.reverse(), item, level, false);
        s.backward_side(found)
    }

    fn starts_different_direction(&self, s: Strategy, item: usize) -> bool {
        let Some(backward) = self.neighbour(item, !s.right, false) else {
            return true;
        };
        if s.right {
            self.rtl(backward) == self.rtl(item) || self.level(backward) > self.level(item)
        } else {
            self.level(backward) >= self.level(item)
        }
    }

    /// `HitTestAdjuster`.
    fn hit_test(&self, unadjusted: BoxSide) -> BoxSide {
        let s = Strategy {
            right: unadjusted.edge == Edge::Left,
        };
        let item = unadjusted.item;
        if self.rtl(item) == self.paragraph_rtl(item) {
            return s.backward_side(item);
        }
        let level = self.level(item);
        let backward = self.neighbour(item, !s.right, true);
        if let Some(backward) = backward {
            if self.level(backward) == level {
                return s.backward_side(item);
            }
            if self.level(backward) > level {
                let most = self.run_boundary(s.reverse(), backward, level);
                return s.forward_side(most);
            }
        }
        let most = self.entire_run_boundary(s, item, level, true);
        if self.rtl(item) == self.rtl(most) {
            s.forward_side(most)
        } else {
            s.backward_side(most)
        }
    }

    /// `FindBidiRun`: the first leaf beyond `start` at or below `level`.
    fn find_run(&self, s: Strategy, start: usize, level: u8) -> Option<usize> {
        let mut runner = self.neighbour(start, s.right, false);
        while let Some(current) = runner {
            if self.level(current) <= level {
                return Some(current);
            }
            runner = self.neighbour(current, s.right, false);
        }
        None
    }

    /// `FindBoundaryOfBidiRunIgnoringLineBreak`: the last leaf above `level`.
    fn run_boundary(&self, s: Strategy, start: usize, level: u8) -> usize {
        let mut last = start;
        let mut runner = self.neighbour(start, s.right, true);
        while let Some(current) = runner {
            if self.level(current) <= level {
                return last;
            }
            last = current;
            runner = self.neighbour(current, s.right, true);
        }
        last
    }

    /// `FindBoundaryOfEntireBidiRun`: the last leaf at or above `level`.
    fn entire_run_boundary(
        &self,
        s: Strategy,
        start: usize,
        level: u8,
        ignore_breaks: bool,
    ) -> usize {
        let mut last = start;
        let mut runner = self.neighbour(start, s.right, ignore_breaks);
        while let Some(current) = runner {
            if self.level(current) < level {
                return last;
            }
            last = current;
            runner = self.neighbour(current, s.right, ignore_breaks);
        }
        last
    }
}

impl Strategy {
    fn reverse(self) -> Self {
        Self { right: !self.right }
    }

    fn forward_side(self, item: usize) -> BoxSide {
        BoxSide {
            item,
            edge: if self.right { Edge::Right } else { Edge::Left },
        }
    }

    fn backward_side(self, item: usize) -> BoxSide {
        self.reverse().forward_side(item)
    }
}
