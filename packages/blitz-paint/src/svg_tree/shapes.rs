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
    Ok(convert::to_brush(paint, opacity))
}
