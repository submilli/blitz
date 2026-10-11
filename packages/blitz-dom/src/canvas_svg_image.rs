//! What `feImage` primitives draw, captured with Canvas filter snapshots.
//!
//! A snapshot holds no DOM handles: fetched images are shared decoded data, and
//! a referenced element is serialized into its own parsed tree.

use crate::canvas_svg_filter::CanvasSvgLimit;
use crate::fe_image::FeImageTarget;
use crate::node::{ImageData, RasterImageData, SvgImageData};
use crate::{BaseDocument, NodeId};
use std::sync::Arc;

/// An `feImage` primitive's source and placement.
#[derive(Clone, Debug)]
pub struct CanvasSvgImage {
    pub source: CanvasSvgImageSource,
    /// Its `preserveAspectRatio`.
    pub aspect: svgtypes::AspectRatio,
}

/// What an `feImage` draws.
#[derive(Clone, Debug)]
pub enum CanvasSvgImageSource {
    /// Transparent black: nothing loaded, failed, or a missing element.
    None,
    /// Decoded straight RGBA8 pixels.
    Raster(RasterImageData),
    /// An SVG document used as an image.
    Svg(SvgImageData),
    /// A same-document element's subtree.
    Element(CanvasSvgElement),
}

/// A referenced element as Chrome paints it (Blink's
/// `FEImage::CreateImageFilterForLayoutObject`): its subtree's content with
/// computed styles but without transforms or effects, culled to its visual
/// rectangle with the transforms applied.
#[derive(Clone, Debug)]
pub struct CanvasSvgElement {
    pub tree: Arc<usvg::Tree>,
    /// The painted bounds, stroke included, in the parent's user space.
    pub bounds: usvg::NonZeroRect,
    /// The viewport Chrome maps onto the primitive subregion, when the
    /// element's visible content depends on it (Blink's
    /// `FEImage::SourceToDestinationTransform`).
    pub viewport: Option<[f32; 2]>,
}

impl BaseDocument {
    /// Capture what `fe_image` draws, and whether its result taints a canvas.
    /// As in Blink (`SVGFEImageElement::TaintsOrigin`), only a fetched image
    /// can be readable: element references and missing or unresolvable
    /// `href`s taint, and fetched images taint when they are not CORS
    /// same-origin. A pending or failed fetch draws nothing and does not taint.
    pub(crate) fn snapshot_fe_image(
        &self,
        fe_image: NodeId,
        inline: &mut crate::fe_image::InlineDecodes,
    ) -> Result<(CanvasSvgImage, bool), CanvasSvgLimit> {
        let aspect = self.nodes[fe_image]
            .attr(crate::local_name!("preserveAspectRatio"))
            .and_then(|value| value.parse().ok())
            .unwrap_or_default();
        let (source, tainted) = match self.fe_image_target(fe_image, inline) {
            FeImageTarget::Unfetched => (CanvasSvgImageSource::None, true),
            FeImageTarget::Fragment(id) => (self.referenced_element(fe_image, &id)?, true),
            FeImageTarget::Fetched(None) => (CanvasSvgImageSource::None, false),
            FeImageTarget::Fetched(Some(ImageData::Raster(raster))) => {
                let tainted = !raster.origin_clean;
                (CanvasSvgImageSource::Raster(raster), tainted)
            }
            FeImageTarget::Fetched(Some(ImageData::Svg(svg))) => {
                let tainted = !svg.origin_clean;
                (CanvasSvgImageSource::Svg(svg), tainted)
            }
            FeImageTarget::Fetched(Some(ImageData::None)) => (CanvasSvgImageSource::None, false),
        };
        Ok((CanvasSvgImage { source, aspect }, tainted))
    }

    /// The rendered SVG element `id` names in `fe_image`'s tree, as a parsed
    /// subtree. Like Blink, an element without a box (not SVG, or not
    /// displayed) draws nothing.
    fn referenced_element(
        &self,
        fe_image: NodeId,
        id: &str,
    ) -> Result<CanvasSvgImageSource, CanvasSvgLimit> {
        let Some(target) = self.find_svg_reference(fe_image, id)? else {
            return Ok(CanvasSvgImageSource::None);
        };
        let node = &self.nodes[target];
        let Some(style) = node.primary_styles().map(|style| style.clone()) else {
            return Ok(CanvasSvgImageSource::None);
        };
        let svg = node
            .element_data()
            .is_some_and(|element| element.name.ns == markup5ever::ns!(svg));
        if !svg || style.clone_display().is_none() {
            return Ok(CanvasSvgImageSource::None);
        }
        // Past the serialization bounds the reference draws nothing (it still
        // taints), rather than failing the whole draw.
        Ok(match self.capture_svg_element(target) {
            Ok(Some(element)) => CanvasSvgImageSource::Element(element),
            Ok(None) | Err(CanvasSvgLimit) => CanvasSvgImageSource::None,
        })
    }
}
