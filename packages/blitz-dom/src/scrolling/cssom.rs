//! CSSOM body/root mappings, scroll options and nested alignment policy.
use super::{
    BaseDocument, NodeId, Point, ScrollAmount, ScrollAnimationState, ScrollBehavior,
    ScrollLogicalPosition, ScrollOverflow, ScrollRequest, ScrollSource, ScrollTarget,
};
use style::values::computed::Overflow;

impl BaseDocument {
    /// CSSOM scroll position: no target names the viewport. Boxless elements
    /// expose zero even when detached layout data retains an earlier offset.
    pub fn cssom_scroll_position(&self, target: Option<NodeId>) -> Point<f64> {
        let Some(id) = target else {
            return self.viewport_scroll();
        };
        if !self.has_rendered_boxes(id) {
            return Point::ZERO;
        }
        let quirks = self.document_metadata(id).quirks;
        if self.try_root_element().is_some_and(|node| node.id == id) {
            return if quirks {
                Point::ZERO
            } else {
                self.viewport_scroll()
            };
        }
        if quirks && self.body_uses_viewport_scroll(id) {
            return self.viewport_scroll();
        }
        self.get_node(id)
            .map_or(Point::ZERO, |node| *node.scroll_offset())
    }

    pub(crate) fn body_uses_viewport_scroll(&self, id: NodeId) -> bool {
        let Some(node) = self.get_node(id) else {
            return false;
        };
        if !node
            .element_data()
            .is_some_and(|e| e.name.local.as_ref() == "body")
        {
            return false;
        }
        // CSSOM View's potentially-scrollable body test requires both the
        // body and its parent to have non-visible, non-clip overflow axes.
        let scrollable = |node: &crate::Node| {
            node.primary_styles().is_some_and(|s| {
                !matches!(s.clone_overflow_x(), Overflow::Visible | Overflow::Clip)
                    && !matches!(s.clone_overflow_y(), Overflow::Visible | Overflow::Clip)
            })
        };
        !(scrollable(node)
            && node
                .flat_parent_id()
                .and_then(|id| self.get_node(id))
                .is_some_and(scrollable))
    }

    /// Converted CSSOM options, with omitted absolute axes retaining their current
    /// position. Layout must be current; this never runs script.
    pub fn scroll_cssom(
        &mut self,
        target: Option<NodeId>,
        x: Option<f64>,
        y: Option<f64>,
        relative: bool,
        behavior: ScrollBehavior,
    ) {
        let Some(id) = target.or_else(|| self.try_root_element().map(|n| n.id)) else {
            return;
        };
        if !self.has_rendered_boxes(id) {
            return;
        }
        let quirks = self.document_metadata(id).quirks;
        let root = self.try_root_element().map(|node| node.id);
        if target.is_some() && quirks && root == Some(id) {
            return;
        }
        let current = self.cssom_scroll_position(target);
        let id = if quirks && self.body_uses_viewport_scroll(id) {
            root.unwrap_or(id)
        } else {
            id
        };
        if relative {
            self.scroll_by(id, x.unwrap_or(0.0), y.unwrap_or(0.0), behavior);
        } else {
            self.scroll_to(id, x.unwrap_or(current.x), y.unwrap_or(current.y), behavior);
        }
    }

    /// Scroll an element to the given absolute scroll offset in CSS pixels.
    ///
    /// Unlike a user-initiated scroll, a programmatic scroll targets exactly one scroller:
    /// scroll the element cannot consume is discarded rather than transferred to an ancestor.
    pub fn scroll_to(&mut self, node_id: NodeId, x: f64, y: f64, behavior: ScrollBehavior) {
        self.scroll_programmatically(node_id, ScrollAmount::To(Point { x, y }), behavior);
    }

    /// Scroll an element by the given relative offset in CSS pixels.
    pub fn scroll_by(&mut self, node_id: NodeId, x: f64, y: f64, behavior: ScrollBehavior) {
        self.scroll_programmatically(node_id, ScrollAmount::By(Point { x, y }), behavior);
    }

    fn scroll_programmatically(
        &mut self,
        node_id: NodeId,
        amount: ScrollAmount,
        behavior: ScrollBehavior,
    ) {
        if self.nodes.get(node_id).is_none() {
            return;
        }

        let mut events = Vec::new();
        self.scroll(
            ScrollRequest {
                target: ScrollTarget::Node(node_id),
                amount,
                overflow: ScrollOverflow::Clamp,
                source: ScrollSource::Programmatic,
                behavior,
                interrupt_animation: true,
            },
            &mut |event| events.push(event),
        );
        for event in events {
            self.queue_scroll_notification(event.target);
        }
    }

    fn aligned_scroll_offset(
        current: f64,
        viewport_size: f64,
        target_start: f64,
        target_size: f64,
        position: ScrollLogicalPosition,
    ) -> f64 {
        let target_end = target_start + target_size;
        match position {
            ScrollLogicalPosition::Start => target_start,
            ScrollLogicalPosition::Center => target_start - (viewport_size - target_size) / 2.0,
            ScrollLogicalPosition::End => target_end - viewport_size,
            ScrollLogicalPosition::Nearest => {
                let viewport_end = current + viewport_size;
                if (target_start >= current && target_end <= viewport_end)
                    || (target_start <= current && target_end >= viewport_end)
                {
                    current
                } else {
                    let start_offset = target_start;
                    let end_offset = target_end - viewport_size;
                    if (start_offset - current).abs() < (end_offset - current).abs() {
                        start_offset
                    } else {
                        end_offset
                    }
                }
            }
        }
    }

    /// Scroll containing boxes from innermost to outermost, then the viewport.
    pub fn scroll_into_view(
        &mut self,
        node_id: NodeId,
        behavior: ScrollBehavior,
        vertical: ScrollLogicalPosition,
        horizontal: ScrollLogicalPosition,
    ) {
        if !self.has_rendered_boxes(node_id) {
            return;
        }
        let mut planned = Vec::new();
        let root = self.try_root_element().map(|node| node.id);
        let mut ancestor = if root == Some(node_id) {
            root
        } else {
            self.get_node(node_id)
                .and_then(|node| node.flat_parent_id())
        };
        while let Some(id) = ancestor {
            ancestor = self.get_node(id).and_then(|node| node.flat_parent_id());
            if !self.has_rendered_boxes(id) {
                continue;
            }
            let root = self.try_root_element().is_some_and(|node| node.id == id);
            let stop = (!root).then_some(id);
            let Some(target) = self.client_bounding_rect_until(node_id, stop) else {
                break;
            };
            let current = self
                .scroll_state(self.canonical_scroll_target(ScrollTarget::Node(id)), true)
                .0;
            let (x, y, width, height) = if root {
                let scale = self.viewport.scale_f64();
                (
                    0.0,
                    0.0,
                    f64::from(self.viewport.window_size.0) / scale,
                    f64::from(self.viewport.window_size.1) / scale,
                )
            } else {
                let Some(rect) = self.client_bounding_rect_until(id, Some(id)) else {
                    continue;
                };
                let metrics = self.element_box_metrics(id);
                (
                    rect.x + metrics.client_left,
                    rect.y + metrics.client_top,
                    metrics.client_width,
                    metrics.client_height,
                )
            };
            let left = Self::aligned_scroll_offset(
                current.x,
                width,
                current.x + target.x - x,
                target.width,
                horizontal,
            );
            let top = Self::aligned_scroll_offset(
                current.y,
                height,
                current.y + target.y - y,
                target.height,
                vertical,
            );
            let scroll_target = self.canonical_scroll_target(ScrollTarget::Node(id));
            if self.should_scroll_smoothly(scroll_target, behavior) {
                self.scroll_to(id, left, top, behavior);
                if let ScrollAnimationState::ScrollTo(animations) = &self.scroll_animation {
                    if let Some(animation) = animations.iter().find(|a| a.target == scroll_target) {
                        let end = animation.end;
                        planned.push((scroll_target, current));
                        self.set_planned_scroll_offset(scroll_target, end);
                    }
                }
            } else {
                self.scroll_to(id, left, top, behavior);
            }
            if root {
                break;
            }
        }
        for (target, offset) in planned {
            self.set_planned_scroll_offset(target, offset);
        }
    }

    // Private projection state while planning nested smooth destinations. The
    // original positions are restored before script or rendering can observe them.
    fn set_planned_scroll_offset(&mut self, target: ScrollTarget, offset: Point<f64>) {
        match target {
            ScrollTarget::Viewport => self.set_viewport_scroll(offset),
            ScrollTarget::Node(id) => {
                if let Some(node) = self.nodes.get_mut(id) {
                    *node.scroll_offset_mut() = offset;
                }
            }
        }
    }
}
