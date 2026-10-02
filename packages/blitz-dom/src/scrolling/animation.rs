//! Per-target smooth scrolling and input fling progression on the injected clock.
use super::{
    BaseDocument, Point, ScrollAnimationState, ScrollBehavior, ScrollTarget, ScrollToState,
};

/// Cubic ease-in-out easing function, mapping a normalised time `t` in `[0, 1]`
/// to an eased progress value in `[0, 1]`. Used to give smooth scrolls a natural
/// acceleration/deceleration curve.
fn ease_in_out_cubic(t: f64) -> f64 {
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        let f = 2.0 * t - 2.0;
        1.0 + (f * f * f) / 2.0
    }
}

impl BaseDocument {
    /// Start a smooth (animated) scroll towards the given absolute scroll offset. The
    /// animation is advanced each frame in [`BaseDocument::resolve_scroll_animation`].
    pub(super) fn start_scroll_animation(&mut self, target: ScrollTarget, end: Point<f64>) {
        let start = self.scroll_state(target, true).0;

        let start_time = self.clock.now_ms().floor();

        if start == end {
            return;
        }
        let animation = ScrollToState {
            target,
            start,
            end,
            start_time,
            duration: Self::SMOOTH_SCROLL_DURATION_MS,
        };
        if let ScrollAnimationState::ScrollTo(animations) = &mut self.scroll_animation {
            animations.retain(|animation| animation.target != target);
            animations.push(animation);
        } else {
            self.scroll_animation = ScrollAnimationState::ScrollTo(vec![animation]);
        }

        // Ensure the frame loop runs so the animation is driven to completion.
        self.shell_provider.request_redraw();
    }

    pub(super) fn should_scroll_smoothly(
        &self,
        target: ScrollTarget,
        behavior: ScrollBehavior,
    ) -> bool {
        match behavior {
            ScrollBehavior::Auto => {
                let styled_node = match target {
                    ScrollTarget::Node(node_id) => self.nodes.get(node_id),
                    ScrollTarget::Viewport => self.try_root_element(),
                };
                styled_node.is_some_and(|node| {
                    node.primary_styles().is_some_and(|style| {
                        style.clone_scroll_behavior()
                            == style::computed_values::scroll_behavior::T::Smooth
                    })
                })
            }
            ScrollBehavior::Instant => false,
            ScrollBehavior::Smooth => true,
        }
    }

    pub fn resolve_scroll_animation(&mut self) {
        match &mut self.scroll_animation {
            ScrollAnimationState::Fling(fling_state) => {
                let time_ms = self.clock.now_ms().floor();

                let time_diff_ms = time_ms - fling_state.last_seen_time;

                // 0.95 @ 60fps normalized to actual frame times
                let deceleration = 1.0 - ((0.05 / 16.66666) * time_diff_ms);

                fling_state.x_velocity *= deceleration;
                fling_state.y_velocity *= deceleration;
                fling_state.last_seen_time = time_ms;
                let fling_state = fling_state.clone();

                let dx = fling_state.x_velocity * time_diff_ms;
                let dy = fling_state.y_velocity * time_diff_ms;

                self.scroll_chain_by(Some(fling_state.target), dx, dy, &mut |_| {});
                if fling_state.x_velocity.abs() < 0.1 && fling_state.y_velocity.abs() < 0.1 {
                    self.scroll_animation = ScrollAnimationState::None;
                }
            }
            ScrollAnimationState::ScrollTo(_) => {
                let ScrollAnimationState::ScrollTo(mut animations) =
                    std::mem::replace(&mut self.scroll_animation, ScrollAnimationState::None)
                else {
                    unreachable!("matched smooth scroll animations");
                };
                let time_ms = self.clock.now_ms().floor();
                animations.retain(|animation| {
                    if let ScrollTarget::Node(id) = animation.target {
                        if !self.has_rendered_boxes(id) {
                            return false;
                        }
                    }
                    let progress = if animation.duration <= 0.0 {
                        1.0
                    } else {
                        ((time_ms - animation.start_time) / animation.duration).clamp(0.0, 1.0)
                    };
                    let eased = ease_in_out_cubic(progress);
                    let offset = Point {
                        x: animation.start.x + (animation.end.x - animation.start.x) * eased,
                        y: animation.start.y + (animation.end.y - animation.start.y) * eased,
                    };
                    let max = self.scroll_state(animation.target, true).1;
                    let offset = Point {
                        x: offset.x.clamp(0.0, max.x),
                        y: offset.y.clamp(0.0, max.y),
                    };
                    let mut event = None;
                    self.write_scroll_offset(animation.target, offset, &mut |value| {
                        event = Some(value)
                    });
                    if let Some(event) = event {
                        self.queue_scroll_notification(event.target);
                    }
                    progress < 1.0
                });
                if !animations.is_empty() {
                    self.scroll_animation = ScrollAnimationState::ScrollTo(animations);
                }
            }
            ScrollAnimationState::None => {
                // Do nothing
            }
        }
    }
}
