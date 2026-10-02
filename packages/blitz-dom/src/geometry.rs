//! CSSOM box eligibility and viewport coordinate projection.
use crate::document::{BoundingRect, snap_to_layout_unit};
use crate::{BaseDocument, NodeId};
use kurbo::{Affine, Point};
use style::values::specified::box_::{DisplayInside, DisplayOutside};

impl BaseDocument {
    /// Whether the element generates boxes in the active document after layout.
    /// A descendant of display:none has no boxes even if it retains old layout data.
    pub fn has_rendered_boxes(&self, id: NodeId) -> bool {
        let Some(node) = self.get_node(id) else {
            return false;
        };
        if !node.is_element() || !node.has_boxes() {
            return false;
        }
        let mut current = node;
        loop {
            if current
                .display_style()
                .is_some_and(|d| d.inside() == DisplayInside::None)
            {
                return false;
            }
            let Some(parent) = current.flat_parent_id() else {
                break;
            };
            let Some(node) = self.get_node(parent) else {
                return false;
            };
            current = node;
        }
        true
    }

    // Fixed descendants inherit the viewport coordinate space until an actual
    // transformed containing block intervenes, matching the layout paint tree.
    fn geometry_viewport_scroll(&self, id: NodeId) -> crate::Point<f64> {
        let mut current = self.get_node(id);
        while let Some(node) = current {
            let parent = node.containing_block().and_then(|id| self.get_node(id));
            if node.taffy_position() == taffy::Position::Fixed
                && parent.is_some_and(|cb| {
                    self.try_root_element().is_some_and(|root| root.id == cb.id)
                        && cb.transform().is_none()
                })
            {
                return crate::Point::ZERO;
            }
            current = parent;
        }
        self.viewport_scroll()
    }

    /// Apply each containing box's CSS transform to a viewport-space rectangle.
    /// Keep the corners until the end: successive rotations must not inflate an
    /// intermediate axis-aligned bounding box. Own scroll moves content, not its box.
    fn transform_client_rect(
        &self,
        id: NodeId,
        rect: BoundingRect,
        stop: Option<NodeId>,
    ) -> BoundingRect {
        let mut corners = [
            Point::new(rect.x, rect.y),
            Point::new(rect.x + rect.width, rect.y),
            Point::new(rect.x, rect.y + rect.height),
            Point::new(rect.x + rect.width, rect.y + rect.height),
        ];
        let mut current = Some(id);
        let scale = self.viewport().scale_f64();
        while let Some(node) = current.and_then(|id| self.get_node(id)) {
            if Some(node.id) == stop {
                break;
            }
            if let Some(transform) = node.transform().as_deref() {
                let origin = node.unrounded_absolute_position(
                    node.scroll_offset().x as f32,
                    node.scroll_offset().y as f32,
                );
                let x = f64::from(origin.x) - self.geometry_viewport_scroll(node.id).x;
                let y = f64::from(origin.y) - self.geometry_viewport_scroll(node.id).y;
                let [a, b, c, d, e, f] = transform.as_coeffs();
                let transform = Affine::translate((x, y))
                    * Affine::new([a, b, c, d, e / scale, f / scale])
                    * Affine::translate((-x, -y));
                for corner in &mut corners {
                    *corner = transform * *corner;
                }
            }
            current = node.containing_block();
        }
        let x = corners.iter().map(|p| p.x).fold(f64::INFINITY, f64::min);
        let y = corners.iter().map(|p| p.y).fold(f64::INFINITY, f64::min);
        let right = corners
            .iter()
            .map(|p| p.x)
            .fold(f64::NEG_INFINITY, f64::max);
        let bottom = corners
            .iter()
            .map(|p| p.y)
            .fold(f64::NEG_INFINITY, f64::max);
        BoundingRect {
            x,
            y,
            width: right - x,
            height: bottom - y,
        }
    }
}

#[cfg(test)]
mod tests;

/// Untransformed CSSOM layout metrics. Floating-point values are rounded by the
/// Web IDL adapter; scroll positions retain subpixel precision separately.
#[derive(Default)]
pub struct BoxMetrics {
    pub offset_width: f64,
    pub offset_height: f64,
    pub offset_left: f64,
    pub offset_top: f64,
    pub offset_parent: Option<NodeId>,
    pub client_width: f64,
    pub client_height: f64,
    pub client_left: f64,
    pub client_top: f64,
    pub scroll_width: f64,
    pub scroll_height: f64,
}
impl BaseDocument {
    /// CSSOM View element metrics after the caller flushes style and layout.
    pub fn element_box_metrics(&self, id: NodeId) -> BoxMetrics {
        if !self.has_rendered_boxes(id) {
            return BoxMetrics::default();
        }
        let Some(node) = self.get_node(id) else {
            return BoxMetrics::default();
        };
        let layout = node.unrounded_layout();
        let offset = node.offset_top_left();
        let is_body = node
            .element_data()
            .is_some_and(|e| e.name.local.as_ref() == "body");
        let is_root = self.try_root_element().is_some_and(|n| n.id == id);
        let is_fixed = node
            .primary_styles()
            .is_some_and(|s| s.get_box().position == style::computed_values::position::T::Fixed);
        let parent = if is_root || is_body {
            None
        } else if is_fixed {
            node.containing_block()
                .and_then(|id| self.get_node(id))
                .filter(|cb| {
                    !self.try_root_element().is_some_and(|root| root.id == cb.id)
                        || cb.transform().is_some()
                })
        } else {
            node.offset_parent()
        };
        let mut result = BoxMetrics {
            offset_width: f64::from(layout.size.width),
            offset_height: f64::from(layout.size.height),
            offset_left: if is_body { 0.0 } else { f64::from(offset.x) },
            offset_top: if is_body { 0.0 } else { f64::from(offset.y) },
            offset_parent: parent.map(|n| n.id),
            client_width: f64::from(node.client_width()),
            client_height: f64::from(node.client_height()),
            client_left: f64::from(layout.border.left),
            client_top: f64::from(layout.border.top),
            scroll_width: f64::from(node.scroll_width()),
            scroll_height: f64::from(node.scroll_height()),
        };
        if let Some(rects) = self.inline_fragment_rects(id) {
            result.client_width = 0.0;
            result.client_height = 0.0;
            result.client_left = 0.0;
            result.client_top = 0.0;
            result.scroll_width = 0.0;
            result.scroll_height = 0.0;
            if let Some(first) = rects.first() {
                let left = rects.iter().map(|r| r.x).fold(f64::INFINITY, f64::min);
                let top = rects.iter().map(|r| r.y).fold(f64::INFINITY, f64::min);
                let right = rects
                    .iter()
                    .map(|r| r.x + r.width)
                    .fold(f64::NEG_INFINITY, f64::max);
                let bottom = rects
                    .iter()
                    .map(|r| r.y + r.height)
                    .fold(f64::NEG_INFINITY, f64::max);
                result.offset_width = right - left;
                result.offset_height = bottom - top;
                let root = node
                    .inline_root_ancestor()
                    .expect("inline fragments have an inline root");
                let scrolled = root.unrounded_absolute_position(0.0, 0.0);
                let origin = self.unscrolled_layout_origin(root.id);
                result.offset_left = first.x + self.geometry_viewport_scroll(root.id).x + origin.0
                    - f64::from(scrolled.x);
                result.offset_top = first.y + self.geometry_viewport_scroll(root.id).y + origin.1
                    - f64::from(scrolled.y);
                if let Some(parent) = parent {
                    let static_body = parent
                        .element_data()
                        .is_some_and(|e| e.name.local.as_ref() == "body")
                        && parent.primary_styles().is_some_and(|s| {
                            s.get_box().position == style::computed_values::position::T::Static
                        });
                    if !static_body {
                        let position = self.unscrolled_layout_origin(parent.id);
                        result.offset_left -=
                            position.0 + f64::from(parent.unrounded_layout().border.left);
                        result.offset_top -=
                            position.1 + f64::from(parent.unrounded_layout().border.top);
                    }
                }
            }
        }
        let quirks = self.document_metadata(id).quirks;
        if (is_root && !quirks) || (is_body && quirks) {
            let scale = self.viewport().scale_f64();
            result.client_width = f64::from(self.viewport().window_size.0) / scale;
            result.client_height = f64::from(self.viewport().window_size.1) / scale;
            if !quirks || self.body_uses_viewport_scroll(id) {
                result.scroll_width = result.scroll_width.max(result.client_width);
                result.scroll_height = result.scroll_height.max(result.client_height);
            }
        }
        result
    }
}

impl BaseDocument {
    /// The union of nonempty CSSOM client rectangles, or the first empty one.
    pub fn get_client_bounding_rect(&self, node_id: NodeId) -> Option<BoundingRect> {
        self.client_bounding_rect_until(node_id, None)
    }

    pub(crate) fn client_bounding_rect_until(
        &self,
        node_id: NodeId,
        stop: Option<NodeId>,
    ) -> Option<BoundingRect> {
        let rects = self.node_client_rects_until(node_id, stop);
        let first = rects.first()?;
        let mut nonempty = rects.iter().filter(|r| r.width != 0.0 && r.height != 0.0);
        let Some(initial) = nonempty.next() else {
            return Some(*first);
        };
        let (mut x, mut y, mut right, mut bottom) = (
            initial.x,
            initial.y,
            initial.x + initial.width,
            initial.y + initial.height,
        );
        for rect in nonempty {
            x = x.min(rect.x);
            y = y.min(rect.y);
            right = right.max(rect.x + rect.width);
            bottom = bottom.max(rect.y + rect.height);
        }
        Some(BoundingRect {
            x,
            y,
            width: right - x,
            height: bottom - y,
        })
    }

    /// One rectangle per generated fragment, with CSS transforms applied.
    pub fn node_client_rects(&self, node_id: NodeId) -> Vec<BoundingRect> {
        self.node_client_rects_until(node_id, None)
    }
    fn node_client_rects_until(&self, node_id: NodeId, stop: Option<NodeId>) -> Vec<BoundingRect> {
        if !self.has_rendered_boxes(node_id) {
            return Vec::new();
        }
        if let Some(rects) = self.inline_fragment_rects(node_id) {
            let owner = self
                .get_node(node_id)
                .and_then(|n| n.inline_root_ancestor())
                .map(|n| n.id)
                .unwrap_or(node_id);
            return rects
                .into_iter()
                .map(|rect| self.transform_client_rect(owner, rect, stop))
                .collect();
        }
        let Some(node) = self.get_node(node_id) else {
            return Vec::new();
        };
        let pos = node.unrounded_absolute_position(
            node.scroll_offset().x as f32,
            node.scroll_offset().y as f32,
        );
        let rect = BoundingRect {
            x: snap_to_layout_unit(f64::from(pos.x) - self.geometry_viewport_scroll(node_id).x),
            y: snap_to_layout_unit(f64::from(pos.y) - self.geometry_viewport_scroll(node_id).y),
            width: snap_to_layout_unit(f64::from(node.unrounded_layout().size.width)),
            height: snap_to_layout_unit(f64::from(node.unrounded_layout().size.height)),
        };
        vec![self.transform_client_rect(node_id, rect, stop)]
    }

    /// Computes per-line-box fragment rects for a non-atomic inline element by walking
    /// the containing inline root's text layout. Returns `None` for nodes that have
    /// their own layout box (which should use `get_client_bounding_rect` instead).
    pub fn inline_fragment_rects(&self, node_id: NodeId) -> Option<Vec<BoundingRect>> {
        use parley::PositionedLayoutItem;

        let node = self.get_node(node_id)?;

        // Only non-atomic inline elements lack their own layout box: they are
        // flattened into the containing inline root's text layout as style spans.
        if !node.is_element() || node.flags.is_inline_root() {
            return None;
        }
        let display = node.primary_styles()?.clone_display();
        if !(display.outside() == DisplayOutside::Inline && display.inside() == DisplayInside::Flow)
        {
            return None;
        }

        let inline_root = node.inline_root_ancestor()?;
        let inline_layout = inline_root.element_data()?.inline_layout_data.as_ref()?;
        let layout = &inline_layout.layout;
        let scale = layout.scale() as f64;

        // Walk up the DOM parent chain from `id` to check whether it is (or is
        // inside) the target node, stopping at the inline root.
        let is_in_target = |mut id: NodeId| -> bool {
            loop {
                if id == node_id {
                    return true;
                }
                if id == inline_root.id {
                    return false;
                }
                match self.get_node(id).and_then(|n| n.parent) {
                    Some(parent) => id = parent,
                    None => return false,
                }
            }
        };

        // Fragment rects are relative to the inline root's content box.
        let root_layout = inline_root.unrounded_layout();
        let root_pos = inline_root.unrounded_absolute_position(0.0, 0.0);
        let origin_x = root_pos.x as f64
            + (root_layout.padding.left + root_layout.border.left) as f64
            - self.geometry_viewport_scroll(inline_root.id).x;
        let origin_y = root_pos.y as f64
            + (root_layout.padding.top + root_layout.border.top) as f64
            - self.geometry_viewport_scroll(inline_root.id).y;

        let mut rects: Vec<BoundingRect> = Vec::new();
        for line in layout.lines() {
            let line_metrics = line.metrics();
            // Union all of the target's fragments on this line into a single rect
            let mut line_rect: Option<(f64, f64, f64, f64)> = None;
            let mut add = |x0: f64, y0: f64, x1: f64, y1: f64| {
                line_rect = Some(match line_rect {
                    Some((lx0, ly0, lx1, ly1)) => {
                        (lx0.min(x0), ly0.min(y0), lx1.max(x1), ly1.max(y1))
                    }
                    None => (x0, y0, x1, y1),
                });
            };

            for item in line.items() {
                match item {
                    PositionedLayoutItem::GlyphRun(glyph_run) => {
                        if !is_in_target(glyph_run.style().brush.id) {
                            continue;
                        }
                        let run = glyph_run.run();
                        let hanging = f64::from(line_metrics.hanging_advance);
                        let line_start = f64::from(line_metrics.offset);
                        let line_end = line_start + f64::from(line_metrics.advance);
                        let (visible_start, visible_end) = if run.is_rtl() {
                            (line_start + hanging, line_end)
                        } else {
                            (line_start, line_end - hanging)
                        };
                        let x0 = f64::from(glyph_run.offset()).max(visible_start);
                        let x1 = f64::from(glyph_run.offset() + glyph_run.advance())
                            .min(visible_end)
                            .max(x0);
                        // CSS inline rectangles use the font box, excluding leading
                        // and collapsed end-of-line whitespace (CSSOM View §6).
                        let metrics = run.font_metrics();
                        let y0 = f64::from(glyph_run.baseline() - metrics.ascent.round());
                        let y1 = f64::from(glyph_run.baseline() + metrics.descent.round());
                        add(x0, y0, x1, y1);
                    }
                    PositionedLayoutItem::InlineBox(inline_box) => {
                        if !is_in_target(NodeId::from_u64(inline_box.id)) {
                            continue;
                        }
                        let x0 = inline_box.x as f64;
                        let y0 = inline_box.y as f64;
                        add(
                            x0,
                            y0,
                            x0 + inline_box.width as f64,
                            y0 + inline_box.height as f64,
                        );
                    }
                }
            }

            if let Some((x0, y0, x1, y1)) = line_rect {
                rects.push(BoundingRect {
                    x: snap_to_layout_unit(origin_x + x0 / scale),
                    y: snap_to_layout_unit(origin_y + y0 / scale),
                    width: snap_to_layout_unit((x1 - x0) / scale),
                    height: snap_to_layout_unit((y1 - y0) / scale),
                });
            }
        }

        Some(rects)
    }
}

impl BaseDocument {
    fn unscrolled_layout_origin(&self, id: NodeId) -> (f64, f64) {
        let mut current = Some(id);
        let (mut x, mut y) = (0.0, 0.0);
        while let Some(node) = current.and_then(|id| self.get_node(id)) {
            x += f64::from(node.unrounded_layout().location.x);
            y += f64::from(node.unrounded_layout().location.y);
            current = node.containing_block();
        }
        (x, y)
    }
}
