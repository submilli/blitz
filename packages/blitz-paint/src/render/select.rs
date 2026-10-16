//! Paint a `<select>`'s content: a drop-down's label and arrow, or a list
//! box's rows, from the layout blitz-dom built for it.

use super::ElementCx;
use crate::color::ToColorColor as _;
use anyrender::PaintScene;
use blitz_dom::node::{DropDownLayout, ListBoxLayout, SelectContent};
use kurbo::{Affine, BezPath, Cap, Join, Point, Rect, Stroke};
use peniko::{Color, Fill};
use style::computed_values::direction::T as Direction;

/// The background of a selected list box row, as Chrome draws an
/// unfocused list box's selection.
pub(crate) const SELECTED_ROW: Color = Color::from_rgba8(206, 206, 206, 255);
/// The drop-down arrow's size, in CSS px: a chevron this wide and half as
/// tall, stroked this thick.
const ARROW_WIDTH: f64 = 8.0;
const ARROW_STROKE: f64 = 1.5;

impl ElementCx<'_, '_> {
    /// `content` is the content box's origin from the border box's, in
    /// CSS px.
    pub(super) fn draw_select(&self, scene: &mut impl PaintScene, content: Point) {
        let Some(select) = self.element.select_layout.as_deref() else {
            return;
        };
        let size = self.node.final_layout().content_box_size();
        let content_box =
            Rect::from_origin_size(content, (f64::from(size.width), f64::from(size.height)));
        // Huge author lengths leave nothing drawable.
        if !(content_box.x1.is_finite() && content_box.y1.is_finite()) {
            return;
        }
        let rtl = self.style.clone_direction() == Direction::Rtl;
        match &select.content {
            SelectContent::DropDown(drop_down) => {
                self.draw_drop_down(scene, drop_down, content_box, rtl)
            }
            SelectContent::ListBox(list_box) => {
                self.draw_list_box(scene, list_box, content_box, rtl)
            }
        }
    }

    fn draw_drop_down(
        &self,
        scene: &mut impl PaintScene,
        drop_down: &DropDownLayout,
        content: Rect,
        rtl: bool,
    ) {
        let line_top = content.y0 + f64::from(drop_down.line_top(content.height() as f32));
        if let Some(label) = drop_down.label.as_deref() {
            let width = f64::from(label.full_width() / label.scale());
            let inset = f64::from(drop_down.label_inset);
            let x = label_start(content, inset, width, rtl);
            let baseline = line_top + f64::from(drop_down.baseline);
            // A label too long for a sized control stops at the button
            // part, as Chrome clips its inner element.
            let label_area = without_end(content, f64::from(drop_down.button_width), rtl);
            scene.push_clip_layer(self.transform * Affine::scale(self.scale), &label_area);
            self.draw_label(scene, label, x, baseline);
            scene.pop_layer();
        }
        // The chevron centered in the button part.
        let button = f64::from(drop_down.button_width);
        let center_x = if rtl {
            content.x0 + button / 2.0
        } else {
            content.x1 - button / 2.0
        };
        let center_y = line_top + f64::from(drop_down.line_height) / 2.0;
        let mut arrow = BezPath::new();
        arrow.move_to((center_x - ARROW_WIDTH / 2.0, center_y - ARROW_WIDTH / 4.0));
        arrow.line_to((center_x, center_y + ARROW_WIDTH / 4.0));
        arrow.line_to((center_x + ARROW_WIDTH / 2.0, center_y - ARROW_WIDTH / 4.0));
        let stroke = Stroke::new(ARROW_STROKE * self.scale)
            .with_caps(Cap::Round)
            .with_join(Join::Round);
        let color = self.style.clone_color().as_srgb_color();
        let device = Affine::scale(self.scale);
        scene.stroke(&stroke, self.transform * device, color, None, &arrow);
    }

    fn draw_list_box(
        &self,
        scene: &mut impl PaintScene,
        list_box: &ListBoxLayout,
        content: Rect,
        rtl: bool,
    ) {
        let device = Affine::scale(self.scale);
        // Rows take the content box but its scrollbar.
        let rows = without_end(content, f64::from(list_box.scrollbar_width), rtl);
        for row in &list_box.rows {
            let top = content.y0 + f64::from(row.top);
            if top >= content.y1 {
                break;
            }
            if row.selected {
                let bottom = (top + f64::from(row.height)).min(content.y1);
                let background = Rect::new(rows.x0, top, rows.x1, bottom);
                scene.fill(
                    Fill::NonZero,
                    self.transform * device,
                    SELECTED_ROW,
                    None,
                    &background,
                );
            }
            let width = f64::from(row.label.full_width() / row.label.scale());
            let inset = f64::from(row.inset);
            let x = label_start(content, inset, width, rtl);
            self.draw_label(scene, &row.label, x, top + f64::from(row.baseline));
        }
    }

    /// Draw a one-line label with its start at `x` and its baseline at
    /// `baseline`, in CSS px from the border box's origin.
    fn draw_label(
        &self,
        scene: &mut impl PaintScene,
        label: &parley::Layout<blitz_dom::node::TextBrush>,
        x: f64,
        baseline: f64,
    ) {
        let Some(line) = label.lines().next() else {
            return;
        };
        let top = baseline - f64::from(line.metrics().baseline / label.scale());
        let transform = self.transform * Affine::translate((x * self.scale, top * self.scale));
        let mut context = self.context.draw_text_context.borrow_mut();
        crate::text::stroke_text(
            &mut crate::inline_opacity::DirectScene(scene),
            label.lines(),
            self.context.dom,
            transform,
            self.scale,
            self.node.id,
            &mut context,
        );
    }
}

/// Where a label of this width starts, `inset` from the content box's start
/// edge.
fn label_start(content: Rect, inset: f64, width: f64, rtl: bool) -> f64 {
    if rtl {
        content.x1 - inset - width
    } else {
        content.x0 + inset
    }
}

/// The content box without `width` at its end edge.
fn without_end(content: Rect, width: f64, rtl: bool) -> Rect {
    let width = width.min(content.width());
    if rtl {
        Rect {
            x0: content.x0 + width,
            ..content
        }
    } else {
        Rect {
            x1: content.x1 - width,
            ..content
        }
    }
}
