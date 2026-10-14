//! Parsed SVG filter graphs. Coordinates and resources are resolved by usvg;
//! Vello validates and executes the graph before its result is composited.

use super::{Budget, children, convert, primitive, rect, resolution::Resolution};
use anyrender::PaintScene;
use anyrender_vello_cpu::VelloCpuScenePainter;
use blitz_dom::SvgNestedRasters;
use kurbo::{Affine, Rect};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use vello_common::filter::graph::{ColorSpace, GraphError, Input, MAX_NODES, Node, SvgGraph};
use vello_common::filter::image::FilterImage;
use vello_common::filter_effects::{CompositeOperator, FilterPrimitive, matrices};
use vello_cpu::Pixmap;

pub(super) fn paint(
    scene: &mut impl PaintScene,
    group: &usvg::Group,
    rasters: &SvgNestedRasters,
    global: Affine,
    budget: &mut Budget,
    depth: usize,
) -> Result<(), GraphError> {
    // Chrome does not generate filtered paint for an empty SVG group,
    // even with an explicit user-space filter region and feFlood.
    if !group.has_children() {
        return Ok(());
    }
    let transform = convert::to_affine(&group.abs_transform());
    if !transform.determinant().is_finite() || transform.determinant() == 0.0 {
        return Ok(());
    }
    let resolution = Resolution::new(global * transform)?;
    let stroke = group.stroke_bounding_box();
    let source_bounds = Rect::new(
        f64::from(stroke.left()),
        f64::from(stroke.top()),
        f64::from(stroke.right()),
        f64::from(stroke.bottom()),
    );
    let extent = resolution
        .rect(group.filters().iter().fold(
            if source_bounds.area() > 0.0 {
                rect(group.layer_bounding_box()).union(source_bounds)
            } else {
                rect(group.layer_bounding_box())
            },
            |extent, filter| extent.union(rect(filter.rect())),
        ))
        .expand();
    let [width, height] = dimensions(extent)?;
    // Validate topology and parameters, and charge every graph's admitted
    // execution before any source or image painter allocates its surfaces.
    let mut admitted_work = Vec::new();
    for filter in group.filters() {
        let graph =
            graph(filter, rasters, budget, depth, false, resolution)?.with_source_region(extent)?;
        let cost = graph.admit_raster(extent, extent)?.cost();
        budget.charge_graph(cost)?;
        admitted_work.push(cost.work);
    }
    let local = Affine::translate(-extent.origin().to_vec2())
        * Affine::scale(resolution.0)
        * transform.inverse();
    let mut source = VelloCpuScenePainter::new(width, height);
    budget.with_surface(usize::from(width) * usize::from(height), |budget| {
        children(
            &mut source,
            group,
            rasters,
            local,
            budget,
            &mut HashSet::new(),
            depth,
        )
    })?;
    let mut pixels = source.finish();
    for (filter, admitted) in group.filters().iter().zip(admitted_work) {
        let graph =
            graph(filter, rasters, budget, depth, true, resolution)?.with_source_region(extent)?;
        let actual = graph.admit_raster(extent, extent)?.cost().work;
        // Image placeholders validate topology before rasterization; reserve
        // their actual sampling work before executing the completed graph.
        budget.take_work(actual.saturating_sub(admitted))?;
        vello_cpu::apply_svg_graph(&graph, &mut pixels, extent.origin())?;
    }
    let brush = brush(pixels);
    let placement = global
        * transform
        * Affine::scale(1.0 / resolution.0)
        * Affine::translate(extent.origin().to_vec2());
    budget.draw_work()?;
    scene.draw_image(brush.as_ref(), placement);
    Ok(())
}

/// A bounded whole-pixel surface, admitted before conversion to renderer sides.
pub(super) fn dimensions(extent: Rect) -> Result<[u16; 2], GraphError> {
    let sides = [extent.width(), extent.height()];
    if ![extent.x0, extent.y0, extent.x1, extent.y1]
        .iter()
        .all(|v| v.is_finite() && v.abs() <= 1e6)
        || !sides.iter().all(|v| *v >= 1.0 && *v <= 16_384.0)
    {
        return Err(GraphError::Memory);
    }
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "Whole positive pixel dimensions checked above."
    )]
    Ok(sides.map(|v| v as u16))
}

fn graph(
    filter: &usvg::filter::Filter,
    rasters: &SvgNestedRasters,
    budget: &mut Budget,
    depth: usize,
    images: bool,
    resolution: Resolution,
) -> Result<SvgGraph, GraphError> {
    let mut nodes = Vec::new();
    let mut results = HashMap::new();
    for primitive in filter.primitives() {
        let region = resolution.rect(rect(primitive.rect()).intersect(rect(filter.rect())));
        let space = match primitive.color_interpolation() {
            usvg::filter::ColorInterpolation::SRGB => ColorSpace::Srgb,
            usvg::filter::ColorInterpolation::LinearRGB => ColorSpace::LinearRgb,
        };
        let kind = primitive.kind();
        let output = match kind {
            usvg::filter::Kind::Image(image) => {
                let pixels = if images {
                    image_pixels(
                        image.root(),
                        rect(primitive.rect()),
                        region,
                        rasters,
                        budget,
                        depth + 1,
                        resolution,
                    )?
                } else {
                    None
                };
                let operation = pixels.map_or(
                    FilterPrimitive::Flood {
                        color: vello_common::color::AlphaColor::TRANSPARENT,
                    },
                    |image| FilterPrimitive::Image { image },
                );
                push(
                    &mut nodes,
                    operation,
                    Input::SourceGraphic,
                    None,
                    region,
                    ColorSpace::Srgb,
                )?
            }
            usvg::filter::Kind::Merge(merge) => {
                let mut output = None;
                for input in merge.inputs() {
                    let input = resolve(input, &results)?;
                    output = Some(match output {
                        None => push(
                            &mut nodes,
                            FilterPrimitive::ColorMatrix {
                                matrix: matrices::IDENTITY,
                            },
                            input,
                            None,
                            region,
                            space,
                        )?,
                        Some(previous) => push(
                            &mut nodes,
                            FilterPrimitive::Composite {
                                operator: CompositeOperator::Over,
                            },
                            input,
                            Some(previous),
                            region,
                            space,
                        )?,
                    });
                }
                match output {
                    Some(output) => output,
                    None => push(
                        &mut nodes,
                        FilterPrimitive::Flood {
                            color: vello_common::color::AlphaColor::TRANSPARENT,
                        },
                        Input::SourceGraphic,
                        None,
                        region,
                        space,
                    )?,
                }
            }
            _ => {
                let operation = primitive::operation(
                    kind,
                    Some(blitz_dom::CanvasSvgUnits::from_filter(filter)),
                )?;
                let input = operation
                    .input
                    .map(|input| resolve(input, &results))
                    .transpose()?
                    .unwrap_or(Input::SourceGraphic);
                let input2 = operation
                    .input2
                    .map(|input| resolve(input, &results))
                    .transpose()?;
                push(
                    &mut nodes,
                    resolution.operation(operation.primitive),
                    input,
                    input2,
                    region,
                    space,
                )?
            }
        };
        results.insert(primitive.result(), output);
    }
    let output = nodes.len().checked_sub(1).ok_or(GraphError::Size)?;
    SvgGraph::new(nodes, resolution.rect(rect(filter.rect())), output)
}

fn resolve(
    input: &usvg::filter::Input,
    results: &HashMap<&str, Input>,
) -> Result<Input, GraphError> {
    match input {
        usvg::filter::Input::SourceGraphic => Ok(Input::SourceGraphic),
        usvg::filter::Input::SourceAlpha => Ok(Input::SourceAlpha),
        usvg::filter::Input::Reference(name) => {
            results.get(name.as_str()).copied().ok_or(GraphError::Input)
        }
    }
}

fn push(
    nodes: &mut Vec<Node>,
    primitive: FilterPrimitive,
    input: Input,
    input2: Option<Input>,
    region: Rect,
    color_space: ColorSpace,
) -> Result<Input, GraphError> {
    if nodes.len() == MAX_NODES {
        return Err(GraphError::Size);
    }
    let output = Input::Result(nodes.len());
    nodes.push(Node {
        primitive,
        input,
        input2,
        region,
        color_space,
    });
    Ok(output)
}

/// usvg has already placed the image content inside this root, including its
/// preserveAspectRatio transform. Its held decodes share the parent lifetime.
fn image_pixels(
    group: &usvg::Group,
    subregion: Rect,
    region: Rect,
    rasters: &SvgNestedRasters,
    budget: &mut Budget,
    depth: usize,
    resolution: Resolution,
) -> Result<Option<FilterImage>, GraphError> {
    if region.area() <= 0.0 {
        return Ok(None);
    }
    let extent = region.expand();
    let [width, height] = dimensions(extent)?;
    budget.take(usize::from(width) * usize::from(height) * 2)?;
    let mut scene = VelloCpuScenePainter::new(width, height);
    budget.with_surface(usize::from(width) * usize::from(height), |budget| {
        children(
            &mut scene,
            group,
            rasters,
            Affine::translate(-extent.origin().to_vec2())
                * Affine::scale(resolution.0)
                * Affine::translate(subregion.origin().to_vec2()),
            budget,
            &mut HashSet::new(),
            depth,
        )
    })?;
    let pixels = scene.finish();
    let size = [u32::from(width), u32::from(height)];
    let source = Rect::from_origin_size((0.0, 0.0), extent.size());
    let image = FilterImage::plan(size, source, extent, region, region)
        .and_then(|plan| plan.keep(pixels, [0, 0]));
    Ok(image)
}

pub(super) fn brush(pixels: Pixmap) -> peniko::ImageBrush {
    let width = u32::from(pixels.width());
    let height = u32::from(pixels.height());
    peniko::ImageBrush::new(peniko::ImageData {
        data: peniko::Blob::new(Arc::new(pixels.data_as_u8_slice().to_vec())),
        format: peniko::ImageFormat::Rgba8,
        alpha_type: peniko::ImageAlphaType::AlphaPremultiplied,
        width,
        height,
    })
}
