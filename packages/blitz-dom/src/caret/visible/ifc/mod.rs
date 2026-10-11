//! One inline formatting context as Blink's editing sees it: the offset
//! mapping between DOM positions and the context's text content
//! (`OffsetMapping`), and its line boxes with their leaf items
//! (`FragmentItems`).
//!
//! The text content follows Blink's `InlineItemsBuilder`: rendered text
//! after white-space collapsing, U+FFFC for an atomic inline, a line feed
//! for a `<br>`, and nothing for floats and positioned boxes, so the
//! positions before and after one map to the same offset. Offsets count
//! UTF-16 code units.

use super::position::Position;
use super::tree::Tree;
use crate::NodeId;
use std::collections::{HashMap, HashSet};
use std::ops::Range;

mod build;
mod lines;

pub(super) use build::build;

/// A run of DOM offsets and the text content it maps to.
#[derive(Clone, Debug)]
pub(super) struct Unit {
    /// The DOM node, or `None` for generated content.
    pub(super) node: Option<NodeId>,
    pub(super) kind: UnitKind,
    pub(super) dom: Range<usize>,
    pub(super) content: Range<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum UnitKind {
    /// Each DOM unit is one content unit.
    Identity,
    /// DOM text that maps to content of another length (`text-transform`).
    Variable,
    /// Collapsed DOM text, mapping to an empty range.
    Collapsed,
}

/// A child of a line box: a leaf (text from one node, a forced break, or
/// an atomic inline), or a float the line holds.
#[derive(Clone, Debug)]
pub(super) struct Item {
    pub(super) kind: ItemKind,
    pub(super) line: usize,
    pub(super) content: Range<usize>,
    /// The item's horizontal extent, in document coordinates.
    pub(super) left: f32,
    pub(super) right: f32,
    pub(super) rtl: bool,
    /// Caret positions at each content offset of a text item, from its
    /// start to its end.
    pub(super) carets: Vec<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ItemKind {
    /// Text of `node` (`None` for generated content). A forced break is an
    /// item of its own.
    Text {
        node: Option<NodeId>,
        line_break: bool,
    },
    Atomic(NodeId),
    /// A float, which a line holds as a child but not as a leaf: it takes
    /// no text content and no caret position.
    Float(NodeId),
}

impl Item {
    /// The DOM node a leaf item draws; `None` for generated text and for
    /// floats, which are not leaves.
    pub(super) fn kind_node(&self) -> Option<NodeId> {
        match self.kind {
            ItemKind::Text { node, .. } => node,
            ItemKind::Atomic(node) => Some(node),
            ItemKind::Float(_) => None,
        }
    }

    /// A line's leaf: text, generated or not, or an atomic inline.
    pub(super) fn is_leaf(&self) -> bool {
        !matches!(self.kind, ItemKind::Float(_))
    }

    /// A forced break, an item of its own.
    pub(super) fn is_line_break(&self) -> bool {
        matches!(
            self.kind,
            ItemKind::Text {
                line_break: true,
                ..
            }
        )
    }
}

/// A line box.
#[derive(Clone, Debug)]
pub(super) struct Line {
    pub(super) items: Range<usize>,
    pub(super) top: f32,
    pub(super) bottom: f32,
    /// The line box's edges, around its leaves, in document coordinates.
    pub(super) left: f32,
    pub(super) right: f32,
    /// The edges of the block's content box.
    pub(super) container_left: f32,
    pub(super) container_right: f32,
    /// Text aligns to the right, so a caret keeps inside the line box's
    /// right edge rather than the block's (`ShouldAlignCaretRight`).
    pub(super) align_right: bool,
    pub(super) rtl: bool,
    /// The line ends at a soft wrap rather than a forced break or the end.
    pub(super) soft_wrap: bool,
}

pub(super) struct Ifc {
    pub(super) root: NodeId,
    text: Vec<u16>,
    units: Vec<Unit>,
    ranges: HashMap<NodeId, Range<usize>>,
    /// Inline-level boxes, floats and positioned boxes laid out in this
    /// context (Blink's `IsInLayoutNGInlineFormattingContext`).
    members: HashSet<NodeId>,
    pub(super) items: Vec<Item>,
    pub(super) lines: Vec<Line>,
    /// The first and last item of each DOM node with leaf items.
    node_items: HashMap<NodeId, (usize, usize)>,
}

impl Ifc {
    pub(super) fn new(
        root: NodeId,
        text: Vec<u16>,
        units: Vec<Unit>,
        ranges: HashMap<NodeId, Range<usize>>,
        members: HashSet<NodeId>,
        items: Vec<Item>,
        lines: Vec<Line>,
    ) -> Self {
        let mut node_items: HashMap<NodeId, (usize, usize)> = HashMap::new();
        for (index, item) in items.iter().enumerate() {
            if let Some(node) = item.kind_node() {
                node_items
                    .entry(node)
                    .and_modify(|range| range.1 = index)
                    .or_insert((index, index));
            }
        }
        Self {
            root,
            text,
            units,
            ranges,
            members,
            items,
            lines,
            node_items,
        }
    }

    /// The first and last leaf item a DOM node draws
    /// (`FirstInlineFragmentItemIndex` and its last).
    pub(super) fn items_of(&self, node: NodeId) -> Option<(usize, usize)> {
        self.node_items.get(&node).copied()
    }

    pub(super) fn text(&self) -> &[u16] {
        &self.text
    }

    pub(super) fn has_member(&self, node: NodeId) -> bool {
        self.members.contains(&node) || self.ranges.contains_key(&node)
    }

    pub(super) fn units_of(&self, node: NodeId) -> &[Unit] {
        self.ranges
            .get(&node)
            .and_then(|range| self.units.get(range.clone()))
            .unwrap_or(&[])
    }

    /// The unit holding a node's DOM offset: the last one starting at or
    /// before it (`GetMappingUnitForPosition`).
    fn unit_at(&self, node: NodeId, offset: usize) -> Option<(usize, &Unit)> {
        let range = self.ranges.get(&node)?;
        let units = self.units.get(range.clone())?;
        let after = units.partition_point(|unit| unit.dom.start <= offset);
        let index = after.checked_sub(1)?;
        let unit = &units[index];
        (unit.dom.end >= offset).then_some((range.start + index, unit))
    }

    /// `GetTextContentOffset` for a node and DOM offset.
    pub(super) fn content_offset(&self, node: NodeId, offset: usize) -> Option<usize> {
        let (_, unit) = self.unit_at(node, offset)?;
        Some(unit.dom_to_content(offset))
    }

    /// The node and DOM offset a position maps (`ToNodeOffsetPair`), for
    /// positions the mapping accepts.
    pub(super) fn node_offset(tree: &Tree, position: Position) -> (NodeId, usize) {
        use super::position::Anchor;
        let node = position.node;
        let offset = match position.anchor {
            Anchor::Offset(offset) => offset,
            Anchor::Before => 0,
            Anchor::After | Anchor::AfterChildren if tree.is_text(node) => tree.text_length(node),
            Anchor::After | Anchor::AfterChildren => 1,
        };
        (node, offset)
    }

    /// `IsBeforeNonCollapsedContent`.
    pub(super) fn is_before_non_collapsed(&self, node: NodeId, offset: usize) -> bool {
        self.unit_at(node, offset)
            .is_some_and(|(_, unit)| offset < unit.dom.end && unit.kind != UnitKind::Collapsed)
    }

    /// `IsAfterNonCollapsedContent`.
    pub(super) fn is_after_non_collapsed(&self, node: NodeId, offset: usize) -> bool {
        let Some(before) = offset.checked_sub(1) else {
            return false;
        };
        self.unit_at(node, before)
            .is_some_and(|(_, unit)| offset > unit.dom.start && unit.kind != UnitKind::Collapsed)
    }

    /// The content character before a node's DOM offset.
    pub(super) fn character_before(&self, node: NodeId, offset: usize) -> Option<u16> {
        let content = self.content_offset(node, offset)?;
        self.text.get(content.checked_sub(1)?).copied()
    }

    /// `StartOfNextNonCollapsedContent`: the first DOM offset at or after
    /// `offset` in `node` that renders.
    pub(super) fn start_of_next_non_collapsed(&self, node: NodeId, offset: usize) -> Option<usize> {
        let (index, _) = self.unit_at(node, offset)?;
        self.units[index..]
            .iter()
            .take_while(|unit| unit.node == Some(node))
            .find(|unit| unit.dom.end > offset && unit.kind != UnitKind::Collapsed)
            .map(|unit| offset.max(unit.dom.start))
    }

    /// `EndOfLastNonCollapsedContent`.
    pub(super) fn end_of_last_non_collapsed(&self, node: NodeId, offset: usize) -> Option<usize> {
        let (index, _) = self.unit_at(node, offset)?;
        self.units[..=index]
            .iter()
            .rev()
            .take_while(|unit| unit.node == Some(node))
            .find(|unit| unit.dom.start < offset && unit.kind != UnitKind::Collapsed)
            .map(|unit| offset.min(unit.dom.end))
    }

    /// `GetFirstPosition`: the first DOM point mapping to a content offset,
    /// skipping generated content.
    pub(super) fn first_position(&self, offset: usize) -> Option<(NodeId, usize)> {
        if self.units.last()?.content.end < offset {
            return None;
        }
        let start = self.units.partition_point(|unit| unit.content.end < offset);
        let unit = self.units[start..]
            .iter()
            .take_while(|unit| unit.content.start <= offset)
            .find(|unit| unit.node.is_some())?;
        Some((unit.node?, unit.content_to_first_dom(offset)))
    }

    /// `GetLastPosition`: the last DOM point mapping to a content offset,
    /// skipping generated content.
    pub(super) fn last_position(&self, offset: usize) -> Option<(NodeId, usize)> {
        if self.units.first()?.content.start > offset {
            return None;
        }
        let end = self
            .units
            .partition_point(|unit| unit.content.start <= offset);
        let last = end.checked_sub(1)?;
        if self.units[last].content.end < offset {
            return None;
        }
        let unit = self.units[..=last]
            .iter()
            .rev()
            .take_while(|unit| unit.content.end >= offset)
            .find(|unit| unit.node.is_some())?;
        Some((unit.node?, unit.content_to_last_dom(offset)))
    }

    /// Whether a node's laid-out text has content after collapsing
    /// (`HasNonCollapsedText`).
    pub(super) fn has_non_collapsed_text(&self, node: NodeId) -> bool {
        self.units_of(node)
            .iter()
            .any(|unit| unit.kind != UnitKind::Collapsed && !unit.content.is_empty())
    }

    pub(super) fn line_items(&self, line: usize) -> &[Item] {
        self.lines
            .get(line)
            .and_then(|line| self.items.get(line.items.clone()))
            .unwrap_or(&[])
    }
}

impl Unit {
    /// `ConvertDOMOffsetToTextContent`.
    fn dom_to_content(&self, offset: usize) -> usize {
        if offset == self.dom.start {
            return self.content.start;
        }
        if offset == self.dom.end {
            return self.content.end;
        }
        if self.content.is_empty() {
            return self.content.start;
        }
        (offset - self.dom.start + self.content.start).min(self.content.end)
    }

    /// `ConvertTextContentToFirstDOMOffset`.
    fn content_to_first_dom(&self, offset: usize) -> usize {
        if self.content.is_empty() {
            return self.dom.start;
        }
        if self.kind == UnitKind::Identity {
            return self.dom.start + offset.saturating_sub(self.content.start);
        }
        if offset < self.content.end {
            self.dom.start
        } else {
            self.dom.end
        }
    }

    /// `ConvertTextContentToLastDOMOffset`.
    fn content_to_last_dom(&self, offset: usize) -> usize {
        if self.content.is_empty() {
            return self.dom.end;
        }
        self.content_to_first_dom(offset)
    }
}
