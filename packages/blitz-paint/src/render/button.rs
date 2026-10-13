//! Paint a button as Chrome's themed button (macOS) when the page has not
//! styled its border or background: Chrome then draws its native
//! appearance in the border box instead of the UA's `2px outset` border.

use super::ElementCx;
use anyrender::PaintScene;
use kurbo::Stroke;
use peniko::{Color, Fill};
use style::computed_value_flags::ComputedValueFlags;

/// Chrome's themed button: a light fill inside a 1px grey border with
/// rounded corners, all in CSS px.
pub(crate) const FILL: Color = Color::from_rgba8(239, 239, 239, 255);
pub(crate) const BORDER: Color = Color::from_rgba8(118, 118, 118, 255);
const BORDER_WIDTH: f64 = 1.0;
const CORNER_RADIUS: f64 = 3.0;

impl ElementCx<'_, '_> {
    /// Whether the element paints as Chrome's themed button rather than its
    /// computed background and border.
    pub(super) fn is_themed_button(&self) -> bool {
        self.node.is_button()
            && !self
                .style
                .flags
                .contains(ComputedValueFlags::HAS_AUTHOR_SPECIFIED_BORDER_BACKGROUND)
    }

    pub(super) fn draw_themed_button(&self, scene: &mut impl PaintScene) {
        let border_box = self.frame.border_box;
        if border_box.is_zero_area() || !(border_box.x1.is_finite() && border_box.y1.is_finite()) {
            return;
        }
        let width = BORDER_WIDTH * self.scale;
        let radius = CORNER_RADIUS * self.scale;
        let fill = border_box.to_rounded_rect(radius);
        scene.fill(Fill::NonZero, self.transform, FILL, None, &fill);
        // Stroked along the middle of its width, inside the border box.
        let inset = width / 2.0;
        let edge = border_box.inset(-inset);
        let stroke = Stroke::new(width);
        let outline = edge.to_rounded_rect((radius - inset).max(0.0));
        scene.stroke(&stroke, self.transform, BORDER, None, &outline);
    }
}
