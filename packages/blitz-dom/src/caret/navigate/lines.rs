//! Line and paragraph movement. Floats have no line box (see
//! [`crate::caret::float`]): steps pass over them and their stops move
//! from the content they stand for.

use super::{Caret, Mover, line_of};
use crate::caret::float::{self, Beside};
use crate::caret::flow::Flow;

impl Mover<'_> {
    /// The stop on the next line nearest `x`. Past the last line, the caret
    /// goes to the document's end (or start, moving backward).
    pub(super) fn line(&self, caret: Caret, x: f32) -> Option<Caret> {
        let (from, line) = self.vertical_origin(caret)?;
        let within = match line {
            Some(line) if self.forward => line.checked_add(1),
            Some(line) => line.checked_sub(1),
            None => self.forward.then_some(0),
        };
        if let Some(found) = within.and_then(|target| self.root_line(from, target, x)) {
            return Some(found);
        }
        self.edge_line(from, caret.flow, x)
            .or_else(|| self.document_boundary())
    }

    /// The flow and line a vertical step starts from. A float's stop starts
    /// from the content it stands for; one opening a line, from the line
    /// before (`None` above the root's first line); one ending the root,
    /// moving backward, from the line below.
    fn vertical_origin(&self, caret: Caret) -> Option<(usize, Option<u32>)> {
        let flow = self.flows.flow(caret.flow)?;
        let Some(stops) = flow.float_stops() else {
            return Some((caret.flow, Some(line_of(flow, caret))));
        };
        if let Some((beside, beside_flow)) = self.stand_in_on_root(caret) {
            return Some((beside.flow, Some(line_of(beside_flow, beside))));
        }
        let line = flow.line(0);
        Some(match stops.side(caret.index > 0) {
            Beside::LineStart => (caret.flow, line.checked_sub(1)),
            Beside::LineEnd if !self.forward => (caret.flow, line.checked_add(1)),
            _ => (caret.flow, Some(line)),
        })
    }

    /// The stop nearest `x` on `line` of an inline root, from one of its
    /// flows: in that flow, else in the next of the root's flows that holds
    /// the line. Floats hold no line.
    fn root_line(&self, from: usize, line: u32, x: f32) -> Option<Caret> {
        let flow = self.flows.flow(from)?;
        if let Some(found) = nearest(flow, from, line, x).filter(|_| !flow.is_float()) {
            return Some(found);
        }
        let root = flow.root();
        self.beyond(from).find_map(|index| {
            let next = self.flows.flow(index)?;
            if next.root() != root || next.is_float() {
                return None;
            }
            nearest(next, index, line, x)
        })
    }

    /// The stop nearest `x` on the first line of the next paragraph, or the
    /// last line of the previous one.
    pub(super) fn paragraph(&self, caret: Caret, x: f32) -> Option<Caret> {
        let flow = self.flows.flow(caret.flow)?;
        let (start, end) = flow.paragraph_range(caret.index);
        let target = if self.forward {
            (end + 1 < flow.len()).then(|| flow.line(end + 1))
        } else {
            start.checked_sub(1).map(|i| flow.line(i))
        };
        if let Some(found) = target.and_then(|line| nearest(flow, caret.flow, line, x)) {
            return Some(found);
        }
        Some(self.edge_line(caret.flow, caret.flow, x).unwrap_or(caret))
    }

    /// The stop nearest `x` on the facing line of the next flow with stops,
    /// other than the `origin` flow a step started in. A float's stops draw
    /// at one edge: the one facing the movement wins.
    fn edge_line(&self, from: usize, origin: usize, x: f32) -> Option<Caret> {
        self.beyond(from).find_map(|index| {
            let flow = self.flows.flow(index).filter(|_| index != origin)?;
            flow.first_stop()?;
            if flow.is_float() {
                let stop = if self.forward {
                    flow.first_stop()
                } else {
                    flow.last_stop()
                };
                return Some(Caret::at(index, stop?));
            }
            let line = if self.forward {
                flow.line(0)
            } else {
                flow.line(flow.len())
            };
            nearest(flow, index, line, x)
        })
    }

    /// The end (or start) of the caret's visual line. A float's stops have
    /// no line box of their own: they move from the content they stand for,
    /// or stay where there is none.
    pub(super) fn line_boundary(&self, caret: Caret) -> Option<Caret> {
        let flow = self.flows.flow(caret.flow)?;
        if !flow.is_float() {
            return self.line_boundary_from(caret);
        }
        match float::stand_in(self.flows, caret) {
            Some(beside) => self.line_boundary_from(beside),
            None => Some(caret),
        }
    }

    /// The end (or start) of the line of an inline caret. A line continues
    /// through the root's other flows on it, around boxes that split the
    /// root, and passes over floats.
    fn line_boundary_from(&self, caret: Caret) -> Option<Caret> {
        let flow = self.flows.flow(caret.flow)?;
        let line = line_of(flow, caret);
        let (start, end) = flow.line_range(line)?;
        let mut found = self
            .outermost_stop(flow, caret.flow, start, end)
            .unwrap_or(caret);
        if self.open(flow, start, end) {
            found = self.continue_line(caret.flow, line, found);
        }
        let flow = self.flows.flow(found.flow)?;
        found.upstream = self.forward && flow.line(found.index) != line;
        Some(found)
    }

    /// The line's end (or start) through the root's flows after `from`.
    /// A float has no line box and is passed over, but the line ends at
    /// it where Chrome canonicalizes the line's end there: at its stop when
    /// a box on the line comes next (moving forward) or content without
    /// stops comes next (hidden text, an empty box), and, moving forward,
    /// at the content its stop stands for when the line ends right after
    /// it, before a forced break.
    fn continue_line(&self, from: usize, line: u32, mut found: Caret) -> Caret {
        let Some(root) = self.flows.flow(from).map(Flow::root) else {
            return found;
        };
        let mut passed_float = None;
        for index in self.beyond(from) {
            let Some(next) = self.flows.flow(index) else {
                break;
            };
            if next.root() != root {
                continue;
            }
            if next.is_float() {
                let stop = if self.forward {
                    next.last_stop()
                } else {
                    next.first_stop()
                };
                passed_float = stop.map(|stop| Caret::at(index, stop));
                if self.forward && box_follows_on(next, line) {
                    found = passed_float.take().unwrap_or(found);
                }
                continue;
            }
            if next.is_empty() {
                found = passed_float.take().unwrap_or(found);
                continue;
            }
            let float = passed_float.take();
            let Some((start, end)) = next.line_range(line) else {
                break;
            };
            // A flow with no stop on the line (only unselectable content)
            // keeps the previous end.
            if let Some(stop) = self.outermost_stop(next, index, start, end) {
                let at_float = float.filter(|_| self.forward && stop.index == 0);
                found = at_float
                    .and_then(|float| float::stand_in(self.flows, float))
                    .unwrap_or(stop);
            }
            if !self.open(next, start, end) {
                break;
            }
        }
        found
    }

    /// Whether a line's stops `start..=end` reach the flow's edge in the
    /// direction of movement, so the line continues in the next flow.
    fn open(&self, flow: &Flow, start: usize, end: usize) -> bool {
        if self.forward {
            end == flow.len()
        } else {
            start == 0
        }
    }
}

/// Whether a float is followed by a box on `line`, whose content (if any)
/// forms flows of its own.
fn box_follows_on(flow: &Flow, line: u32) -> bool {
    flow.float_stops()
        .is_some_and(|stops| matches!(stops.after, Beside::Into { line: on, .. } if on == line))
}

/// The stop on `line` whose caret is horizontally closest to `x`; the first
/// one wins a tie. A stop past the line's last unit draws upstream.
fn nearest(flow: &Flow, index: usize, line: u32, x: f32) -> Option<Caret> {
    let (start, end) = flow.line_range(line)?;
    let stop = (start..=end)
        .filter(|&i| flow.point(i).is_some())
        .min_by(|&a, &b| {
            let da = (flow.x(a, line) - x).abs();
            let db = (flow.x(b, line) - x).abs();
            da.total_cmp(&db)
        })?;
    Some(Caret {
        flow: index,
        index: stop,
        upstream: flow.line(stop) != line,
    })
}
