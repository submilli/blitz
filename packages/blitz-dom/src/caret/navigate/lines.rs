//! Line and paragraph movement within one flow, for text controls. A text
//! control's content is a single flow; document lines follow Chrome's model
//! in [`crate::caret::visible`].

use super::{Caret, Mover, line_of};
use crate::caret::flow::Flow;

impl Mover<'_> {
    /// The stop on the next line nearest `x`. Past the last line, the caret
    /// goes to the content's end (or start, moving backward).
    pub(super) fn line(&self, caret: Caret, x: f32) -> Option<Caret> {
        let flow = self.flows.flow(caret.flow)?;
        let line = line_of(flow, caret);
        let target = if self.forward {
            line.checked_add(1)
        } else {
            line.checked_sub(1)
        };
        target
            .and_then(|target| nearest(flow, caret.flow, target, x))
            .or_else(|| self.document_boundary())
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
        let found = target.and_then(|line| nearest(flow, caret.flow, line, x));
        Some(found.unwrap_or(caret))
    }

    /// The end (or start) of the caret's visual line.
    pub(super) fn line_boundary(&self, caret: Caret) -> Option<Caret> {
        let flow = self.flows.flow(caret.flow)?;
        let line = line_of(flow, caret);
        let (start, end) = flow.line_range(line)?;
        let mut found = self
            .outermost_stop(flow, caret.flow, start, end)
            .unwrap_or(caret);
        found.upstream = self.forward && flow.line(found.index) != line;
        Some(found)
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
