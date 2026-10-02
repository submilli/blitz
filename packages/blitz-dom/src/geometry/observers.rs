//! Script-free geometry used by rendering observers after layout.
use crate::document::{BoundingRect, snap_to_layout_unit};
use crate::{BaseDocument, NodeId};
use style::values::computed::Overflow;

/// A single untransformed fragment, in CSS pixels except for `device`.
/// Non-replaced inline and boxless elements have zero observer dimensions.
#[derive(Clone, Copy, Debug)]
pub struct ResizeGeometry {
    pub content_rect: BoundingRect,
    pub content: (f64, f64),
    pub border: (f64, f64),
    pub device: (f64, f64),
    pub depth: usize,
}

/// Intersection snapshots in viewport CSS coordinates. Invalid explicit-root
/// ancestry yields zero rectangles, distinct from an offscreen eligible target.
#[derive(Clone, Copy, Debug)]
pub struct IntersectionGeometry {
    pub bounds: BoundingRect,
    pub root: BoundingRect,
    pub hit: BoundingRect,
    pub intersects: bool,
    pub ratio: f64,
}

const ZERO: BoundingRect = BoundingRect {
    x: 0.0,
    y: 0.0,
    width: 0.0,
    height: 0.0,
};

impl BaseDocument {
    /// Resolve root eligibility and clip through the containing-block chain.
    /// Percent root margins follow the measured browser's per-axis projection.
    pub fn intersection_observer_geometry(
        &self,
        target: NodeId,
        root: Option<NodeId>,
        margin: [(f64, bool); 4],
    ) -> IntersectionGeometry {
        let empty = IntersectionGeometry {
            bounds: ZERO,
            root: ZERO,
            hit: ZERO,
            intersects: false,
            ratio: 0.0,
        };
        let Some(node) = self.get_node(target) else {
            return empty;
        };
        if !self.has_rendered_boxes(target) {
            return empty;
        }
        let element_root = root.filter(|id| self.get_node(*id).is_some_and(|n| n.is_element()));
        if let Some(root) = root {
            let mut current = node.flat_parent_id();
            let mut found = false;
            while let Some(id) = current {
                if id == root {
                    found = true;
                    break;
                }
                current = self.get_node(id).and_then(|n| n.flat_parent_id());
            }
            if !found {
                return empty;
            }
        }
        let Some(bounds) = self.get_client_bounding_rect(target) else {
            return empty;
        };
        let root_rect = if let Some(root) = element_root {
            let Some(rect) = self.observer_clip_rect(root, false) else {
                return empty;
            };
            rect
        } else {
            let scale = self.viewport().scale_f64();
            BoundingRect {
                x: 0.0,
                y: 0.0,
                width: f64::from(self.viewport().window_size.0) / scale,
                height: f64::from(self.viewport().window_size.1) / scale,
            }
        };
        let basis = [
            root_rect.height,
            root_rect.width,
            root_rect.height,
            root_rect.width,
        ];
        let [t, r, b, l] = std::array::from_fn(|i| {
            if margin[i].1 {
                margin[i].0 * basis[i] / 100.0
            } else {
                margin[i].0
            }
        });
        let root_rect = BoundingRect {
            x: root_rect.x - l,
            y: root_rect.y - t,
            width: root_rect.width + l + r,
            height: root_rect.height + t + b,
        };
        let root_rect = element_root.map_or(root_rect, |id| {
            self.transform_client_rect(id, root_rect, None)
        });
        let mut hit = Some(bounds);
        let mut current = node.containing_block();
        let mut reached_root = element_root.is_none();
        while let Some(id) = current {
            if Some(id) == element_root {
                reached_root = true;
                break;
            }
            let Some(ancestor) = self.get_node(id) else {
                break;
            };
            if let Some(style) = ancestor.primary_styles() {
                let x = style.clone_overflow_x() != Overflow::Visible;
                let y = style.clone_overflow_y() != Overflow::Visible;
                if (x || y)
                    && let Some(clip) = self.observer_clip_rect(id, true)
                {
                    hit = hit.and_then(|rect| clip_axes(rect, clip, x, y));
                }
            }
            current = ancestor.containing_block();
        }
        if !reached_root {
            return empty;
        }
        let hit = hit.and_then(|rect| clip_axes(rect, root_rect, true, true));
        let area = bounds.width * bounds.height;
        let ratio = hit.map_or(0.0, |h| {
            if area > 0.0 {
                (h.width * h.height / area).clamp(0.0, 1.0)
            } else {
                1.0
            }
        });
        IntersectionGeometry {
            bounds,
            root: root_rect,
            hit: hit.unwrap_or(ZERO),
            intersects: hit.is_some(),
            ratio,
        }
    }

    fn observer_clip_rect(&self, id: NodeId, transformed: bool) -> Option<BoundingRect> {
        let node = self.get_node(id)?;
        if !self.has_rendered_boxes(id) {
            return None;
        }
        let style = node.primary_styles()?;
        if style.clone_overflow_x() == Overflow::Visible
            && style.clone_overflow_y() == Overflow::Visible
        {
            return self.client_bounding_rect_until(id, if transformed { None } else { Some(id) });
        }
        let layout = node.unrounded_layout();
        let pos = node.unrounded_absolute_position(
            node.scroll_offset().x as f32,
            node.scroll_offset().y as f32,
        );
        let rect = BoundingRect {
            x: f64::from(pos.x + layout.border.left) - self.geometry_viewport_scroll(id).x,
            y: f64::from(pos.y + layout.border.top) - self.geometry_viewport_scroll(id).y,
            width: f64::from(
                layout.size.width
                    - layout.border.left
                    - layout.border.right
                    - layout.scrollbar_size.width,
            ),
            height: f64::from(
                layout.size.height
                    - layout.border.top
                    - layout.border.bottom
                    - layout.scrollbar_size.height,
            ),
        };
        Some(if transformed {
            self.transform_client_rect(id, rect, None)
        } else {
            rect
        })
    }
}

fn clip_axes(
    a: BoundingRect,
    b: BoundingRect,
    horizontal: bool,
    vertical: bool,
) -> Option<BoundingRect> {
    let x = if horizontal { a.x.max(b.x) } else { a.x };
    let y = if vertical { a.y.max(b.y) } else { a.y };
    let right = if horizontal {
        (a.x + a.width).min(b.x + b.width)
    } else {
        a.x + a.width
    };
    let bottom = if vertical {
        (a.y + a.height).min(b.y + b.height)
    } else {
        a.y + a.height
    };
    (right >= x && bottom >= y).then_some(BoundingRect {
        x,
        y,
        width: right - x,
        height: bottom - y,
    })
}

impl BaseDocument {
    /// Resize Observer box sizes use logical axes and exclude CSS transforms.
    pub fn resize_observer_geometry(&self, id: NodeId) -> ResizeGeometry {
        let mut result = ResizeGeometry {
            content_rect: BoundingRect {
                x: 0.0,
                y: 0.0,
                width: 0.0,
                height: 0.0,
            },
            content: (0.0, 0.0),
            border: (0.0, 0.0),
            device: (0.0, 0.0),
            depth: self.observer_depth(id),
        };
        if !self.has_rendered_boxes(id) || self.inline_fragment_rects(id).is_some() {
            return result;
        }
        let Some(node) = self.get_node(id) else {
            return result;
        };
        let layout = node.unrounded_layout();
        let w = snap_to_layout_unit(f64::from(
            (layout.size.width
                - layout.border.left
                - layout.border.right
                - layout.scrollbar_size.width
                - layout.padding.left
                - layout.padding.right)
                .max(0.0),
        ));
        let h = snap_to_layout_unit(f64::from(
            (layout.size.height
                - layout.border.top
                - layout.border.bottom
                - layout.scrollbar_size.height
                - layout.padding.top
                - layout.padding.bottom)
                .max(0.0),
        ));
        result.content_rect = BoundingRect {
            x: f64::from(layout.padding.left),
            y: f64::from(layout.padding.top),
            width: w,
            height: h,
        };
        let vertical = node
            .primary_styles()
            .is_some_and(|s| s.writing_mode.is_vertical());
        let logical = |w, h| if vertical { (h, w) } else { (w, h) };
        result.content = logical(w, h);
        result.border = logical(
            snap_to_layout_unit(f64::from(layout.size.width)),
            snap_to_layout_unit(f64::from(layout.size.height)),
        );
        let scale = self.viewport().scale_f64();
        result.device = logical((w * scale).round(), (h * scale).round());
        result
    }

    /// Flattened tree depth drives Resize Observer's monotonically deeper rounds.
    pub fn observer_depth(&self, id: NodeId) -> usize {
        let mut depth = 0;
        let mut current = Some(id);
        while let Some(node) = current.and_then(|id| self.get_node(id)) {
            depth += 1;
            current = node.flat_parent_id();
        }
        depth
    }
}
