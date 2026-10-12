//! Painting a parsed SVG document with the nested rasters Blitz decoded once
//! (see [`SvgNestedRasters`]), so no paint decodes them again.
//!
//! anyrender_svg is built without its `image` feature, so it hands each raster
//! `<image>` to the error handler instead of decoding it, and the handler
//! draws the held decode as anyrender_svg would have drawn its own: straight
//! RGBA8 at the image's absolute transform, with the default sampler.

use anyrender::PaintScene;
use blitz_dom::SvgNestedRasters;
use kurbo::Affine;

/// Paint `tree` under `transform`, drawing its raster images from `rasters`,
/// the decodes held with the tree. A raster without a decode draws nothing,
/// as do features the renderer does not support (such as pattern paints).
pub fn paint_svg_tree(
    scene: &mut impl PaintScene,
    tree: &usvg::Tree,
    rasters: &SvgNestedRasters,
    transform: Affine,
) {
    anyrender_svg::render_svg_tree_with(scene, tree, transform, &mut |scene, node| {
        let usvg::Node::Image(image) = node else {
            return;
        };
        let Some(raster) = rasters.decoded(image.kind()) else {
            return;
        };
        let brush = peniko::ImageBrush::new(peniko::ImageData {
            data: raster.data.clone(),
            format: peniko::ImageFormat::Rgba8,
            alpha_type: peniko::ImageAlphaType::Alpha,
            width: raster.width,
            height: raster.height,
        });
        scene.draw_image(brush.as_ref(), transform * to_affine(image.abs_transform()));
    });
}

fn to_affine(transform: usvg::Transform) -> Affine {
    let usvg::Transform {
        sx,
        kx,
        ky,
        sy,
        tx,
        ty,
    } = transform;
    Affine::new([sx, ky, kx, sy, tx, ty].map(f64::from))
}
