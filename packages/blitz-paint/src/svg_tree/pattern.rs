//! SVG tiles are rasterized once per paint, then repeated by the image shader.
//! usvg resolves bounding-box units and viewBox into the tile's user space.

use super::{Budget, children, convert, filters, rect};
use anyrender::Paint;
use anyrender_vello_cpu::VelloCpuScenePainter;
use blitz_dom::SvgNestedRasters;
use kurbo::Affine;
use std::collections::HashSet;
use vello_common::filter::graph::GraphError;

pub(super) fn brush(
    pattern: &usvg::Pattern,
    opacity: usvg::Opacity,
    device: Affine,
    rasters: &SvgNestedRasters,
    budget: &mut Budget,
    depth: usize,
) -> Result<(Paint, Affine), GraphError> {
    budget.visit(depth)?;
    let transform = convert::to_affine(&pattern.transform());
    let combined = device * transform;
    if !combined.determinant().is_finite() || combined.determinant() == 0.0 {
        return Err(GraphError::Parameter);
    }
    let [a, b, c, d, _, _] = combined.as_coeffs();
    let tile = rect(pattern.rect());
    let width = (tile.width() * a.hypot(b)).ceil().max(1.0);
    let height = (tile.height() * c.hypot(d)).ceil().max(1.0);
    let [width, height] = filters::dimensions(kurbo::Rect::new(0.0, 0.0, width, height))?;
    let area = usize::from(width) * usize::from(height);
    // CPU target, finished pixmap, RGBA brush and outer painter conversion.
    budget.take(area.checked_mul(4).ok_or(GraphError::Memory)?)?;
    budget.take_work(area as u64 * 16)?;
    let scale = Affine::scale_non_uniform(
        f64::from(width) / tile.width(),
        f64::from(height) / tile.height(),
    );
    let mut scene = VelloCpuScenePainter::new(width, height);
    budget.with_surface(area, |budget| {
        children(
            &mut scene,
            pattern.root(),
            rasters,
            scale,
            budget,
            &mut HashSet::new(),
            depth,
        )
    })?;
    let brush = filters::brush(scene.finish())
        .with_extend(peniko::Extend::Repeat)
        .with_alpha(opacity.get());
    let placement = transform * Affine::translate(tile.origin().to_vec2()) * scale.inverse();
    Ok((Paint::Image(brush), placement))
}
