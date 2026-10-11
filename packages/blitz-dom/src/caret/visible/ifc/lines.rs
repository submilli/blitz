//! Line boxes and their leaf items, from the lines Parley laid out.
//!
//! Text items group a node's clusters on a line, as Blink's fragment items
//! do; a forced break is an item of its own, and white space hanging at a
//! soft wrap belongs to no item. Coordinates are in the document.

use super::build::{Mapping, SourceKind};
use super::{Item, ItemKind, Line};
use crate::NodeId;
use crate::caret::geometry::{content_origin, content_width};
use crate::node::{Node, TextLayout};
use parley::{BreakReason, InlineBoxKind, PositionedLayoutItem};
use std::collections::HashSet;
use std::ops::Range;

/// A text item being collected: its clusters by content range, with their
/// leading and trailing caret edges, and the content range they span.
struct Pending {
    kind: ItemKind,
    rtl: bool,
    clusters: Vec<(Range<usize>, f32, f32)>,
    content: Range<usize>,
    left: f32,
    right: f32,
}

pub(super) fn build(
    doc: &crate::BaseDocument,
    node: &Node,
    layout: &TextLayout,
    mapping: &Mapping,
) -> (Vec<Item>, Vec<Line>) {
    let frame = Frame::new(doc, node, layout, mapping);
    let mut per_line: Vec<Vec<Item>> = Vec::new();
    let mut lines = Vec::new();
    let line_count = layout.layout.len();
    for (index, line) in layout.layout.lines().enumerate() {
        let mut items = Vec::new();
        let mut pending: Option<Pending> = None;
        // A glyph run is a run's slice in one style; its clusters are the
        // whole run's, laid out once from its first slice.
        let mut runs_seen = HashSet::new();
        for item in line.items() {
            match item {
                PositionedLayoutItem::GlyphRun(glyphs) => {
                    let run = glyphs.run();
                    if !runs_seen.insert(run.index()) {
                        continue;
                    }
                    frame.glyph_run_items(&glyphs, index, &mut pending, &mut items);
                }
                PositionedLayoutItem::InlineBox(inline) => {
                    flush(&mut pending, index, &mut items);
                    items.extend(frame.inline_box_item(&inline, index));
                }
            }
        }
        flush(&mut pending, index, &mut items);
        let soft_wrap = index + 1 < line_count
            && matches!(
                line.break_reason(),
                BreakReason::Regular | BreakReason::Emergency
            );
        if soft_wrap {
            drop_hanging_space(&mut items, mapping);
        }
        lines.push(frame.line_box(line.metrics(), &items, soft_wrap));
        per_line.push(items);
    }
    add_missing_breaks(layout, mapping, &mut per_line, &lines);
    for (items, line) in per_line.iter_mut().zip(&lines) {
        place_breaks_at_logical_end(items, line.rtl);
    }
    let mut items = Vec::new();
    for (line, line_items) in lines.iter_mut().zip(per_line) {
        let start = items.len();
        items.extend(line_items);
        line.items = start..items.len();
    }
    (items, lines)
}

/// An inline formatting context's layout, placed in the document.
struct Frame<'a> {
    doc: &'a crate::BaseDocument,
    node: &'a Node,
    layout: &'a TextLayout,
    mapping: &'a Mapping,
    origin: (f32, f32),
    scale: f32,
}

impl<'a> Frame<'a> {
    fn new(
        doc: &'a crate::BaseDocument,
        node: &'a Node,
        layout: &'a TextLayout,
        mapping: &'a Mapping,
    ) -> Self {
        let scale = layout.layout.scale();
        Self {
            doc,
            node,
            layout,
            mapping,
            origin: content_origin(node),
            scale: if scale > 0.0 { scale } else { 1.0 },
        }
    }

    fn x(&self, value: f32) -> f32 {
        self.origin.0 + value / self.scale
    }

    fn y(&self, value: f32) -> f32 {
        self.origin.1 + value / self.scale
    }

    /// Group a glyph run's clusters into text items: adjacent clusters of
    /// one node in one direction join, and a forced break stands alone.
    fn glyph_run_items(
        &self,
        glyphs: &parley::GlyphRun<'_, crate::node::TextBrush>,
        line: usize,
        pending: &mut Option<Pending>,
        items: &mut Vec<Item>,
    ) {
        let mut left = glyphs.offset();
        for cluster in glyphs.run().visual_clusters() {
            let right = left + cluster.advance();
            let edges = (self.x(left), self.x(right));
            left = right;
            let Some((content, source)) =
                cluster_content(self.layout, self.mapping, cluster.text_range())
            else {
                continue;
            };
            let rtl = cluster.is_rtl();
            let (lead, trail) = if rtl { (edges.1, edges.0) } else { edges };
            let line_break = source.kind == SourceKind::Break;
            let kind = ItemKind::Text {
                node: (!source.generated).then_some(source.owner),
                line_break,
            };
            let owner = (source.owner, source.generated);
            let joins = pending.as_ref().is_some_and(|p| {
                p.kind == kind
                    && !line_break
                    && p.rtl == rtl
                    && adjacent(p, &content, rtl)
                    && same_owner(p, owner, self.mapping)
            });
            if !joins {
                flush(pending, line, items);
                *pending = Some(Pending {
                    kind,
                    rtl,
                    clusters: Vec::new(),
                    content: content.clone(),
                    left: edges.0,
                    right: edges.1,
                });
            }
            if let Some(p) = pending {
                p.content = p.content.start.min(content.start)..p.content.end.max(content.end);
                p.clusters.push((content, lead, trail));
                p.left = p.left.min(edges.0);
                p.right = p.right.max(edges.1);
            }
            if line_break {
                flush(pending, line, items);
            }
        }
    }

    /// The item of an inline box on a line: an atomic inline, a generated
    /// box, or a float.
    fn inline_box_item(&self, inline: &parley::PositionedInlineBox, line: usize) -> Option<Item> {
        let id = NodeId::from_u64(inline.id);
        match inline.kind {
            InlineBoxKind::CustomOutOfFlow => return float_item(self.doc, self.mapping, id, line),
            InlineBoxKind::InFlow => {}
            _ => return None,
        }
        let offset = *self.mapping.atoms.get(&id)?;
        // A generated box (a pseudo-element's) is a leaf without a DOM
        // node, like generated text.
        let generated = self
            .mapping
            .sources
            .get(offset)
            .is_some_and(|s| s.generated);
        let kind = if generated {
            ItemKind::Text {
                node: None,
                line_break: false,
            }
        } else {
            ItemKind::Atomic(id)
        };
        let (left, right) = (self.x(inline.x), self.x(inline.x + inline.width));
        Some(Item {
            kind,
            line,
            content: offset..offset + 1,
            left,
            right,
            rtl: self.layout.layout.is_rtl(),
            carets: if generated {
                vec![left, right]
            } else {
                Vec::new()
            },
        })
    }

    /// A line box spanning its leaves and Parley's line extent.
    fn line_box(&self, metrics: &parley::LineMetrics, items: &[Item], soft_wrap: bool) -> Line {
        let leaves = || items.iter().filter(|item| item.is_leaf());
        let left = leaves()
            .map(|item| item.left)
            .fold(self.x(metrics.inline_min_coord), f32::min);
        let right = leaves()
            .map(|item| item.right)
            .fold(self.x(metrics.inline_max_coord), f32::max);
        let rtl = self.layout.layout.is_rtl();
        Line {
            items: 0..0,
            top: self.y(metrics.block_min_coord),
            bottom: self.y(metrics.block_max_coord),
            left,
            right,
            container_left: self.origin.0,
            container_right: self.origin.0 + content_width(self.node),
            align_right: aligns_right(self.node, rtl),
            rtl,
            soft_wrap,
        }
    }
}

/// The content range and source of a cluster's characters, or `None` when
/// none of them is in the text content (collapsed or removed white space).
fn cluster_content(
    layout: &TextLayout,
    mapping: &Mapping,
    range: Range<usize>,
) -> Option<(Range<usize>, super::build::Source)> {
    let text = layout.text.get(range.clone())?;
    let mut content: Option<Range<usize>> = None;
    for (byte, ch) in text.char_indices() {
        let Some(&offset) = mapping.bytes.get(&(range.start + byte)) else {
            continue;
        };
        let end = offset + ch.len_utf16();
        content = Some(match content {
            Some(found) => found.start.min(offset)..found.end.max(end),
            None => offset..end,
        });
    }
    let content = content?;
    let source = *mapping.sources.get(content.start)?;
    Some((content, source))
}

fn adjacent(pending: &Pending, content: &Range<usize>, rtl: bool) -> bool {
    if rtl {
        pending.content.start == content.end
    } else {
        pending.content.end == content.start
    }
}

fn same_owner(pending: &Pending, owner: (NodeId, bool), mapping: &Mapping) -> bool {
    pending
        .clusters
        .first()
        .and_then(|c| mapping.sources.get(c.0.start))
        .is_some_and(|source| (source.owner, source.generated) == owner)
}

/// Close the pending text item, computing its caret positions.
fn flush(pending: &mut Option<Pending>, line: usize, items: &mut Vec<Item>) {
    let Some(mut p) = pending.take() else {
        return;
    };
    p.clusters.sort_by_key(|c| c.0.start);
    let (Some(first), Some(last)) = (p.clusters.first(), p.clusters.last()) else {
        return;
    };
    let content = first.0.start..last.0.end;
    let mut carets = vec![first.1; content.len() + 1];
    for (range, lead, trail) in &p.clusters {
        for offset in range.clone() {
            carets[offset - content.start] = *lead;
        }
        carets[range.end - content.start] = *trail;
    }
    items.push(Item {
        kind: p.kind,
        line,
        content,
        left: p.left,
        right: p.right,
        rtl: p.rtl,
        carets,
    });
}

/// A float a line holds, at its box in the document.
fn float_item(
    doc: &crate::BaseDocument,
    mapping: &Mapping,
    id: NodeId,
    line: usize,
) -> Option<Item> {
    let offset = *mapping.floats.get(&id)?;
    let node = doc.get_node(id)?;
    let left = node.unrounded_absolute_position(0.0, 0.0).x;
    let width = node.final_layout().size.width;
    Some(Item {
        kind: ItemKind::Float(id),
        line,
        content: offset..offset,
        left,
        right: left + width,
        rtl: false,
        carets: Vec::new(),
    })
}

/// Collapsible white space ending a soft-wrapped line hangs outside the
/// line's items, though it stays in the text content.
fn drop_hanging_space(items: &mut Vec<Item>, mapping: &Mapping) {
    while let Some(index) = items.iter().rposition(Item::is_leaf) {
        let item = &mut items[index];
        if !matches!(
            item.kind,
            ItemKind::Text {
                line_break: false,
                ..
            }
        ) {
            return;
        }
        let Some(last) = item.content.end.checked_sub(1) else {
            return;
        };
        let hangs = mapping
            .sources
            .get(last)
            .is_some_and(|s| s.kind == SourceKind::Text { collapsible: true });
        if !hangs {
            return;
        }
        item.content.end = last;
        item.carets.pop();
        if item.content.is_empty() {
            items.remove(index);
            continue;
        }
        if let Some(&end) = item.carets.last() {
            if item.rtl {
                item.left = end;
            } else {
                item.right = end;
            }
        }
    }
}

/// A forced break Parley positioned no cluster for still ends its line.
fn add_missing_breaks(
    layout: &TextLayout,
    mapping: &Mapping,
    per_line: &mut [Vec<Item>],
    lines: &[Line],
) {
    let covered: HashSet<usize> = per_line
        .iter()
        .flatten()
        .filter(|item| item.is_line_break())
        .map(|item| item.content.start)
        .collect();
    let mut missing: Vec<(usize, usize)> = mapping
        .bytes
        .iter()
        .filter(|&(_, offset)| !covered.contains(offset))
        .filter(|&(_, &offset)| {
            mapping
                .sources
                .get(offset)
                .is_some_and(|s| s.kind == SourceKind::Break)
        })
        .map(|(&byte, &offset)| (byte, offset))
        .collect();
    missing.sort_unstable();
    // Lines hold the text in order, so one cursor finds each break's line.
    let line_text: Vec<Range<usize>> = layout
        .layout
        .lines()
        .map(|line| line.text_range())
        .collect();
    let mut index = 0;
    for (byte, offset) in missing {
        let Some(source) = mapping.sources.get(offset) else {
            continue;
        };
        while line_text.get(index).is_some_and(|text| text.end <= byte) {
            index += 1;
        }
        if !line_text
            .get(index)
            .is_some_and(|text| text.contains(&byte))
        {
            continue;
        }
        let Some(items) = per_line.get_mut(index) else {
            continue;
        };
        let x = items.iter().rfind(|item| item.is_leaf()).map_or_else(
            || lines[index].left,
            |item| if item.rtl { item.left } else { item.right },
        );
        items.push(Item {
            kind: ItemKind::Text {
                node: (!source.generated).then_some(source.owner),
                line_break: true,
            },
            line: index,
            content: offset..offset + 1,
            left: x,
            right: x,
            rtl: lines[index].rtl,
            carets: vec![x, x],
        });
    }
}

/// A forced break takes its paragraph's level, so bidi reordering puts it
/// at the line's logical end: after the leaves of a left-to-right
/// paragraph, before those of a right-to-left one.
fn place_breaks_at_logical_end(items: &mut Vec<Item>, rtl: bool) {
    let Some(index) = items.iter().position(Item::is_line_break) else {
        return;
    };
    let mut item = items.remove(index);
    let leaves = || items.iter().filter(|item| item.is_leaf());
    let x = if rtl {
        leaves().map(|item| item.left).reduce(f32::min)
    } else {
        leaves().map(|item| item.right).reduce(f32::max)
    }
    .unwrap_or(item.left);
    item.left = x;
    item.right = x;
    item.rtl = rtl;
    item.carets = vec![x, x];
    if rtl {
        items.insert(0, item);
    } else {
        items.push(item);
    }
}

/// Whether a block's text aligns to the right edge of its lines.
fn aligns_right(node: &Node, rtl: bool) -> bool {
    use parley::Alignment;
    let align = node.primary_styles().map_or(Alignment::Start, |s| {
        crate::stylo_to_parley::text_align(s.clone_text_align())
    });
    match align {
        Alignment::Right => true,
        Alignment::Start | Alignment::Justify => rtl,
        Alignment::End => !rtl,
        Alignment::Left | Alignment::Center => false,
    }
}
