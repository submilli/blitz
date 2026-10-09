//! Selection movement over caret stops, following Chrome on macOS.
//!
//! Moving a non-collapsed selection by character collapses it; by word or
//! sentence it moves from the focus; by line, paragraph or a boundary it
//! moves from the start or end. Extension moves the focus, except that word,
//! line and paragraph steps never carry it across the anchor, boundary steps
//! grow the selection at the side they move toward, and the focus stays in
//! the anchor's tree (a selection does not cross a shadow boundary). Line
//! and paragraph steps keep a horizontal position across consecutive moves.

use super::float::{self, Beside};
use super::flow::Flow;
use super::segments;
use super::{Alter, Direction, Granularity, ModifyRequest};
use crate::NodeId;
use std::cmp::Ordering;

/// A caret stop: a unit boundary in one flow, ordered by flow then boundary.
/// At a soft wrap one boundary ends a line and starts the next; `upstream`
/// draws it at the end of the earlier line. Affinity does not affect order
/// or equality.
#[derive(Clone, Copy, Debug)]
pub(super) struct Caret {
    pub(super) flow: usize,
    pub(super) index: usize,
    pub(super) upstream: bool,
}

impl Caret {
    pub(super) fn at(flow: usize, index: usize) -> Self {
        Self {
            flow,
            index,
            upstream: false,
        }
    }
}

impl PartialEq for Caret {
    fn eq(&self, other: &Self) -> bool {
        (self.flow, self.index) == (other.flow, other.index)
    }
}

impl Eq for Caret {}

impl PartialOrd for Caret {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Caret {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.flow, self.index).cmp(&(other.flow, other.index))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Carets {
    pub(super) anchor: Caret,
    pub(super) focus: Caret,
}

/// Flows in tree order.
pub(super) trait Flows {
    fn count(&self) -> usize;
    fn flow(&self, index: usize) -> Option<&Flow>;

    /// The root of the node tree a stop is in; one tree when `None`.
    fn tree(&self, _: Caret) -> Option<NodeId> {
        None
    }
}

/// The moved selection and, after a line or paragraph step, the horizontal
/// position later steps keep.
pub(super) fn modify(
    flows: &dyn Flows,
    selection: Carets,
    request: ModifyRequest,
) -> Option<(Carets, Option<f32>)> {
    let Carets { anchor, focus } = selection;
    // Affinity only matters at a soft wrap; a caret's ends share it.
    let mut focus = focus;
    focus.upstream &= flows.flow(focus.flow)?.is_soft_wrap(focus.index);
    let anchor = if anchor == focus { focus } else { anchor };
    let rtl = flows.flow(focus.flow)?.rtl();
    let forward = match request.direction {
        Direction::Forward => true,
        Direction::Backward => false,
        Direction::Right => !rtl,
        Direction::Left => rtl,
    };
    let (start, end) = (anchor.min(focus), anchor.max(focus));
    let granularity = request.granularity;
    let mover = Mover { flows, forward };
    let vertical = matches!(granularity, Granularity::Line | Granularity::Paragraph);
    if request.alter == Alter::Move {
        let origin = match granularity {
            Granularity::Character if start != end => {
                return Some((collapsed(if forward { end } else { start }), None));
            }
            Granularity::Character | Granularity::Word | Granularity::Sentence => focus,
            _ if forward => end,
            _ => start,
        };
        // Chrome takes a range's vertical position from its start, in either
        // direction.
        let x = request.line_x.or_else(|| mover.x(start)).unwrap_or(0.0);
        let target = mover.step(origin, granularity, x)?;
        return Some((collapsed(target), vertical.then_some(x)));
    }
    if start != end && granularity.is_boundary() {
        let carets = if forward {
            let focus = mover.step(end, granularity, 0.0)?;
            Carets {
                anchor: start,
                focus: clamp_to_tree(flows, start, focus),
            }
        } else {
            let anchor = mover.step(start, granularity, 0.0)?;
            Carets {
                anchor: clamp_to_tree(flows, end, anchor),
                focus: end,
            }
        };
        return Some((carets, None));
    }
    let x = request.line_x.or_else(|| mover.x(focus)).unwrap_or(0.0);
    let mut target = mover.step(focus, granularity, x)?;
    let crosses = (focus > anchor && target < anchor) || (focus < anchor && target > anchor);
    if crosses && granularity.keeps_anchor_side() {
        target = anchor;
    }
    let carets = Carets {
        anchor,
        focus: clamp_to_tree(flows, anchor, target),
    };
    Some((carets, vertical.then_some(x)))
}

fn collapsed(caret: Caret) -> Carets {
    Carets {
        anchor: caret,
        focus: caret,
    }
}

/// Walk `moving` back toward `fixed` until both are in one tree.
fn clamp_to_tree(flows: &dyn Flows, fixed: Caret, moving: Caret) -> Caret {
    let toward = Mover {
        flows,
        forward: moving < fixed,
    };
    let tree = flows.tree(fixed);
    let mut caret = moving;
    while caret != fixed && flows.tree(caret) != tree {
        match toward.character(caret) {
            Some(next) if next != caret => caret = next,
            _ => return fixed,
        }
    }
    caret
}

impl Granularity {
    fn is_boundary(self) -> bool {
        matches!(
            self,
            Self::LineBoundary
                | Self::SentenceBoundary
                | Self::ParagraphBoundary
                | Self::DocumentBoundary
        )
    }

    fn keeps_anchor_side(self) -> bool {
        matches!(self, Self::Word | Self::Line | Self::Paragraph)
    }
}

struct Mover<'a> {
    flows: &'a dyn Flows,
    forward: bool,
}

impl Mover<'_> {
    /// One step of `granularity`; `x` is the kept horizontal position.
    fn step(&self, caret: Caret, granularity: Granularity, x: f32) -> Option<Caret> {
        match granularity {
            Granularity::Character => self.character(caret),
            Granularity::Word => self.word(caret),
            Granularity::Sentence => self.sentence(caret),
            Granularity::SentenceBoundary => self.sentence_boundary(caret),
            Granularity::Line => self.line(caret, x),
            Granularity::Paragraph => self.paragraph(caret, x),
            Granularity::LineBoundary => self.line_boundary(caret),
            Granularity::ParagraphBoundary => self.paragraph_boundary(caret),
            Granularity::DocumentBoundary => self.document_boundary(),
        }
    }

    /// The horizontal position of a caret on its own line; a float's stop
    /// takes that of the content it stands for.
    fn x(&self, caret: Caret) -> Option<f32> {
        let caret = float::stand_in(self.flows, caret).unwrap_or(caret);
        let flow = self.flows.flow(caret.flow)?;
        Some(flow.x(caret.index, line_of(flow, caret)))
    }

    fn character(&self, caret: Caret) -> Option<Caret> {
        let flow = self.flows.flow(caret.flow)?;
        let next = if self.forward {
            flow.next_stop(caret.index)
        } else {
            flow.previous_stop(caret.index)
        };
        if let Some(index) = next {
            return Some(Caret::at(caret.flow, index));
        }
        Some(self.adjacent_edge(caret.flow).unwrap_or(caret))
    }

    fn word(&self, caret: Caret) -> Option<Caret> {
        let flow = self.flows.flow(caret.flow)?;
        if let Some(index) = self.word_stop(flow, flow.offset(caret.index)) {
            return Some(Caret::at(caret.flow, index));
        }
        let beyond = self.beyond(caret.flow).find_map(|index| {
            let flow = self.flows.flow(index)?;
            let edge = if self.forward { 0 } else { flow.text().len() };
            Some(Caret::at(index, self.word_stop(flow, edge)?))
        });
        beyond.or_else(|| self.document_boundary())
    }

    /// The stop at the next word end (or previous word start) from a text offset.
    fn word_stop(&self, flow: &Flow, offset: usize) -> Option<usize> {
        let found = if self.forward {
            segments::next_word_end(flow.text(), offset)
        } else {
            segments::previous_word_start(flow.text(), offset)
        };
        flow.stop_near(flow.index_at(found?))
    }

    /// The next sentence boundary, continuing into adjacent flows.
    fn sentence(&self, caret: Caret) -> Option<Caret> {
        let found = self.sentence_within(caret);
        Some(
            found
                .or_else(|| self.adjacent_edge(caret.flow))
                .unwrap_or(caret),
        )
    }

    /// The current sentence's end (or start), within this flow.
    fn sentence_boundary(&self, caret: Caret) -> Option<Caret> {
        Some(self.sentence_within(caret).unwrap_or(caret))
    }

    fn sentence_within(&self, caret: Caret) -> Option<Caret> {
        let flow = self.flows.flow(caret.flow)?;
        let offset = flow.offset(caret.index);
        let found = if self.forward {
            segments::next_sentence(flow.text(), offset)
        } else {
            segments::previous_sentence(flow.text(), offset)
        };
        let index = flow.stop_near(flow.index_at(found?))?;
        // A boundary with no stop past the caret (unselectable content at
        // the flow's end) leaves the flow.
        let advances = if self.forward {
            index > caret.index
        } else {
            index < caret.index
        };
        advances.then(|| Caret::at(caret.flow, index))
    }

    /// The stop on the next line nearest `x`. Past the last line, the caret
    /// goes to the document's end (or start, moving backward).
    fn line(&self, caret: Caret, x: f32) -> Option<Caret> {
        let (from, line) = self.vertical_origin(caret)?;
        let within = if self.forward {
            line.checked_add(1)
        } else {
            line.checked_sub(1)
        };
        if let Some(found) = within.and_then(|target| self.root_line(from, target, x)) {
            return Some(found);
        }
        self.edge_line(from, x).or_else(|| self.document_boundary())
    }

    /// The flow and line a vertical step starts from. A float's stop starts
    /// from the content it stands for; one opening a line, from the line
    /// before.
    fn vertical_origin(&self, caret: Caret) -> Option<(usize, u32)> {
        let flow = self.flows.flow(caret.flow)?;
        let Some(stops) = flow.float_stops() else {
            return Some((caret.flow, line_of(flow, caret)));
        };
        if let Some(beside) = float::stand_in(self.flows, caret) {
            let beside_flow = self.flows.flow(beside.flow)?;
            return Some((beside.flow, line_of(beside_flow, beside)));
        }
        let line = flow.line(0);
        Some(match stops.side(caret.index > 0) {
            Beside::LineStart => (caret.flow, line.saturating_sub(1)),
            _ => (caret.flow, line),
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
    fn paragraph(&self, caret: Caret, x: f32) -> Option<Caret> {
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
        Some(self.edge_line(caret.flow, x).unwrap_or(caret))
    }

    /// The stop nearest `x` on the facing line of the next flow with stops.
    /// A float's stops draw at one edge: the one facing the movement wins.
    fn edge_line(&self, from: usize, x: f32) -> Option<Caret> {
        self.beyond(from).find_map(|index| {
            let flow = self.flows.flow(index)?;
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
    fn line_boundary(&self, caret: Caret) -> Option<Caret> {
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
        let open = |flow: &Flow, start: usize, end: usize| {
            if self.forward {
                end == flow.len()
            } else {
                start == 0
            }
        };
        if open(flow, start, end) {
            for index in self.beyond(caret.flow) {
                let Some(next) = self.flows.flow(index) else {
                    break;
                };
                if next.root() != flow.root() || next.is_float() {
                    continue;
                }
                let Some((start, end)) = next.line_range(line) else {
                    break;
                };
                // A flow with no stop on the line (only unselectable content)
                // keeps the previous end.
                if let Some(stop) = self.outermost_stop(next, index, start, end) {
                    found = stop;
                }
                if !open(next, start, end) {
                    break;
                }
            }
        }
        let flow = self.flows.flow(found.flow)?;
        found.upstream = self.forward && flow.line(found.index) != line;
        Some(found)
    }

    fn paragraph_boundary(&self, caret: Caret) -> Option<Caret> {
        let flow = self.flows.flow(caret.flow)?;
        let (start, end) = flow.paragraph_range(caret.index);
        Some(
            self.outermost_stop(flow, caret.flow, start, end)
                .unwrap_or(caret),
        )
    }

    /// The outermost stop of `start..=end` in the direction of movement.
    fn outermost_stop(&self, flow: &Flow, index: usize, start: usize, end: usize) -> Option<Caret> {
        let stop = if self.forward {
            Some(end)
                .filter(|&i| flow.point(i).is_some())
                .or_else(|| flow.previous_stop(end))
        } else {
            Some(start)
                .filter(|&i| flow.point(i).is_some())
                .or_else(|| flow.next_stop(start))
        };
        Some(Caret::at(index, stop?))
    }

    fn document_boundary(&self) -> Option<Caret> {
        let count = self.flows.count();
        let found = |flow: usize| {
            let found = self.flows.flow(flow)?;
            let index = if self.forward {
                found.last_stop()
            } else {
                found.first_stop()
            }?;
            Some(Caret::at(flow, index))
        };
        if self.forward {
            (0..count).rev().find_map(found)
        } else {
            (0..count).find_map(found)
        }
    }

    /// The nearest stop of the next flow with stops, entered from its edge.
    fn adjacent_edge(&self, from: usize) -> Option<Caret> {
        self.beyond(from).find_map(|flow| {
            let found = self.flows.flow(flow)?;
            let index = if self.forward {
                found.first_stop()
            } else {
                found.last_stop()
            }?;
            Some(Caret::at(flow, index))
        })
    }

    /// Flow indices beyond `from` in the direction of movement.
    fn beyond(&self, from: usize) -> Box<dyn Iterator<Item = usize>> {
        if self.forward {
            Box::new(from + 1..self.flows.count())
        } else {
            Box::new((0..from).rev())
        }
    }
}

/// The line a caret draws on, honouring its affinity.
fn line_of(flow: &Flow, caret: Caret) -> u32 {
    match caret.index.checked_sub(1) {
        Some(before) if caret.upstream => flow.line(before),
        _ => flow.line(caret.index),
    }
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
