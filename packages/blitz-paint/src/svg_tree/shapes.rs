// Copyright 2024 the Vello Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Leaf path painting, derived from anyrender_svg 0.14. Filters surround groups;
//! leaves keep the same paint conversion and fill/stroke ordering.

use super::{Budget, convert, pattern};
use anyrender::PaintScene;
use blitz_dom::SvgNestedRasters;
use kurbo::{Affine, BezPath};
use vello_common::filter::graph::GraphError;

pub(super) fn path(
    scene: &mut impl PaintScene,
    path: &usvg::Path,
    rasters: &SvgNestedRasters,
    global: Affine,
    budget: &mut Budget,
    depth: usize,
) -> Result<(), GraphError> {
    if !path.is_visible() {
        return Ok(());
    }
    let geometry = convert::to_bez_path(path);
    let transform = global * convert::to_affine(&path.abs_transform());
    budget
        .geometry
        .charge(&geometry, transform)
        .map_err(|_| GraphError::Work)?;
    if let Some(stroke) = path.stroke() {
        budget
            .strokes
            .charge(&geometry, &convert::to_stroke(stroke), transform)
            .map_err(|_| GraphError::Work)?;
    }
    let fill_brush = path
        .fill()
        .map(|fill| {
            brush(
                fill.paint(),
                false,
                fill.opacity(),
                transform,
                rasters,
                budget,
                depth,
            )
        })
        .transpose()?
        .flatten();
    let stroke_brush = path
        .stroke()
        .map(|stroke| {
            brush(
                stroke.paint(),
                true,
                stroke.opacity(),
                transform,
                rasters,
                budget,
                depth,
            )
        })
        .transpose()?
        .flatten();
    match path.paint_order() {
        usvg::PaintOrder::FillAndStroke => {
            if fill_brush.is_some() {
                budget.draw_work()?;
            }
            fill(scene, path, transform, &geometry, fill_brush.as_ref());
            if stroke_brush.is_some() {
                budget.draw_work()?;
            }
            stroke(scene, path, transform, &geometry, stroke_brush.as_ref());
        }
        usvg::PaintOrder::StrokeAndFill => {
            if stroke_brush.is_some() {
                budget.draw_work()?;
            }
            stroke(scene, path, transform, &geometry, stroke_brush.as_ref());
            if fill_brush.is_some() {
                budget.draw_work()?;
            }
            fill(scene, path, transform, &geometry, fill_brush.as_ref());
        }
    }
    Ok(())
}

fn fill(
    scene: &mut impl PaintScene,
    path: &usvg::Path,
    transform: Affine,
    geometry: &BezPath,
    brush: Option<&(anyrender::Paint, Affine)>,
) {
    if let Some(fill) = path.fill()
        && let Some((paint, brush_transform)) = brush
    {
        let rule = match fill.rule() {
            usvg::FillRule::NonZero => peniko::Fill::NonZero,
            usvg::FillRule::EvenOdd => peniko::Fill::EvenOdd,
        };
        scene.fill(
            rule,
            transform,
            paint.as_ref(),
            Some(*brush_transform),
            geometry,
        );
    }
}

fn stroke(
    scene: &mut impl PaintScene,
    path: &usvg::Path,
    transform: Affine,
    geometry: &BezPath,
    brush: Option<&(anyrender::Paint, Affine)>,
) {
    if let Some(stroke) = path.stroke()
        && let Some((paint, brush_transform)) = brush
    {
        if let Some(rect) = path.source_rect() {
            scene.stroke(
                &convert::to_stroke(stroke),
                transform,
                paint.as_ref(),
                Some(*brush_transform),
                &kurbo::Rect::new(
                    f64::from(rect.left()),
                    f64::from(rect.top()),
                    f64::from(rect.right()),
                    f64::from(rect.bottom()),
                ),
            );
            return;
        }
        scene.stroke(
            &convert::to_stroke(stroke),
            transform,
            paint.as_ref(),
            Some(*brush_transform),
            geometry,
        );
    }
}

fn brush(
    paint: &usvg::Paint,
    stroke: bool,
    opacity: usvg::Opacity,
    transform: Affine,
    rasters: &SvgNestedRasters,
    budget: &mut Budget,
    depth: usize,
) -> Result<Option<(anyrender::Paint, Affine)>, GraphError> {
    if let usvg::Paint::Pattern(pattern) = paint {
        return match pattern::brush(pattern, opacity, transform, rasters, budget, depth) {
            Ok(brush) => Ok(Some(brush)),
            Err(_) if budget.surface_area == 0 => Ok(None),
            Err(error) => Err(error),
        };
    }
    let stops = match paint {
        usvg::Paint::LinearGradient(gradient) => gradient.stops(),
        usvg::Paint::RadialGradient(gradient) => gradient.stops(),
        _ => return Ok(convert::to_brush(paint, opacity)),
    };
    let endpoints =
        matches!(stops, [first, last] if first.offset().get() == 0.0 && last.offset().get() == 1.0);
    if let Err(error) = reserve_gradient(&mut budget.gradient_bytes, stops.len(), endpoints, stroke)
    {
        return if budget.surface_area == 0 {
            Ok(None)
        } else {
            Err(error)
        };
    }
    Ok(convert::to_brush(paint, opacity))
}

/// Each draw retains its own CPU paint encoding until the scene renders.
/// Include capacity growth, endpoint padding, and the second hairline encoding.
fn reserve_gradient(
    remaining: &mut usize,
    stops: usize,
    endpoints: bool,
    stroke: bool,
) -> Result<(), GraphError> {
    use vello_common::encode::{EncodedPaint, GradientRange, MAX_GRADIENT_LUT_SIZE};
    let entries = stops.saturating_add(2);
    let ranges =
        entries.saturating_mul(size_of::<GradientRange>() + size_of::<peniko::ColorStop>());
    let encoded = ranges
        .saturating_add(size_of::<EncodedPaint>())
        .saturating_mul(if stroke { 4 } else { 2 });
    let lut_entries = if endpoints {
        256
    } else {
        MAX_GRADIENT_LUT_SIZE
    };
    let bytes = encoded.saturating_add((lut_entries + 1) * size_of::<[f32; 4]>());
    *remaining = remaining.checked_sub(bytes).ok_or(GraphError::Memory)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::reserve_gradient;

    #[test]
    fn gradient_reservations_accumulate_and_refusal_is_atomic() {
        let mut measured = usize::MAX;
        reserve_gradient(&mut measured, 2, true, false).unwrap();
        let cost = usize::MAX - measured;
        let mut available = 2 * cost;
        reserve_gradient(&mut available, 2, true, false).unwrap();
        assert!(reserve_gradient(&mut available, 2, true, true).is_err());
        assert_eq!(available, cost);
        assert!(reserve_gradient(&mut available, 3, false, false).is_err());
        reserve_gradient(&mut available, 2, true, false).unwrap();
        assert_eq!(available, 0);
    }
}
