//! Selection movement over caret stops, following Chrome on macOS.
//!
//! Moving a non-collapsed selection by character collapses it; by word or
//! sentence it moves from the focus; by line, paragraph or a boundary it
//! moves from the start or end. Extension moves the focus, except that word,
//! line and paragraph steps never carry it across the anchor, and boundary
//! steps grow the selection at the side they move toward. Line and
//! paragraph steps keep a horizontal position across consecutive moves.

use super::flow::Flow;
use super::segments;
use super::{Alter, Direction, Granularity, ModifyRequest};

/// A caret stop: a unit boundary in one flow, ordered by flow then boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Caret {
    pub(super) flow: usize,
    pub(super) index: usize,
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
}

/// The moved selection and, after a line or paragraph step, the horizontal
/// position later steps keep.
pub(super) fn modify(
    flows: &dyn Flows,
    selection: Carets,
    request: ModifyRequest,
) -> Option<(Carets, Option<f32>)> {
    let Carets { anchor, focus } = selection;
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
            Carets {
                anchor: start,
                focus: mover.step(end, granularity, 0.0)?,
            }
        } else {
            Carets {
                anchor: mover.step(start, granularity, 0.0)?,
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
        focus: target,
    };
    Some((carets, vertical.then_some(x)))
}

fn collapsed(caret: Caret) -> Carets {
    Carets {
        anchor: caret,
        focus: caret,
    }
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

    /// The horizontal position of a caret on its own line.
    fn x(&self, caret: Caret) -> Option<f32> {
        let flow = self.flows.flow(caret.flow)?;
        Some(flow.x(caret.index, flow.line(caret.index)))
    }

    fn character(&self, caret: Caret) -> Option<Caret> {
        let flow = self.flows.flow(caret.flow)?;
        let next = if self.forward {
            flow.next_stop(caret.index)
        } else {
            flow.previous_stop(caret.index)
        };
        if let Some(index) = next {
            return Some(Caret { index, ..caret });
        }
        Some(self.adjacent_edge(caret.flow).unwrap_or(caret))
    }

    fn word(&self, caret: Caret) -> Option<Caret> {
        let flow = self.flows.flow(caret.flow)?;
        let offset = flow.offset(caret.index);
        let within = if self.forward {
            segments::next_word_end(flow.text(), offset)
        } else {
            segments::previous_word_start(flow.text(), offset)
        };
        if let Some(index) = within.and_then(|o| flow.stop_near(flow.index_at(o))) {
            return Some(Caret { index, ..caret });
        }
        for index in self.beyond(caret.flow) {
            let Some(flow) = self.flows.flow(index) else {
                continue;
            };
            let found = if self.forward {
                segments::next_word_end(flow.text(), 0)
            } else {
                segments::previous_word_start(flow.text(), flow.text().len())
            };
            if let Some(stop) = found.and_then(|o| flow.stop_near(flow.index_at(o))) {
                return Some(Caret {
                    flow: index,
                    index: stop,
                });
            }
        }
        self.document_boundary()
    }

    /// The next sentence boundary, continuing into adjacent flows.
    fn sentence(&self, caret: Caret) -> Option<Caret> {
        let found = self.sentence_within(caret)?;
        Some(
            found
                .or_else(|| self.adjacent_edge(caret.flow))
                .unwrap_or(caret),
        )
    }

    /// The current sentence's end (or start), within this flow.
    fn sentence_boundary(&self, caret: Caret) -> Option<Caret> {
        Some(self.sentence_within(caret)?.unwrap_or(caret))
    }

    fn sentence_within(&self, caret: Caret) -> Option<Option<Caret>> {
        let flow = self.flows.flow(caret.flow)?;
        let offset = flow.offset(caret.index);
        let found = if self.forward {
            segments::next_sentence(flow.text(), offset)
        } else {
            segments::previous_sentence(flow.text(), offset)
        };
        let index = found.and_then(|o| flow.stop_near(flow.index_at(o)));
        Some(index.map(|index| Caret { index, ..caret }))
    }

    /// The stop on the next line nearest `x`. Past the last line, the caret
    /// goes to the document's end (or start, moving backward).
    fn line(&self, caret: Caret, x: f32) -> Option<Caret> {
        let flow = self.flows.flow(caret.flow)?;
        let line = flow.line(caret.index);
        let within = if self.forward {
            line.checked_add(1).filter(|&l| l < flow.line_count())
        } else {
            line.checked_sub(1)
        };
        if let Some(index) = within.and_then(|target| nearest(flow, target, x)) {
            return Some(Caret { index, ..caret });
        }
        self.edge_line(caret.flow, x)
            .or_else(|| self.document_boundary())
    }

    /// The stop nearest `x` on the first line of the next paragraph, or the
    /// last line of the previous one.
    fn paragraph(&self, caret: Caret, x: f32) -> Option<Caret> {
        let flow = self.flows.flow(caret.flow)?;
        let (start, end) = flow.paragraph_range(caret.index);
        let target = if self.forward {
            (end < flow.len()).then(|| flow.line(end + 1))
        } else {
            start.checked_sub(1).map(|i| flow.line(i))
        };
        if let Some(index) = target.and_then(|line| nearest(flow, line, x)) {
            return Some(Caret { index, ..caret });
        }
        Some(self.edge_line(caret.flow, x).unwrap_or(caret))
    }

    /// The stop nearest `x` on the facing line of the next flow with stops.
    fn edge_line(&self, from: usize, x: f32) -> Option<Caret> {
        self.beyond(from).find_map(|index| {
            let flow = self.flows.flow(index)?;
            flow.first_stop()?;
            let line = if self.forward {
                flow.line(0)
            } else {
                flow.line_count().saturating_sub(1)
            };
            Some(Caret {
                flow: index,
                index: nearest(flow, line, x)?,
            })
        })
    }

    fn line_boundary(&self, caret: Caret) -> Option<Caret> {
        let flow = self.flows.flow(caret.flow)?;
        let (start, end) = flow.line_range(flow.line(caret.index))?;
        Some(self.outermost_stop(flow, caret, start, end))
    }

    fn paragraph_boundary(&self, caret: Caret) -> Option<Caret> {
        let flow = self.flows.flow(caret.flow)?;
        let (start, end) = flow.paragraph_range(caret.index);
        Some(self.outermost_stop(flow, caret, start, end))
    }

    /// The outermost stop of `start..=end` in the direction of movement.
    fn outermost_stop(&self, flow: &Flow, caret: Caret, start: usize, end: usize) -> Caret {
        let index = if self.forward {
            Some(end)
                .filter(|&i| flow.point(i).is_some())
                .or_else(|| flow.previous_stop(end))
        } else {
            Some(start)
                .filter(|&i| flow.point(i).is_some())
                .or_else(|| flow.next_stop(start))
        };
        Caret {
            index: index.unwrap_or(caret.index),
            ..caret
        }
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
            Some(Caret { flow, index })
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
            Some(Caret { flow, index })
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

/// The stop on `line` whose caret is horizontally closest to `x`; the first
/// one wins a tie.
fn nearest(flow: &Flow, line: u32, x: f32) -> Option<usize> {
    let (start, end) = flow.line_range(line)?;
    (start..=end)
        .filter(|&i| flow.point(i).is_some())
        .min_by(|&a, &b| {
            let da = (flow.x(a, line) - x).abs();
            let db = (flow.x(b, line) - x).abs();
            da.total_cmp(&db)
        })
}
