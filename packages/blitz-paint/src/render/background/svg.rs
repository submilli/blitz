//! SVG images in background layers, sized by the CSS default sizing
//! algorithm and drawn into their tile.

use super::{
    BackgroundSizeComputeMode, ImageLayerStyles, compute_layer_position, compute_layer_size,
};
use crate::render::ElementCx;
use anyrender::PaintScene;
use blitz_dom::node::ImageData;

impl ElementCx<'_, '_> {
    pub(super) fn draw_svg_image_layer(
        &self,
        scene: &mut impl PaintScene,
        layer: &ImageLayerStyles,
    ) {
        use kurbo::Affine;

        let Some(bg_image) = layer.image_data else {
            return;
        };
        let ImageData::Svg(svg) = &bg_image.image else {
            return;
        };

        if !svg.renders_content() {
            return;
        }

        let (origin_rect, base_transform) = if self.layer_is_fixed(layer) {
            self.fixed_positioning_area()
        } else {
            (self.box_rect(layer.origin), self.transform)
        };

        let frame_w = (origin_rect.width() / self.scale) as f32;
        let frame_h = (origin_rect.height() / self.scale) as f32;

        let svg_size = svg.tree.size();

        // Size the SVG per the CSS default sizing algorithm
        // (https://drafts.csswg.org/css-images/#default-sizing). An SVG image
        // may lack an intrinsic width, height, and/or aspect ratio, so each is
        // passed separately (usvg's resolved `Tree::size` is only used as the
        // source coordinate space of the rendered tree).
        // A zero natural dimension is kept: such an image sizes to nothing
        // unless `background-size` gives it a box.
        let intrinsic_width = svg.intrinsic_width().filter(|w| w.is_finite());
        let intrinsic_height = svg.intrinsic_height().filter(|h| h.is_finite());
        let aspect_ratio = match (intrinsic_width, intrinsic_height) {
            (Some(w), Some(h)) => Some(w / h),
            _ => svg.viewbox_aspect_ratio(),
        }
        .filter(|r| r.is_finite() && *r > 0.0);

        let bg_size = compute_layer_size(
            layer,
            frame_w,
            frame_h,
            BackgroundSizeComputeMode::Intrinsic {
                width: intrinsic_width,
                height: intrinsic_height,
                ratio: aspect_ratio,
            },
        );

        if bg_size.width <= 0.0 || bg_size.height <= 0.0 {
            return;
        }

        let (x_ratio, y_ratio) = if svg.intrinsic_dimensions.view_box_size.is_some() {
            (
                bg_size.width / svg_size.width() as f64,
                bg_size.height / svg_size.height() as f64,
            )
        } else {
            content_scale_without_view_box(bg_size, intrinsic_width, intrinsic_height)
        };
        let (x_ratio, y_ratio) = (x_ratio * self.scale, y_ratio * self.scale);

        let bg_pos = compute_layer_position(
            layer,
            frame_w - bg_size.width as f32,
            frame_h - bg_size.height as f32,
        );

        let transform = base_transform
            * kurbo::Affine::translate((
                origin_rect.x0 + bg_pos.x * self.scale,
                origin_rect.y0 + bg_pos.y * self.scale,
            ))
            * Affine::scale_non_uniform(x_ratio, y_ratio);

        // Replaced content is clipped to its box, here the tile.
        let tile = kurbo::Rect::from_origin_size(
            (
                origin_rect.x0 + bg_pos.x * self.scale,
                origin_rect.y0 + bg_pos.y * self.scale,
            ),
            (bg_size.width * self.scale, bg_size.height * self.scale),
        );
        scene.push_clip_layer(base_transform, &tile);
        crate::paint_svg_tree(scene, &svg.tree, svg.nested_rasters(), transform);
        scene.pop_layer();
    }
}

/// How an SVG image without a `viewBox` scales its content into a `tile`
/// (Chrome): user units are CSS pixels, stretched along an axis with a
/// positive natural dimension to the tile's, and not at all along one
/// without. A zero natural dimension leaves the content unscaled in both.
fn content_scale_without_view_box(
    tile: kurbo::Size,
    natural_width: Option<f32>,
    natural_height: Option<f32>,
) -> (f64, f64) {
    if natural_width == Some(0.0) || natural_height == Some(0.0) {
        return (1.0, 1.0);
    }
    let ratio = |tile: f64, natural: Option<f32>| natural.map_or(1.0, |n| tile / f64::from(n));
    (
        ratio(tile.width, natural_width),
        ratio(tile.height, natural_height),
    )
}
