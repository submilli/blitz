use super::ElementCx;
use blitz_dom::{ShapeReferenceBox as ReferenceBox, basic_shape_to_path};
use kurbo::{BezPath, Rect, Shape};
use style::values::computed::basic_shape::ClipPath;
use style::values::generics::basic_shape::{ShapeBox, ShapeGeometryBox};

impl ElementCx<'_, '_> {
    /// Compute the rectangle (in scaled pixels, relative to the border box origin) described by
    /// the CSS 2 `clip` property, if it applies. `clip` only applies to absolutely positioned
    /// elements; `auto` components resolve to the corresponding border box edge.
    pub(super) fn css_clip_rect(&self) -> Option<Rect> {
        use style::computed_values::position::T as Position;
        use style::values::computed::LengthOrAuto;
        use style::values::generics::ClipRectOrAuto;

        if !matches!(
            self.style.get_box().position,
            Position::Absolute | Position::Fixed
        ) {
            return None;
        }
        let ClipRectOrAuto::Rect(rect) = &self.style.get_effects().clip else {
            return None;
        };

        let border_box = self.frame.border_box;
        let resolve = |component: &LengthOrAuto, auto: f64| match component {
            LengthOrAuto::Auto => auto,
            LengthOrAuto::LengthPercentage(length) => length.px() as f64 * self.scale,
        };
        let x0 = resolve(&rect.left, 0.0);
        let y0 = resolve(&rect.top, 0.0);
        let x1 = resolve(&rect.right, border_box.width());
        let y1 = resolve(&rect.bottom, border_box.height());

        Some(Rect::new(x0, y0, x1.max(x0), y1.max(y0)))
    }

    /// Compute the clip-path BezPath (if any) for this element.
    /// Returns `None` if clip-path is `none` or unsupported.
    pub(super) fn clip_path_shape(&self) -> Option<BezPath> {
        let clip_path = self.style.clone_clip_path();
        match clip_path {
            ClipPath::None => None,
            ClipPath::Url(_) => {
                // URL references (e.g. `clip-path: url(#myClip)`) are not yet supported
                None
            }
            ClipPath::Shape(basic_shape, geometry_box) => {
                let reference_box = self.resolve_geometry_box(&geometry_box);
                basic_shape_to_path(&basic_shape, reference_box)
            }
            ClipPath::Box(geometry_box) => {
                let reference_box = self.resolve_geometry_box(&geometry_box);
                Some(Rect::from(reference_box).into_path(0.1))
            }
        }
    }

    /// Resolve a ShapeGeometryBox to a concrete rectangle (x, y, width, height) in scaled pixels
    ///
    /// For SVG elements without associated CSS layout box, the used value for content-box and padding-box is fill-box and for border-box and margin-box is stroke-box.
    /// For elements with associated CSS layout box, the used value for fill-box is content-box and for stroke-box and view-box is border-box.
    fn resolve_geometry_box(&self, geometry_box: &ShapeGeometryBox) -> ReferenceBox {
        match geometry_box {
            ShapeGeometryBox::ElementDependent
            | ShapeGeometryBox::StrokeBox
            | ShapeGeometryBox::ViewBox
            | ShapeGeometryBox::ShapeBox(ShapeBox::BorderBox) => ReferenceBox {
                x: 0.0,
                y: 0.0,
                width: self.frame.border_box.width() / self.scale,
                height: self.frame.border_box.height() / self.scale,
            },
            ShapeGeometryBox::ShapeBox(ShapeBox::PaddingBox) => ReferenceBox {
                x: self.frame.border_width.x0 / self.scale,
                y: self.frame.border_width.y0 / self.scale,
                width: self.frame.padding_box.width() / self.scale,
                height: self.frame.padding_box.height() / self.scale,
            },
            ShapeGeometryBox::FillBox | ShapeGeometryBox::ShapeBox(ShapeBox::ContentBox) => {
                ReferenceBox {
                    x: (self.frame.border_width.x0 + self.frame.padding_width.x0) / self.scale,
                    y: (self.frame.border_width.y0 + self.frame.padding_width.y0) / self.scale,
                    width: self.frame.content_box.width() / self.scale,
                    height: self.frame.content_box.height() / self.scale,
                }
            }
            ShapeGeometryBox::ShapeBox(ShapeBox::MarginBox) => {
                // Margin box is not tracked in CssBox, fall back to border box
                ReferenceBox {
                    x: 0.0,
                    y: 0.0,
                    width: self.frame.border_box.width() / self.scale,
                    height: self.frame.border_box.height() / self.scale,
                }
            }
        }
    }
}
