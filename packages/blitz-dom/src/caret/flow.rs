//! A run of inline content as a sequence of caret units.
//!
//! A unit is one step of character movement: a grapheme cluster, an atomic
//! inline (a replaced element or control), an unselectable run, or a forced
//! line break. Boundary `i` lies before unit `i`; it is a caret stop when some
//! DOM position renders there. Collapsed white space, hidden text and
//! generated content take no units, so positions around them coincide.

use super::geometry::Edges;
use super::order::Key;
use crate::{NodeId, ranges::Boundary};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum UnitKind {
    Text,
    Atom,
    Break,
}

#[derive(Clone, Copy, Debug)]
struct Unit {
    kind: UnitKind,
    edges: Edges,
}

/// A DOM boundary point and the unit boundary it renders at.
#[derive(Clone, Copy, Debug)]
pub(super) struct Entry {
    pub(super) key: Key,
    pub(super) point: Boundary,
    pub(super) index: usize,
    /// Whether the point may represent its caret stop. The first candidate
    /// in tree order wins, which keeps a position at the end of a preceding
    /// Text node rather than the start of the following one.
    pub(super) candidate: bool,
}

/// Built by pushing units and entries in order, then [`Flow::finish`].
pub(super) struct Flow {
    root: NodeId,
    rtl: bool,
    /// The units' text, for word and sentence segmentation.
    text: String,
    /// Text offset of every unit's start; `finish` appends the end.
    offsets: Vec<usize>,
    units: Vec<Unit>,
    /// Entries in push order, whose indices never decrease.
    entries: Vec<Entry>,
    /// The representative DOM point of every unit boundary that is a stop.
    stops: Vec<Option<Boundary>>,
}

impl Flow {
    /// An empty flow of the inline root `root`.
    pub(super) fn new(root: NodeId, rtl: bool) -> Self {
        Self {
            root,
            rtl,
            text: String::new(),
            offsets: Vec::new(),
            units: Vec::new(),
            entries: Vec::new(),
            stops: Vec::new(),
        }
    }

    pub(super) fn push_unit(&mut self, kind: UnitKind, edges: Edges, text: &str) {
        self.offsets.push(self.text.len());
        self.text.push_str(text);
        self.units.push(Unit { kind, edges });
    }

    /// Continue the last text unit with more of its grapheme cluster.
    /// Returns false when the last unit is not text.
    pub(super) fn extend_text(&mut self, text: &str, trailing: f32) -> bool {
        let Some(unit) = self.units.last_mut().filter(|u| u.kind == UnitKind::Text) else {
            return false;
        };
        unit.edges.trailing = trailing;
        self.text.push_str(text);
        true
    }

    /// Remove the last unit, moving the entries after it onto its start.
    pub(super) fn pop_unit(&mut self) {
        let Some(start) = self.offsets.pop() else {
            return;
        };
        self.units.pop();
        self.text.truncate(start);
        let dropped = self.units.len();
        for entry in self.entries.iter_mut().rev() {
            if entry.index <= dropped {
                break;
            }
            entry.index = dropped;
        }
    }

    pub(super) fn push_entry(&mut self, entry: Entry) {
        self.entries.push(entry);
    }

    pub(super) fn len(&self) -> usize {
        self.units.len()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.units.is_empty()
    }

    /// Close the text and index the caret stops.
    pub(super) fn finish(mut self) -> Self {
        self.offsets.push(self.text.len());
        self.stops = vec![None; self.units.len() + 1];
        for entry in &self.entries {
            if entry.candidate && self.stops[entry.index].is_none() {
                self.stops[entry.index] = Some(entry.point);
            }
        }
        self
    }

    pub(super) fn root(&self) -> NodeId {
        self.root
    }

    pub(super) fn rtl(&self) -> bool {
        self.rtl
    }

    pub(super) fn text(&self) -> &str {
        &self.text
    }

    /// The tree-order key of the first entry, which orders flows.
    pub(super) fn first_key(&self) -> Option<Key> {
        self.entries.first().map(|entry| entry.key)
    }

    pub(super) fn last_key(&self) -> Option<Key> {
        self.entries.last().map(|entry| entry.key)
    }

    pub(super) fn point(&self, index: usize) -> Option<Boundary> {
        self.stops.get(index).copied().flatten()
    }

    pub(super) fn first_stop(&self) -> Option<usize> {
        self.stops.iter().position(Option::is_some)
    }

    pub(super) fn last_stop(&self) -> Option<usize> {
        self.stops.iter().rposition(Option::is_some)
    }

    pub(super) fn next_stop(&self, index: usize) -> Option<usize> {
        (index + 1..self.stops.len()).find(|&i| self.stops[i].is_some())
    }

    pub(super) fn previous_stop(&self, index: usize) -> Option<usize> {
        (0..index.min(self.stops.len())).rfind(|&i| self.stops[i].is_some())
    }

    /// The stop at or after `index`, else the one before it.
    pub(super) fn stop_near(&self, index: usize) -> Option<usize> {
        if self.point(index).is_some() {
            return Some(index);
        }
        self.next_stop(index).or_else(|| self.previous_stop(index))
    }

    /// The stop of a boundary point that renders in this flow.
    pub(super) fn exact(&self, key: Key) -> Option<usize> {
        let entry = self.entries.iter().find(|entry| entry.key == key)?;
        self.stop_near(entry.index)
    }

    /// The first stop represented by a point after `key`.
    pub(super) fn after(&self, key: Key) -> Option<usize> {
        self.entries
            .iter()
            .find(|entry| entry.candidate && entry.key > key)
            .map(|entry| entry.index)
    }

    pub(super) fn offset(&self, index: usize) -> usize {
        self.offsets.get(index).copied().unwrap_or(self.text.len())
    }

    /// The unit boundary at a text offset, or the next one after it.
    pub(super) fn index_at(&self, offset: usize) -> usize {
        self.offsets.partition_point(|&o| o < offset)
    }

    /// The line a boundary is drawn on: the line of the unit after it, or of
    /// the last unit at the end.
    pub(super) fn line(&self, index: usize) -> u32 {
        self.units
            .get(index)
            .or_else(|| self.units.last())
            .map_or(0, |unit| unit.edges.line)
    }

    /// The caret's horizontal position at `index` when drawn on `line`.
    pub(super) fn x(&self, index: usize, line: u32) -> f32 {
        match self.units.get(index) {
            Some(unit) if unit.edges.line == line => unit.edges.leading,
            _ => index
                .checked_sub(1)
                .and_then(|i| self.units.get(i))
                .map_or(0.0, |unit| unit.edges.trailing),
        }
    }

    /// Stops drawn on `line`, from its first unit to its end. A line ends
    /// before a forced break, and before the white space a soft wrap hangs.
    pub(super) fn line_range(&self, line: u32) -> Option<(usize, usize)> {
        let first = self.units.iter().position(|u| u.edges.line == line)?;
        let last = self.units.iter().rposition(|u| u.edges.line == line)?;
        let wraps = self.units.get(last + 1).is_some();
        let hangs = wraps && last > first && self.unit_text(last).trim().is_empty();
        let end = if self.units[last].kind == UnitKind::Break || hangs {
            last
        } else {
            last + 1
        };
        Some((first, end))
    }

    pub(super) fn line_count(&self) -> u32 {
        self.units
            .iter()
            .map(|u| u.edges.line + 1)
            .max()
            .unwrap_or(0)
    }

    /// The paragraph around `index`: boundaries between forced breaks.
    pub(super) fn paragraph_range(&self, index: usize) -> (usize, usize) {
        let is_break = |i: &usize| self.units[*i].kind == UnitKind::Break;
        let start = (0..index.min(self.units.len()))
            .rfind(is_break)
            .map_or(0, |i| i + 1);
        let end = (index..self.units.len())
            .find(is_break)
            .unwrap_or(self.units.len());
        (start, end)
    }

    fn unit_text(&self, index: usize) -> &str {
        let end = self
            .offsets
            .get(index + 1)
            .copied()
            .unwrap_or(self.text.len());
        self.text.get(self.offset(index)..end).unwrap_or("")
    }
}
