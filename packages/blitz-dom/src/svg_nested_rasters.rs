//! The raster images an SVG document nests (`<image>` elements with `data:`
//! URLs, at any nesting level), decoded once while it parses and drawn from
//! that decode by every paint of the document.
//!
//! usvg keeps nested rasters encoded in its tree, and anyrender_svg would
//! decode one on every paint without bounds, so an SVG image shown by many
//! elements would hold one decode per element until the frame ends. Decoded
//! here, a nested raster has the decode bounds of any image, is charged to the
//! decoded image budget once, and every paint draws the same pixels, which a
//! renderer's image cache (keyed by the pixel blob) also converts once.
use crate::DecodedImageBudget;
use crate::node::RasterImageData;
use crate::svg_walk::{Visit, walk};
use std::collections::HashMap;
use std::sync::{Arc, Weak};

/// Decoded bytes of the raster images one SVG document may nest, a quarter of
/// the decoded image budget.
const MAX_NESTED_SVG_IMAGE_BYTES: u64 = 32 * 1024 * 1024;

/// The decoded rasters of one SVG document, shared by its clones.
#[derive(Clone, Debug, Default)]
pub struct SvgNestedRasters {
    rasters: Arc<Rasters>,
    /// RGBA bytes of the distinct decodes, as the budget charges them.
    decoded_bytes: usize,
}

/// Decodes by the address of their body in the usvg tree. Text can hold one
/// raster per glyph, so lookups must not be linear.
type Rasters = HashMap<usize, NestedRaster>;

#[derive(Debug)]
struct NestedRaster {
    /// The body in the usvg tree, found by identity: holding it keeps its
    /// address from being reused by another tree's body, so an image of
    /// another tree never matches.
    encoded: Arc<Vec<u8>>,
    decoded: RasterImageData,
}

impl SvgNestedRasters {
    /// The decode of `image`, a raster of the document these belong to.
    /// `None` for a nested SVG document and for an image of another tree,
    /// such as one parsed without bounds.
    pub fn decoded(&self, image: &usvg::ImageKind) -> Option<&RasterImageData> {
        let encoded = raster_body(image)?;
        self.rasters
            .get(&address(encoded))
            .filter(|raster| Arc::ptr_eq(&raster.encoded, encoded))
            .map(|raster| &raster.decoded)
    }

    /// Bytes of the distinct decodes, as the decoded image budget charges them.
    pub(crate) fn decoded_bytes(&self) -> usize {
        self.decoded_bytes
    }

    /// A reference that keeps no decode alive, for the image cache.
    pub(crate) fn downgrade(&self) -> WeakSvgNestedRasters {
        WeakSvgNestedRasters {
            rasters: Arc::downgrade(&self.rasters),
            decoded_bytes: self.decoded_bytes,
        }
    }
}

/// See [`SvgNestedRasters::downgrade`].
#[derive(Debug)]
pub(crate) struct WeakSvgNestedRasters {
    rasters: Weak<Rasters>,
    decoded_bytes: usize,
}

impl WeakSvgNestedRasters {
    pub(crate) fn upgrade(&self) -> Option<SvgNestedRasters> {
        Some(SvgNestedRasters {
            rasters: self.rasters.upgrade()?,
            decoded_bytes: self.decoded_bytes,
        })
    }
}

/// Decodes the nested rasters of one SVG document once it is parsed. usvg
/// resolves an image before deciding to keep it (an `<image>` without area is
/// skipped), and makes some rasters without its resolver (the colour bitmap
/// glyphs of flattened text), so only the finished tree says what to decode.
#[derive(Clone)]
pub(crate) struct NestedRasterDecoder {
    budget: DecodedImageBudget,
    purpose: RasterPurpose,
}

/// What a parsed document is for, which decides whether its rasters decode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RasterPurpose {
    /// It is painted: rasters decode.
    Paint,
    /// Only its geometry is measured: nothing decodes.
    Measure,
}

impl NestedRasterDecoder {
    pub(crate) fn new(budget: &DecodedImageBudget, purpose: RasterPurpose) -> Self {
        Self {
            budget: budget.clone(),
            purpose,
        }
    }

    /// Whether the resolver may give usvg `image`: a raster whose header is
    /// readable and within the decode bounds and the document's bound. Nested
    /// SVG documents are always kept.
    pub(crate) fn admits(image: &usvg::ImageKind) -> bool {
        raster_body(image).is_none_or(|encoded| {
            crate::image_decode::bounded_rgba_bytes(encoded)
                .is_some_and(|rgba| rgba <= MAX_NESTED_SVG_IMAGE_BYTES)
        })
    }

    /// Decode the rasters the finished `tree` keeps, in document order:
    /// equal bodies share one decode, and each is charged to the budget once.
    /// A raster past the document's bound or the budget, or undecodable, has
    /// no decode and draws nothing. A measured document decodes nothing.
    pub(crate) fn decode(&self, tree: &usvg::Tree) -> SvgNestedRasters {
        if self.purpose == RasterPurpose::Measure {
            return SvgNestedRasters::default();
        }
        let mut rasters = Rasters::new();
        // `Arc<Vec<u8>>` hashes its content, so equal bodies share one decode.
        let mut by_content: HashMap<&Arc<Vec<u8>>, Option<RasterImageData>> = HashMap::new();
        let mut bytes = 0u64;
        for encoded in kept_bodies(tree) {
            let decoded = by_content
                .entry(encoded)
                .or_insert_with(|| self.decode_within(encoded, &mut bytes));
            if let Some(decoded) = decoded {
                let encoded = Arc::clone(encoded);
                let decoded = decoded.clone();
                rasters.insert(address(&encoded), NestedRaster { encoded, decoded });
            }
        }
        let decoded_bytes = by_content
            .values()
            .flatten()
            .map(|decoded| decoded.data.len())
            .fold(0, usize::saturating_add);
        SvgNestedRasters {
            rasters: Arc::new(rasters),
            decoded_bytes,
        }
    }

    /// Decode `encoded` if its RGBA bytes keep `bytes` within the document's
    /// bound, and add them.
    fn decode_within(&self, encoded: &[u8], bytes: &mut u64) -> Option<RasterImageData> {
        let rgba = crate::image_decode::bounded_rgba_bytes(encoded)?;
        let total = bytes.saturating_add(rgba);
        if total > MAX_NESTED_SVG_IMAGE_BYTES {
            return None;
        }
        let decoded = crate::image_decode::decode_raster_charged(encoded, &self.budget)?;
        *bytes = total;
        Some(decoded)
    }
}

/// The distinct raster bodies `tree` keeps, in document order.
fn kept_bodies(tree: &usvg::Tree) -> Vec<&Arc<Vec<u8>>> {
    let mut seen = std::collections::HashSet::new();
    let mut kept = Vec::new();
    walk(tree, |visit| {
        if let Visit::Node(usvg::Node::Image(image)) = visit
            && let Some(body) = raster_body(image.kind())
            && seen.insert(address(body))
        {
            kept.push(body);
        }
    });
    kept
}

/// A body's identity while something holds it.
fn address(body: &Arc<Vec<u8>>) -> usize {
    Arc::as_ptr(body) as usize
}

/// The encoded body of a raster image; `None` for a nested SVG document.
fn raster_body(image: &usvg::ImageKind) -> Option<&Arc<Vec<u8>>> {
    match image {
        usvg::ImageKind::JPEG(data)
        | usvg::ImageKind::PNG(data)
        | usvg::ImageKind::GIF(data)
        | usvg::ImageKind::WEBP(data) => Some(data),
        usvg::ImageKind::SVG(_) => None,
    }
}

#[cfg(test)]
mod tests;
