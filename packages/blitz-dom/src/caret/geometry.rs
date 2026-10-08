//! Caret edges of laid-out clusters and inline boxes, in document coordinates.

use crate::node::{Node, TextBrush};
use parley::{Layout, PositionedLayoutItem};
use std::collections::HashMap;
use std::ops::Range;

/// One cluster's horizontal extent on its line.
#[derive(Clone, Copy, Debug)]
struct ClusterEdge {
    start: usize,
    end: usize,
    left: f32,
    right: f32,
    rtl: bool,
}

/// An inline box's horizontal extent on its line.
#[derive(Clone, Copy, Debug)]
pub(super) struct BoxEdge {
    pub(super) left: f32,
    pub(super) right: f32,
    pub(super) line: u32,
}

/// The leading and trailing caret edges of a run of layout text.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Edges {
    pub(super) leading: f32,
    pub(super) trailing: f32,
    pub(super) line: u32,
}

/// Cluster and inline-box edges of one Parley layout, offset to the
/// document position of its content box.
pub(super) struct LayoutGeometry {
    clusters: Vec<ClusterEdge>,
    boxes: HashMap<u64, BoxEdge>,
    /// Each line's text range and start edge. A line holding only a forced
    /// break may have no positioned cluster.
    lines: Vec<(Range<usize>, f32)>,
}

impl LayoutGeometry {
    /// Geometry of `layout`, which `node` lays out in its content box.
    pub(super) fn new(layout: &Layout<TextBrush>, node: &Node) -> Self {
        let origin = content_origin(node);
        let scale = if layout.scale() > 0.0 {
            layout.scale()
        } else {
            1.0
        };
        let x = |value: f32| origin.0 + value / scale;
        let mut clusters = Vec::new();
        let mut boxes = HashMap::new();
        let mut lines = Vec::new();
        for (index, line) in layout.lines().enumerate() {
            let line_index = u32::try_from(index).unwrap_or(u32::MAX);
            lines.push((line.text_range(), x(line.metrics().offset)));
            for item in line.items() {
                match item {
                    PositionedLayoutItem::GlyphRun(glyphs) => {
                        let run = glyphs.run();
                        let mut left = glyphs.offset();
                        for cluster in run.visual_clusters() {
                            let right = left + cluster.advance();
                            let range = cluster.text_range();
                            clusters.push(ClusterEdge {
                                start: range.start,
                                end: range.end,
                                left: x(left),
                                right: x(right),
                                rtl: cluster.is_rtl(),
                            });
                            left = right;
                        }
                    }
                    PositionedLayoutItem::InlineBox(inline) => {
                        let edge = BoxEdge {
                            left: x(inline.x),
                            right: x(inline.x + inline.width),
                            line: line_index,
                        };
                        boxes.insert(inline.id, edge);
                    }
                }
            }
        }
        clusters.sort_by_key(|cluster| cluster.start);
        Self {
            clusters,
            boxes,
            lines,
        }
    }

    /// Edges of the text between two layout bytes. Text that Parley did not
    /// position, such as an empty line's break, starts at its line's edge.
    pub(super) fn text(&self, start: usize, end: usize) -> Edges {
        let line = self.line(start);
        let first = self.containing(start).filter(|c| start < c.end);
        let last = self
            .containing(end.saturating_sub(1).max(start))
            .filter(|c| c.end > start);
        let edge = self.lines.get(line as usize).map_or(0.0, |(_, x)| *x);
        Edges {
            leading: first.map_or(edge, ClusterEdge::leading),
            trailing: last.map_or(edge, ClusterEdge::trailing),
            line,
        }
    }

    /// The line holding a layout byte; the end of the text is on the last.
    fn line(&self, byte: usize) -> u32 {
        let index = self
            .lines
            .partition_point(|(range, _)| range.end <= byte)
            .min(self.lines.len().saturating_sub(1));
        u32::try_from(index).unwrap_or(u32::MAX)
    }

    pub(super) fn inline_box(&self, id: u64) -> Option<BoxEdge> {
        self.boxes.get(&id).copied()
    }

    fn containing(&self, byte: usize) -> Option<&ClusterEdge> {
        let after = self
            .clusters
            .partition_point(|cluster| cluster.start <= byte);
        after.checked_sub(1).map(|index| &self.clusters[index])
    }
}

impl ClusterEdge {
    fn leading(&self) -> f32 {
        if self.rtl { self.right } else { self.left }
    }

    fn trailing(&self) -> f32 {
        if self.rtl { self.left } else { self.right }
    }
}

/// The document position of a box's content edge, where its layout starts.
fn content_origin(node: &Node) -> (f32, f32) {
    let layout = node.final_layout();
    let position = node.absolute_position(0.0, 0.0);
    (
        position.x + layout.border.left + layout.padding.left,
        position.y + layout.border.top + layout.padding.top,
    )
}
