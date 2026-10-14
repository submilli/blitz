//! SVG raster placement and decode-cache sizing. The caller owns its decode;
//! Vello owns pixel conversion, sampling, mip generation and graph execution.

use super::{Budget, filters};
use anyrender::PaintScene;
use blitz_dom::node::RasterImageData;
use kurbo::{Affine, Rect, Shape};
use std::sync::Arc;
use vello_common::filter::graph::{ColorSpace, GraphError, Input, Node, SvgGraph};
use vello_common::filter::image::FilterImage;
use vello_common::filter_effects::FilterPrimitive;
use vello_cpu::{PaintType, Pixmap, RenderContext, Resources, SamplingPrecision};

pub(super) fn paint(
    scene: &mut impl PaintScene,
    raster: &RasterImageData,
    transform: Affine,
    destination: Rect,
    budget: &mut Budget,
) -> Result<(), GraphError> {
    // A single texel has no sampling phase or mip detail. Keep its held blob
    // so repeated paints share the renderer's existing conversion cache.
    if raster.width == 1 && raster.height == 1 {
        let brush = peniko::ImageBrush::new(peniko::ImageData {
            data: raster.data.clone(),
            format: peniko::ImageFormat::Rgba8,
            alpha_type: peniko::ImageAlphaType::Alpha,
            width: 1,
            height: 1,
        });
        scene.draw_image(brush.as_ref(), transform);
        return Ok(());
    }
    let source = Rect::new(0.0, 0.0, f64::from(raster.width), f64::from(raster.height));
    let extent = destination.expand();
    let [width, height] = filters::dimensions(extent)?;
    let area = usize::from(width) * usize::from(height);
    budget.take(area * 3)?;
    budget.take_work(area as u64 * 16)?;
    // Each leaf converts its held RGBA input into a premultiplied surface.
    // Charge every conversion before allocating; no conversion cache is held.
    let source_area = u64::from(raster.width) * u64::from(raster.height);
    budget.take(usize::try_from(source_area).map_err(|_| GraphError::Memory)?)?;
    budget.take_work(source_area * 4)?;
    let input = pixels(raster)?;
    let matrix = transform.as_coeffs();
    let mut output = if matrix[1] == 0.0
        && matrix[2] == 0.0
        && matrix[0] > 0.0
        && matrix[3] > 0.0
        && (matrix[0] < 1.0 || matrix[3] < 1.0)
    {
        if budget.fixed_image_sampling {
            let (input, source, transform) = nearest_mip(input, source, destination, budget)?;
            bitmap(input, source, transform, extent, [width, height], true)
        } else {
            downscale(input, source, destination, extent, [width, height], budget)?
        }
    } else {
        bitmap(
            input,
            source,
            transform,
            extent,
            [width, height],
            budget.fixed_image_sampling,
        )
    };
    output.recompute_may_have_transparency();
    let brush = filters::brush(output).with_quality(peniko::ImageQuality::Low);
    scene.draw_image(brush.as_ref(), Affine::translate(extent.origin().to_vec2()));
    Ok(())
}

fn pixels(raster: &RasterImageData) -> Result<Pixmap, GraphError> {
    let width = u16::try_from(raster.width).map_err(|_| GraphError::Size)?;
    let height = u16::try_from(raster.height).map_err(|_| GraphError::Size)?;
    let mut pixels = Pixmap::new(width, height);
    for (pixel, rgba) in pixels
        .data_mut()
        .iter_mut()
        .zip(raster.data.data().as_chunks::<4>().0)
    {
        let alpha = u16::from(rgba[3]);
        let channel = |i| ((u16::from(rgba[i]) * alpha + 127) / 255) as u8;
        *pixel = vello_common::color::PremulRgba8 {
            r: channel(0),
            g: channel(1),
            b: channel(2),
            a: rgba[3],
        };
    }
    Ok(pixels)
}

fn downscale(
    input: Pixmap,
    source: Rect,
    destination: Rect,
    extent: Rect,
    [width, height]: [u16; 2],
    budget: &mut Budget,
) -> Result<Pixmap, GraphError> {
    let plan = FilterImage::plan(
        [u32::from(input.width()), u32::from(input.height())],
        source,
        destination,
        destination,
        extent,
    )
    .ok_or(GraphError::Size)?;
    budget.take(
        usize::try_from(plan.mip_texels().ok_or(GraphError::Size)?)
            .map_err(|_| GraphError::Memory)?,
    )?;
    budget.take_work(plan.stream_work())?;
    let image = plan.share(Arc::new(input)).ok_or(GraphError::Memory)?;
    let graph = SvgGraph::new(
        vec![Node {
            primitive: FilterPrimitive::Image { image },
            input: Input::SourceGraphic,
            input2: None,
            region: destination,
            color_space: ColorSpace::Srgb,
        }],
        extent,
        0,
    )?;
    budget.charge_graph(graph.admit_raster(extent, extent)?.cost())?;
    let mut output = Pixmap::new(width, height);
    vello_cpu::apply_svg_graph(&graph, &mut output, extent.origin())?;
    Ok(output)
}

fn bitmap(
    input: Pixmap,
    source: Rect,
    transform: Affine,
    extent: Rect,
    [width, height]: [u16; 2],
    fixed: bool,
) -> Pixmap {
    let mut context = RenderContext::new(width, height);
    let local = Affine::translate(-extent.origin().to_vec2()) * transform;
    context.set_transform(Affine::IDENTITY);
    context.set_paint(PaintType::Image(peniko::ImageBrush {
        image: vello_cpu::ImageSource::Pixmap(Arc::new(input)),
        sampler: peniko::ImageSampler::default(),
    }));
    context.set_paint_transform(local);
    if fixed {
        context.set_sampling_precision(SamplingPrecision::FixedPointAtOrigin {
            span_x: Some((transform.transform_rect_bbox(source).x0.ceil() - extent.x0) as i32),
            origin: [extent.x0 as i32, extent.y0 as i32],
        });
    }
    context.fill_path(&(local * source.to_path(0.1)));
    let mut output = Pixmap::new(width, height);
    context.render(output.as_mut(), &mut Resources::new());
    output
}

fn nearest_mip(
    input: Pixmap,
    source: Rect,
    destination: Rect,
    budget: &mut Budget,
) -> Result<(Pixmap, Rect, Affine), GraphError> {
    let original = [u32::from(input.width()), u32::from(input.height())];
    let target = [
        destination.width().round().max(1.0) as u32,
        destination.height().round().max(1.0) as u32,
    ];
    let mut cached = original;
    for level in 1..=15 {
        let next = original.map(|side| side.div_ceil(1 << level));
        if next == cached || next[0] < target[0] || next[1] < target[1] {
            break;
        }
        cached = next;
    }
    let cache_rect = Rect::new(0.0, 0.0, f64::from(cached[0]), f64::from(cached[1]));
    let place = |size: [u32; 2]| {
        let sx = (destination.width() as f32) / size[0] as f32;
        let sy = (destination.height() as f32) / size[1] as f32;
        Affine::new([
            f64::from(sx),
            0.0,
            0.0,
            f64::from(sy),
            destination.x0,
            destination.y0,
        ])
    };
    if cached == original {
        return Ok((input, source, place(original)));
    }
    let plan = FilterImage::plan(original, source, cache_rect, cache_rect, cache_rect)
        .ok_or(GraphError::Size)?;
    budget.take(
        usize::try_from(plan.mip_texels().ok_or(GraphError::Size)?)
            .map_err(|_| GraphError::Memory)?,
    )?;
    // Mip streaming, extraction and the cache-sized resize are additional
    // to the final destination; admit them before any of those allocations.
    budget.take_work(plan.stream_work())?;
    let cache_area = u64::from(cached[0]) * u64::from(cached[1]);
    budget.take(usize::try_from(cache_area.saturating_mul(3)).map_err(|_| GraphError::Memory)?)?;
    budget.take_work(cache_area.saturating_mul(16))?;
    let image = plan.share(Arc::new(input)).ok_or(GraphError::Memory)?;
    let mip = image.sampling().mipmap;
    let level = mip.map_or(0, |m| m.level + u8::from(m.weight >= 0.5));
    let [width, height] = image.sampling().level_size(level);
    let window = image.window(level).ok_or(GraphError::Size)?;
    budget.take(
        usize::try_from(u64::from(width) * u64::from(height)).map_err(|_| GraphError::Memory)?,
    )?;
    budget.take_work(u64::from(width) * u64::from(height))?;
    let mut pixels = Pixmap::new(
        u16::try_from(width).map_err(|_| GraphError::Size)?,
        u16::try_from(height).map_err(|_| GraphError::Size)?,
    );
    for y in 0..height {
        for x in 0..width {
            pixels.data_mut()[(y * width + x) as usize] = window.texel(i64::from(x), i64::from(y));
        }
    }
    let mip_rect = Rect::new(0.0, 0.0, f64::from(width), f64::from(height));
    let resize = Affine::scale_non_uniform(
        f64::from(cached[0]) / f64::from(width),
        f64::from(cached[1]) / f64::from(height),
    );
    let resized = bitmap(
        pixels,
        mip_rect,
        resize,
        cache_rect,
        [cached[0] as u16, cached[1] as u16],
        false,
    );
    Ok((resized, cache_rect, place(cached)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_held_rasters_fit_the_unchanged_sampling_budget() {
        let raster =
            RasterImageData::new(2048, 2048, Arc::new([0, 255, 0, 255].repeat(2048 * 2048)));
        let destination = Rect::new(0.0, 0.0, 32.0, 32.0);
        for fixed in [true, false] {
            let mut budget = Budget {
                fixed_image_sampling: fixed,
                ..Default::default()
            };
            budget
                .raster(&raster, &mut std::collections::HashSet::new())
                .unwrap();
            let mut scene = anyrender::Scene::new();
            paint(
                &mut scene,
                &raster,
                Affine::scale(1.0 / 64.0),
                destination,
                &mut budget,
            )
            .unwrap();
            assert!(!scene.commands.is_empty());
        }
    }

    #[test]
    fn repeated_cache_resizes_share_one_cumulative_pixel_and_work_budget() {
        let raster = RasterImageData::new(16, 16, Arc::new([0, 255, 0, 255].repeat(16 * 16)));
        let source = Rect::new(0.0, 0.0, 16.0, 16.0);
        let destination = Rect::new(0.0, 0.0, 5.0, 5.0);
        for (fixed, pixels, work, error) in [
            (true, 1000, 1_000_000, GraphError::Memory),
            (true, 5000, 14_000, GraphError::Work),
            (false, 5000, 15_000, GraphError::Work),
        ] {
            let mut budget = Budget {
                fixed_image_sampling: fixed,
                pixels,
                work,
                ..Default::default()
            };
            let mut scene = anyrender::Scene::new();
            let transform = Affine::scale_non_uniform(
                destination.width() / source.width(),
                destination.height() / source.height(),
            );
            paint(&mut scene, &raster, transform, destination, &mut budget).unwrap();
            let commands = scene.commands.len();
            assert!(commands > 0);
            assert_eq!(
                paint(&mut scene, &raster, transform, destination, &mut budget),
                Err(error)
            );
            assert_eq!(
                scene.commands.len(),
                commands,
                "refused work commits no image"
            );
        }
    }
}
