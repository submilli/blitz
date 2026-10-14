// Copyright 2024 the Vello Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Leaf path painting, derived from anyrender_svg 0.14. Filters surround groups;
//! leaves keep the same paint conversion and fill/stroke ordering.

use super::{Budget, convert};
use anyrender::PaintScene;
use kurbo::{Affine, BezPath};
use vello_common::filter::graph::GraphError;

pub(super) fn path(
    scene: &mut impl PaintScene,
    path: &usvg::Path,
    global: Affine,
    budget: &mut Budget,
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
    match path.paint_order() {
        usvg::PaintOrder::FillAndStroke => {
            fill(scene, path, transform, &geometry);
            stroke(scene, path, transform, &geometry);
        }
        usvg::PaintOrder::StrokeAndFill => {
            stroke(scene, path, transform, &geometry);
            fill(scene, path, transform, &geometry);
        }
    }
    Ok(())
}

fn fill(scene: &mut impl PaintScene, path: &usvg::Path, transform: Affine, geometry: &BezPath) {
    if let Some(fill) = path.fill()
        && let Some((paint, brush_transform)) = convert::to_brush(fill.paint(), fill.opacity())
    {
        let rule = match fill.rule() {
            usvg::FillRule::NonZero => peniko::Fill::NonZero,
            usvg::FillRule::EvenOdd => peniko::Fill::EvenOdd,
        };
        scene.fill(
            rule,
            transform,
            paint.as_ref(),
            Some(brush_transform),
            geometry,
        );
    }
}

fn stroke(scene: &mut impl PaintScene, path: &usvg::Path, transform: Affine, geometry: &BezPath) {
    if let Some(stroke) = path.stroke()
        && let Some((paint, brush_transform)) = convert::to_brush(stroke.paint(), stroke.opacity())
    {
        if let Some(rect) = path.source_rect() {
            scene.stroke(
                &convert::to_stroke(stroke),
                transform,
                paint.as_ref(),
                Some(brush_transform),
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
            Some(brush_transform),
            geometry,
        );
    }
}
