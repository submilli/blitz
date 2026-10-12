//! SVG image data and CSS intrinsic sizing for SVG.

use std::sync::Arc;

use crate::svg_nested_rasters::{NestedRasterDecoder, RasterPurpose, SvgNestedRasters};
use crate::svg_root_size;
use crate::svg_subresources::SubresourceRecorder;
use usvg::roxmltree;

/// Dimensions declared on the root `<svg>` element, before any resolution.
///
/// Unlike [`usvg::Tree::size`], which always produces a concrete size, this
/// preserves what the SVG actually declared: absent attributes are `None` and
/// percentage lengths are kept unresolved.
#[derive(Debug, Clone, Copy, Default)]
pub struct SvgIntrinsicDimensions {
    /// The root `width` attribute, if declared. Percentages are unresolved.
    pub width: Option<svgtypes::Length>,
    /// The root `height` attribute, if declared. Percentages are unresolved.
    pub height: Option<svgtypes::Length>,
    /// The root `viewBox` width/height, if declared and valid.
    pub view_box_size: Option<(f32, f32)>,
    /// Whether the root declared a `viewBox` with a zero width or height,
    /// which disables rendering of the element per the SVG spec.
    pub degenerate_view_box: bool,
    /// The root `preserveAspectRatio`, which places the `viewBox` in the
    /// viewport (`xMidYMid meet` when absent or invalid).
    pub preserve_aspect_ratio: svgtypes::AspectRatio,
}

impl SvgIntrinsicDimensions {
    /// Extract the `width`/`height`/`viewBox` attributes declared on the root
    /// element of an already-parsed SVG document, with absent attributes as
    /// `None` and percentages unresolved. [`usvg::Tree`] does not preserve
    /// these (it always resolves to a concrete size), so they are read from
    /// the XML document here.
    pub fn from_xmltree(doc: &roxmltree::Document) -> Self {
        let root = doc.root_element();

        let parse_length = |name: &str| -> Option<svgtypes::Length> {
            svg_root_size::root_dimension(doc, name)?
                .value()
                .parse::<svgtypes::Length>()
                .ok()
        };
        // Parsed manually rather than via `svgtypes::ViewBox`, which rejects
        // zero sizes: a zero `viewBox` width/height is distinguished from an
        // invalid `viewBox` as it disables rendering of the element.
        let view_box_dims = root.attribute("viewBox").and_then(|s| {
            let mut numbers = svgtypes::NumberListParser::from(s);
            let _x = numbers.next()?.ok()?;
            let _y = numbers.next()?.ok()?;
            let w = numbers.next()?.ok()? as f32;
            let h = numbers.next()?.ok()? as f32;
            (numbers.next().is_none() && w.is_finite() && h.is_finite() && w >= 0.0 && h >= 0.0)
                .then_some((w, h))
        });
        let view_box_size = view_box_dims.filter(|&(w, h)| w > 0.0 && h > 0.0);
        let degenerate_view_box = view_box_dims.is_some_and(|(w, h)| w == 0.0 || h == 0.0);

        Self {
            width: parse_length("width"),
            height: parse_length("height"),
            view_box_size,
            degenerate_view_box,
            preserve_aspect_ratio: root
                .attribute("preserveAspectRatio")
                .and_then(|value| value.parse().ok())
                .unwrap_or_default(),
        }
    }
}

/// What one parse of an SVG image collects: the images it loads itself,
/// which usvg's resolver must consult (`admits`, `scan_nested`; see
/// [`crate::svg_subresources`]), and the decodes of the rasters the parsed
/// tree keeps (see [`crate::svg_nested_rasters`]).
pub(crate) struct SvgParseCollectors {
    pub(crate) recorder: SubresourceRecorder,
    rasters: NestedRasterDecoder,
    /// What converting the document and those it nests holds from the budget
    /// until it is parsed (see [`crate::svg_bounds`]).
    pub(crate) conversions: crate::svg_bounds::ConversionReservations,
}

impl SvgParseCollectors {
    pub(crate) fn new(
        shown: crate::svg_subresources::NestedImages,
        budget: &crate::DecodedImageBudget,
        purpose: RasterPurpose,
    ) -> Self {
        Self {
            recorder: SubresourceRecorder::new(shown),
            rasters: NestedRasterDecoder::new(budget, purpose),
            conversions: crate::svg_bounds::ConversionReservations::new(budget),
        }
    }

    /// What the parse collected, decoding the rasters the parsed `tree` keeps.
    fn finish(
        &self,
        tree: &usvg::Tree,
    ) -> (crate::svg_subresources::SvgSubresources, SvgNestedRasters) {
        (self.recorder.finish(), self.rasters.decode(tree))
    }
}

/// A parsed SVG image.
///
/// usvg always resolves the root `<svg>` to a concrete [`usvg::Tree::size`],
/// falling back to the `viewBox` size when `width`/`height` are absent or given
/// as percentages. For CSS sizing purposes, however, such an SVG has *no*
/// intrinsic width/height (only an intrinsic aspect ratio). The accessors on
/// this type resolve the CSS intrinsic dimensions from the declared root
/// attributes, which are captured at parse time.
#[derive(Debug, Clone)]
pub struct SvgImageData {
    /// The parsed SVG tree.
    pub tree: Arc<usvg::Tree>,
    /// The dimensions declared on the root `<svg>` element.
    pub intrinsic_dimensions: SvgIntrinsicDimensions,
    /// Readback authority travels with the image, as for [`super::RasterImageData`].
    pub origin_clean: bool,
    /// What keeping the document costs the decoded image budget, estimated
    /// once at parse (see [`crate::svg_bounds::retained_bytes`]).
    pub(crate) retained_bytes: usize,
    /// What the SVG image loads itself (see [`crate::svg_subresources`]).
    pub(crate) subresources: crate::svg_subresources::SvgSubresources,
    /// The decodes of the rasters it nests, which paints draw.
    pub(crate) nested_rasters: SvgNestedRasters,
}

impl SvgImageData {
    /// Parse an SVG image from raw data, capturing both the rendered
    /// [`usvg::Tree`] and the declared root dimensions. The XML is parsed
    /// once, or twice for a root with an empty size (see
    /// [`crate::svg_root_size`]).
    ///
    /// Unlike [`usvg::Tree::from_data`], `data` is never inflated. Only for
    /// trusted data: nothing bounds what it expands to. Page data is parsed
    /// through `svg_bounds::parse_svg_image`, which inflates SVGZ and bounds
    /// the document's expansion first.
    pub fn from_data(data: &[u8], options: &usvg::Options) -> Result<Self, usvg::Error> {
        Self::parse(data, options, None).map_err(|failure| match failure {
            ParseFailure::Usvg(error) => error,
            // Only collectors refuse documents.
            ParseFailure::Refused(_) => usvg::Error::ElementsLimitReached,
        })
    }

    /// [`Self::from_data`] with `collectors`, whose recorder the resolver of
    /// `options` must consult, and which reserve what converting the document
    /// allocates (see [`SvgParseCollectors`]).
    pub(crate) fn parse_recording(
        data: &[u8],
        options: &usvg::Options,
        collectors: &SvgParseCollectors,
    ) -> Result<Self, crate::svg_bounds::SvgRefusal> {
        use crate::svg_bounds::SvgRefusal;
        Self::parse(data, options, Some(collectors)).map_err(|failure| match failure {
            ParseFailure::Usvg(_) => SvgRefusal::Invalid,
            ParseFailure::Refused(refusal) => refusal,
        })
    }

    fn parse(
        data: &[u8],
        options: &usvg::Options,
        collectors: Option<&SvgParseCollectors>,
    ) -> Result<Self, ParseFailure> {
        let text = std::str::from_utf8(data).map_err(|_| usvg::Error::NotAnUtf8Str)?;
        let declared = svg_root_size::parse_xml(text)?;
        let intrinsic_dimensions = SvgIntrinsicDimensions::from_xmltree(&declared);
        let resized = svg_root_size::chrome_root_size(text, &declared);
        let doc = match resized.as_deref() {
            None => declared,
            Some(resized) => {
                // Only one document is held at a time, as reserved.
                drop(declared);
                svg_root_size::parse_xml(resized)?
            }
        };
        let _levels = match collectors {
            Some(collectors) => {
                collectors.recorder.scan_document(&doc);
                let levels = collectors.conversions.reserve(&doc);
                Some(levels.map_err(ParseFailure::Refused)?)
            }
            None => None,
        };
        let tree = usvg::Tree::from_xmltree(&doc, options)?;
        let (subresources, nested_rasters) = collectors
            .map(|collectors| collectors.finish(&tree))
            .unwrap_or_default();
        let retained_bytes =
            crate::svg_bounds::retained_bytes(&tree).saturating_add(subresources.retained_bytes());
        Ok(Self {
            retained_bytes,
            tree: Arc::new(tree),
            intrinsic_dimensions,
            origin_clean: false,
            subresources,
            nested_rasters,
        })
    }

    /// The decodes of the raster images the document nests, which paints
    /// draw instead of decoding them again. Empty for a document parsed by
    /// [`Self::from_data`], which decodes none.
    pub fn nested_rasters(&self) -> &SvgNestedRasters {
        &self.nested_rasters
    }

    /// Bytes the decoded image budget charges for the document and its
    /// nested rasters.
    pub(crate) fn decoded_bytes(&self) -> usize {
        self.retained_bytes
            .saturating_add(self.nested_rasters.decoded_bytes())
    }

    /// The intrinsic width in CSS px, present only when the root `<svg>`
    /// declared an absolute (non-percentage) `width`. A width that is not
    /// positive is zero, as in Chrome.
    pub fn intrinsic_width(&self) -> Option<f32> {
        absolute_dimension(self.intrinsic_dimensions.width, self.tree.size().width())
    }

    /// The intrinsic height in CSS px, present only when the root `<svg>`
    /// declared an absolute (non-percentage) `height`. A height that is not
    /// positive is zero, as in Chrome.
    pub fn intrinsic_height(&self) -> Option<f32> {
        absolute_dimension(self.intrinsic_dimensions.height, self.tree.size().height())
    }

    /// Whether the image renders its content: a `viewBox` with a zero width
    /// or height disables rendering ([SVG 2]).
    ///
    /// [SVG 2]: https://svgwg.org/svg2-draft/coords.html#ViewBoxAttribute
    pub fn renders_content(&self) -> bool {
        !self.intrinsic_dimensions.degenerate_view_box
    }

    /// The aspect ratio of the root `<svg>`'s `viewBox`, if it declares one.
    pub fn viewbox_aspect_ratio(&self) -> Option<f32> {
        self.intrinsic_dimensions.view_box_size.map(|(w, h)| w / h)
    }

    /// The root `width` attribute resolved against a containing block width:
    /// percentages resolve against the containing block (`None` if it is
    /// indefinite) and an absent attribute is `None`.
    ///
    /// This is only appropriate for an inline `<svg>` element, where the
    /// attributes behave as presentation attributes. SVG used as an image
    /// (e.g. `<img src>` or a background) must use [`Self::intrinsic_width`],
    /// as its intrinsic dimensions are context-free per CSS.
    pub fn resolved_width(&self, container_width: Option<f32>) -> Option<f32> {
        use svgtypes::LengthUnit;
        match self.intrinsic_dimensions.width {
            Some(len) if len.unit == LengthUnit::Percent => {
                container_width.map(|cw| cw * (len.number as f32) / 100.0)
            }
            _ => self.intrinsic_width(),
        }
    }

    /// The root `height` attribute resolved against a containing block height.
    /// See [`Self::resolved_width`].
    pub fn resolved_height(&self, container_height: Option<f32>) -> Option<f32> {
        use svgtypes::LengthUnit;
        match self.intrinsic_dimensions.height {
            Some(len) if len.unit == LengthUnit::Percent => {
                container_height.map(|ch| ch * (len.number as f32) / 100.0)
            }
            _ => self.intrinsic_height(),
        }
    }

    /// The intrinsic aspect ratio of the SVG: the ratio of its declared
    /// `width`/`height` when both are absolute and positive, otherwise the
    /// `viewBox` ratio, otherwise the ratio of the resolved
    /// [`usvg::Tree::size`] (which is always positive).
    pub fn aspect_ratio(&self) -> f32 {
        match (self.intrinsic_width(), self.intrinsic_height()) {
            (Some(w), Some(h)) if w > 0.0 && h > 0.0 => w / h,
            _ => self.viewbox_aspect_ratio().unwrap_or_else(|| {
                let size = self.tree.size();
                size.width() / size.height()
            }),
        }
    }

    /// The concrete object size of the SVG as an image without a specified
    /// size: the CSS [default sizing algorithm] against `default`. A missing
    /// natural dimension follows the other one through the `viewBox` ratio, or
    /// is the default's; without either, the `viewBox` ratio is contained in
    /// `default`, and without a ratio the size is `default`. Unlike
    /// [`Self::intrinsic_size`], there is no fallback to the resolved tree size.
    ///
    /// [default sizing algorithm]: https://drafts.csswg.org/css-images-3/#default-sizing
    pub fn concrete_size(&self, default: (f32, f32)) -> (f32, f32) {
        let ratio = self.viewbox_aspect_ratio();
        match (self.intrinsic_width(), self.intrinsic_height()) {
            (Some(w), Some(h)) => (w, h),
            (Some(w), None) => (w, ratio.map_or(default.1, |ratio| w / ratio)),
            (None, Some(h)) => (ratio.map_or(default.0, |ratio| h * ratio), h),
            (None, None) => ratio.map_or(default, |ratio| {
                if default.0 / default.1 > ratio {
                    (default.1 * ratio, default.1)
                } else {
                    (default.0, default.0 / ratio)
                }
            }),
        }
    }

    /// The intrinsic dimensions of the SVG resolved per CSS replaced element
    /// sizing: a missing dimension is computed from the declared one and the
    /// intrinsic aspect ratio; if neither is declared, the resolved
    /// [`usvg::Tree::size`] is used as a fallback.
    pub fn intrinsic_size(&self) -> (f32, f32) {
        let aspect_ratio = self.aspect_ratio();
        match (self.intrinsic_width(), self.intrinsic_height()) {
            (Some(w), Some(h)) => (w, h),
            (Some(w), None) => (w, w / aspect_ratio),
            (None, Some(h)) => (h * aspect_ratio, h),
            (None, None) => {
                // No intrinsic dimensions. If there is an intrinsic aspect ratio, apply
                // the CSS default sizing algorithm: contain within the default object
                // size of 300x150. Otherwise fall back to the resolved tree size.
                if self.viewbox_aspect_ratio().is_some() {
                    let scale = (300.0 / aspect_ratio).min(150.0);
                    (scale * aspect_ratio, scale)
                } else {
                    let size = self.tree.size();
                    (size.width(), size.height())
                }
            }
        }
    }
}

/// Why [`SvgImageData::parse`] made no document.
enum ParseFailure {
    Usvg(usvg::Error),
    Refused(crate::svg_bounds::SvgRefusal),
}

impl From<usvg::Error> for ParseFailure {
    fn from(error: usvg::Error) -> Self {
        Self::Usvg(error)
    }
}

/// A declared absolute dimension in CSS px, `None` for a percentage or a
/// missing dimension. One that is not positive is zero, as Chrome reports it
/// (`width="-5"` too).
///
/// `resolved` is usvg's tree size, which usvg replaces with its content's
/// bounds when the other dimension is missing or a percentage and there is
/// no `viewBox`. So absolute units are converted here, at 96 px per inch
/// ([absolute lengths]); only `em` and `ex` take `resolved`, as usvg knows
/// the root's font size. A conversion that overflows takes `resolved` too,
/// which usvg keeps finite (it refuses such a root, read from the same
/// attribute; see [`crate::svg_root_size`]).
///
/// [absolute lengths]: https://drafts.csswg.org/css-values-4/#absolute-lengths
fn absolute_dimension(declared: Option<svgtypes::Length>, resolved: f32) -> Option<f32> {
    use svgtypes::LengthUnit as Unit;
    let declared = declared?;
    let number = declared.number as f32;
    let px = match declared.unit {
        Unit::Percent => return None,
        _ if svg_root_size::is_empty_length(declared) => 0.0,
        Unit::None | Unit::Px => number,
        Unit::In => number * 96.0,
        Unit::Cm => number * 96.0 / 2.54,
        Unit::Mm => number * 96.0 / 25.4,
        Unit::Pt => number * 96.0 / 72.0,
        Unit::Pc => number * 96.0 / 6.0,
        Unit::Em | Unit::Ex => resolved,
    };
    Some(if px.is_finite() { px } else { resolved })
}
