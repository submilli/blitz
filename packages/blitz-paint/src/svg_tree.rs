//! Paint parsed SVG images with their held raster decodes and bounded filters.
//!
//! SVG interpretation stays here; Vello owns pixel generation and filtering.
//! A paint transaction shares one budget across nested groups and documents.

mod convert;
mod filters;
mod pattern;
pub(crate) mod primitive;
mod raster;
mod resolution;
mod shapes;

use anyrender::PaintScene;
use blitz_dom::SvgNestedRasters;
use kurbo::{Affine, Rect};
use std::collections::HashSet;
use vello_common::filter::graph::{GraphError, MAX_INTERMEDIATE_PIXELS, MAX_WORK};

/// Paint `tree` under `transform`, drawing raster images from the decodes held
/// with it. Resource refusal paints no additional content from the refused group.
/// Unsupported mask behavior retains the existing renderer contract.
pub fn paint_svg_tree(
    scene: &mut impl PaintScene,
    tree: &usvg::Tree,
    rasters: &SvgNestedRasters,
    transform: Affine,
) {
    paint_tree(scene, tree, rasters, transform, true);
}

/// Filter-image sources use exact raster sampling rather than
/// the software bitmap shader used by SVG image painting.
pub fn paint_svg_tree_exact(
    scene: &mut impl PaintScene,
    tree: &usvg::Tree,
    rasters: &SvgNestedRasters,
    transform: Affine,
) {
    paint_tree(scene, tree, rasters, transform, false);
}

fn paint_tree(
    scene: &mut impl PaintScene,
    tree: &usvg::Tree,
    rasters: &SvgNestedRasters,
    transform: Affine,
    fixed: bool,
) {
    let mut budget = Budget {
        fixed_image_sampling: fixed,
        ..Default::default()
    };
    let _ = children(
        scene,
        tree.root(),
        rasters,
        transform,
        &mut budget,
        &mut HashSet::new(),
        0,
    );
}

/// Work is cumulative, including temporary images released before later groups.
struct Budget {
    fixed_image_sampling: bool,
    pixels: usize,
    surface_area: usize,
    work: u64,
    visits: usize,
    geometry: vello_common::geometry_budget::RasterBudget,
    strokes: vello_common::geometry_budget::StrokeBudget,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            fixed_image_sampling: true,
            pixels: MAX_INTERMEDIATE_PIXELS,
            surface_area: 0,
            work: MAX_WORK,
            visits: 65_536,
            geometry: Default::default(),
            strokes: Default::default(),
        }
    }
}

impl Budget {
    /// Temporary SVG painters account every draw and layer over their target.
    /// The counter stays shared when a nested pattern or filter changes target.
    fn with_surface<T>(
        &mut self,
        area: usize,
        paint: impl FnOnce(&mut Self) -> Result<T, GraphError>,
    ) -> Result<T, GraphError> {
        let previous = std::mem::replace(&mut self.surface_area, area);
        let result = paint(self);
        self.surface_area = previous;
        result
    }

    fn draw_work(&mut self) -> Result<(), GraphError> {
        self.take_work((self.surface_area as u64).saturating_mul(16))
    }

    fn take(&mut self, pixels: usize) -> Result<(), GraphError> {
        self.pixels = self.pixels.checked_sub(pixels).ok_or(GraphError::Memory)?;
        Ok(())
    }

    fn take_work(&mut self, work: u64) -> Result<(), GraphError> {
        self.work = self.work.checked_sub(work).ok_or(GraphError::Work)?;
        Ok(())
    }

    fn raster(
        &mut self,
        raster: &blitz_dom::node::RasterImageData,
        converted: &mut HashSet<u64>,
    ) -> Result<(), GraphError> {
        if converted.contains(&raster.data.id()) {
            return Ok(());
        }
        let area = u64::from(raster.width) * u64::from(raster.height);
        self.take(usize::try_from(area).map_err(|_| GraphError::Memory)?)?;
        converted.insert(raster.data.id());
        Ok(())
    }

    fn charge_graph(
        &mut self,
        cost: vello_common::filter::frames::RasterCost,
    ) -> Result<(), GraphError> {
        self.take(cost.pixels)?;
        self.work = self.work.checked_sub(cost.work).ok_or(GraphError::Work)?;
        Ok(())
    }

    fn visit(&mut self, depth: usize) -> Result<(), GraphError> {
        if depth > 128 {
            return Err(GraphError::Size);
        }
        self.visits = self.visits.checked_sub(1).ok_or(GraphError::Size)?;
        Ok(())
    }
}

fn children(
    scene: &mut impl PaintScene,
    group: &usvg::Group,
    rasters: &SvgNestedRasters,
    global: Affine,
    budget: &mut Budget,
    converted: &mut HashSet<u64>,
    depth: usize,
) -> Result<(), GraphError> {
    budget.visit(depth)?;
    for node in group.children() {
        budget.visit(depth)?;
        match node {
            usvg::Node::Group(group) => {
                // A resource-refused group is transparent; subsequent siblings
                // still paint, and the shared budget is never replenished.
                let result =
                    paint_group(scene, group, rasters, global, budget, converted, depth + 1);
                if budget.surface_area != 0 {
                    result?;
                }
            }
            usvg::Node::Text(text) => children(
                scene,
                text.flattened(),
                rasters,
                global,
                budget,
                converted,
                depth + 1,
            )?,
            usvg::Node::Path(path) => {
                let result = shapes::path(scene, path, rasters, global, budget, depth + 1);
                if budget.surface_area != 0 {
                    result?;
                }
            }
            usvg::Node::Image(image) if image.is_visible() => {
                let transform = global * convert::to_affine(&image.abs_transform());
                budget.draw_work()?;
                if let usvg::ImageKind::SVG(tree) = image.kind() {
                    children(
                        scene,
                        tree.root(),
                        rasters,
                        transform,
                        budget,
                        converted,
                        depth + 1,
                    )?;
                } else if let Some(raster) = rasters.decoded(image.kind()) {
                    // Each temporary painter has a separate image conversion
                    // cache. Charge whole decoded inputs once per painter, even
                    // when its output raster is tiny.
                    if budget.raster(raster, converted).is_err() {
                        continue;
                    }
                    let bounds = image.abs_bounding_box();
                    let destination = global.transform_rect_bbox(Rect::new(
                        f64::from(bounds.left()),
                        f64::from(bounds.top()),
                        f64::from(bounds.right()),
                        f64::from(bounds.bottom()),
                    ));
                    let _ = raster::paint(scene, raster, transform, destination, budget);
                }
            }
            usvg::Node::Image(_) => {}
        }
    }
    Ok(())
}

fn paint_group(
    scene: &mut impl PaintScene,
    group: &usvg::Group,
    rasters: &SvgNestedRasters,
    global: Affine,
    budget: &mut Budget,
    converted: &mut HashSet<u64>,
    depth: usize,
) -> Result<(), GraphError> {
    let transform = global * convert::to_affine(&group.abs_transform());
    let mix = convert::to_mix(group.blend_mode());
    let alpha = group.opacity().get();
    let clip = group
        .clip_path()
        .and_then(|clip| clip.root().children().first());
    if clip.is_some() || mix != peniko::Mix::Normal || alpha < 1.0 {
        budget.draw_work()?;
    }
    let layered = if let Some(usvg::Node::Path(path)) = clip {
        let path = convert::to_bez_path(path);
        budget
            .geometry
            .charge(&path, transform)
            .map_err(|_| GraphError::Work)?;
        scene.push_layer(mix, alpha, transform, &path, None, None);
        true
    } else if mix != peniko::Mix::Normal || alpha < 1.0 {
        scene.push_layer(
            mix,
            alpha,
            transform,
            &rect(group.layer_bounding_box()),
            None,
            None,
        );
        true
    } else {
        false
    };
    let saved_sampling = budget.fixed_image_sampling;
    if clip.is_some() {
        budget.fixed_image_sampling = true;
    }
    let result = if group.filters().is_empty() {
        children(scene, group, rasters, global, budget, converted, depth)
    } else {
        filters::paint(scene, group, rasters, global, budget, depth)
    };
    budget.fixed_image_sampling = saved_sampling;
    if layered {
        scene.pop_layer();
    }
    result
}

fn rect(rect: usvg::NonZeroRect) -> Rect {
    Rect::new(
        f64::from(rect.left()),
        f64::from(rect.top()),
        f64::from(rect.right()),
        f64::from(rect.bottom()),
    )
}

#[cfg(test)]
mod tests;
