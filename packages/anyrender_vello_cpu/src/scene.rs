use std::sync::Arc;

use anyrender::{Filter, NormalizedCoord, Paint, PaintRef, PaintScene, RenderContext};
use glifo::FontEmbolden;
use kurbo::{Affine, Diagonal2, Rect, Shape, Stroke};
use peniko::{BlendMode, Color, Fill, FontData, ImageBrush, StyleRef};
use vello_cpu::{PaintType, Pixmap};

use crate::image_cache::{ImageCache, ImageCacheConfig};

const DEFAULT_TOLERANCE: f64 = 0.1;

pub struct VelloCpuScenePainter {
    pub(crate) render_ctx: vello_cpu::RenderContext,
    pub(crate) resources: vello_cpu::Resources,
    pub(crate) image_cache: ImageCache,
}

impl VelloCpuScenePainter {
    pub fn new(width: u16, height: u16) -> Self {
        Self::with_image_cache_config(width, height, ImageCacheConfig::default())
    }

    pub fn with_image_cache_config(width: u16, height: u16, config: ImageCacheConfig) -> Self {
        Self {
            render_ctx: vello_cpu::RenderContext::new(width, height),
            resources: vello_cpu::Resources::new(),
            image_cache: ImageCache::new(config),
        }
    }

    fn convert_paint(&mut self, paint: PaintRef<'_>) -> PaintType {
        match paint {
            Paint::Solid(alpha_color) => PaintType::Solid(alpha_color),
            Paint::Gradient(gradient) => PaintType::Gradient(gradient.clone()),
            Paint::Image(image) => PaintType::Image(ImageBrush {
                image: self
                    .image_cache
                    .get_or_register(&mut self.resources, image.image),
                sampler: image.sampler,
            }),
            // TODO: custom paint
            Paint::Resource(_) => PaintType::Solid(peniko::color::palette::css::TRANSPARENT),
            Paint::Custom(_) => PaintType::Solid(peniko::color::palette::css::TRANSPARENT),
        }
    }

    /// Advance the image cache's frame counter and evict stale entries.
    ///
    /// Should be called once per frame, after rendering.
    pub fn maintain(&mut self) {
        self.image_cache.maintain(&mut self.resources);
    }

    /// Drop all cached image conversions.
    pub fn clear_image_cache(&mut self) {
        self.image_cache.clear(&mut self.resources);
    }

    pub fn finish(mut self) -> Pixmap {
        let mut pixmap = Pixmap::new(self.render_ctx.width(), self.render_ctx.height());
        self.render_ctx.render(pixmap.as_mut(), &mut self.resources);
        pixmap
    }
}

impl RenderContext for VelloCpuScenePainter {}
impl PaintScene for VelloCpuScenePainter {
    fn reset(&mut self) {
        self.render_ctx.reset();
    }

    fn push_layer(
        &mut self,
        blend: impl Into<BlendMode>,
        alpha: f32,
        transform: Affine,
        clip: &impl Shape,
        filter: Option<Arc<Filter>>,
        _backdrop_filter: Option<Arc<Filter>>,
    ) {
        #[cfg(feature = "filters")]
        let filter = filter
            .and_then(crate::filters::convert_filter)
            .filter(|_| cfg!(not(feature = "multithreading")));

        #[cfg(not(feature = "filters"))]
        let filter = {
            let _ = filter;
            None
        };

        self.render_ctx.set_transform(transform);
        self.render_ctx.push_layer(
            Some(&clip.into_path(DEFAULT_TOLERANCE)),
            Some(blend.into()),
            Some(alpha),
            None,
            filter,
        );
    }

    fn push_clip_layer(&mut self, transform: Affine, clip: &impl Shape) {
        self.render_ctx.set_transform(transform);
        self.render_ctx
            .push_clip_layer(&clip.into_path(DEFAULT_TOLERANCE));
    }

    fn pop_layer(&mut self) {
        self.render_ctx.pop_layer();
    }

    fn stroke<'a>(
        &mut self,
        style: &Stroke,
        transform: Affine,
        paint: impl Into<PaintRef<'a>>,
        brush_transform: Option<Affine>,
        shape: &impl Shape,
    ) {
        self.render_ctx.set_transform(transform);
        self.render_ctx.set_stroke(style.clone());
        let paint = self.convert_paint(paint.into());
        self.render_ctx.set_paint(paint);
        self.render_ctx
            .set_paint_transform(brush_transform.unwrap_or(Affine::IDENTITY));
        if let Some(rect) = shape.as_rect() {
            self.render_ctx.stroke_rect(&rect);
        } else {
            self.render_ctx
                .stroke_path(&shape.into_path(DEFAULT_TOLERANCE));
        }
    }

    fn fill<'a>(
        &mut self,
        style: Fill,
        transform: Affine,
        paint: impl Into<PaintRef<'a>>,
        brush_transform: Option<Affine>,
        shape: &impl Shape,
    ) {
        self.render_ctx.set_transform(transform);
        self.render_ctx.set_fill_rule(style);
        let paint = self.convert_paint(paint.into());
        self.render_ctx.set_paint(paint);
        self.render_ctx
            .set_paint_transform(brush_transform.unwrap_or(Affine::IDENTITY));
        self.render_ctx
            .fill_path(&shape.into_path(DEFAULT_TOLERANCE));
    }

    fn draw_glyphs<'a, 's: 'a>(
        &'a mut self,
        font: &'a FontData,
        font_size: f32,
        hint: bool,
        normalized_coords: &'a [NormalizedCoord],
        embolden: kurbo::Vec2,
        style: impl Into<StyleRef<'a>>,
        paint: impl Into<PaintRef<'a>>,
        _brush_alpha: f32,
        transform: Affine,
        glyph_transform: Option<Affine>,
        glyphs: impl Iterator<Item = anyrender::Glyph> + Clone,
    ) {
        self.render_ctx.set_transform(transform);
        let paint = self.convert_paint(paint.into());
        self.render_ctx.set_paint(paint);

        let style: StyleRef<'a> = style.into();
        match style {
            StyleRef::Fill(fill) => {
                self.render_ctx.set_fill_rule(fill);
                self.render_ctx
                    .glyph_run(&mut self.resources, font)
                    .font_size(font_size)
                    .hint(hint)
                    .normalized_coords(normalized_coords)
                    .font_embolden(FontEmbolden::new(Diagonal2::new(embolden.x, embolden.y)))
                    .glyph_transform(glyph_transform.unwrap_or_default())
                    .fill_glyphs(glyphs.map(|g| vello_cpu::Glyph {
                        id: g.id,
                        x: g.x,
                        y: g.y,
                    }));
            }
            StyleRef::Stroke(stroke) => {
                self.render_ctx.set_stroke(stroke.clone());
                self.render_ctx
                    .glyph_run(&mut self.resources, font)
                    .font_size(font_size)
                    .hint(hint)
                    .normalized_coords(normalized_coords)
                    .glyph_transform(glyph_transform.unwrap_or_default())
                    .stroke_glyphs(glyphs.map(|g| vello_cpu::Glyph {
                        id: g.id,
                        x: g.x,
                        y: g.y,
                    }));
            }
        }
    }
    fn draw_box_shadow(
        &mut self,
        transform: Affine,
        rect: Rect,
        color: Color,
        radius: f64,
        std_dev: f64,
    ) {
        self.render_ctx.set_transform(transform);
        self.render_ctx.set_paint(PaintType::Solid(color));
        self.render_ctx
            .fill_blurred_rounded_rect(&rect, radius as f32, std_dev as f32, false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use peniko::color::palette::css::RED;

    #[test]
    fn source_rect_and_rectangular_path_keep_distinct_nonuniform_coverage() {
        let rect = Rect::new(60.0, 30.0, 210.0, 105.0);
        for (primitive, expected) in [(true, 255), (false, 184)] {
            let mut scene = VelloCpuScenePainter::new(16, 16);
            let stroke = Stroke::new(23.717).with_join(kurbo::Join::Miter);
            let transform = Affine::scale_non_uniform(8.0 / 300.0, 8.0 / 150.0);
            if primitive {
                scene.stroke(&stroke, transform, RED, None, &rect);
            } else {
                scene.stroke(&stroke, transform, RED, None, &rect.to_path(0.1));
            }
            let image = scene.finish();
            assert!(
                image.data()[17].a.abs_diff(expected) <= 1,
                "primitive={primitive}, alpha={}",
                image.data()[17].a
            );
        }
    }
}
