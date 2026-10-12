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
use std::sync::{Arc, Mutex, PoisonError, Weak};

/// Decoded bytes of the raster images one SVG document may nest, a quarter of
/// the decoded image budget.
const MAX_NESTED_SVG_IMAGE_BYTES: u64 = 32 * 1024 * 1024;

/// The decoded rasters of one SVG document, shared by its clones.
#[derive(Clone, Debug)]
pub struct SvgNestedRasters {
    rasters: Arc<[NestedRaster]>,
    /// RGBA bytes of the distinct decodes, as the budget charges them.
    decoded_bytes: usize,
}

#[derive(Debug)]
struct NestedRaster {
    /// The body in the usvg tree, found by identity: holding it keeps its
    /// address from being reused by another tree's body, so an image of
    /// another tree never matches.
    encoded: Arc<Vec<u8>>,
    decoded: RasterImageData,
}

impl Default for SvgNestedRasters {
    fn default() -> Self {
        Self {
            rasters: Arc::new([]),
            decoded_bytes: 0,
        }
    }
}

impl SvgNestedRasters {
    /// The decode of `image`, a raster of the document these belong to.
    /// `None` for a nested SVG document and for an image of another tree,
    /// such as one parsed without bounds.
    pub fn decoded(&self, image: &usvg::ImageKind) -> Option<&RasterImageData> {
        let encoded = raster_body(image)?;
        self.rasters
            .iter()
            .find(|raster| Arc::ptr_eq(&raster.encoded, encoded))
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
    rasters: Weak<[NestedRaster]>,
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

/// Decodes the nested rasters of one SVG document as usvg's resolver meets
/// them. Shared with the resolver, which usvg requires to be `Send + Sync`.
#[derive(Clone)]
pub(crate) struct NestedRasterDecoder {
    budget: DecodedImageBudget,
    purpose: RasterPurpose,
    state: Arc<Mutex<DecoderState>>,
}

/// What a parsed document is for, which decides whether its rasters decode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RasterPurpose {
    /// It is painted: rasters decode.
    Paint,
    /// Only its geometry is measured: rasters are admitted by their headers
    /// within the same bounds, and nothing decodes.
    Measure,
}

#[derive(Default)]
struct DecoderState {
    rasters: Vec<NestedRaster>,
    /// RGBA bytes of the distinct decodes, against
    /// [`MAX_NESTED_SVG_IMAGE_BYTES`].
    bytes: u64,
}

impl NestedRasterDecoder {
    pub(crate) fn new(budget: &DecodedImageBudget, purpose: RasterPurpose) -> Self {
        Self {
            budget: budget.clone(),
            purpose,
            state: Default::default(),
        }
    }

    /// Whether the tree may keep the `image` usvg resolved: a raster is
    /// decoded, or refused and must be left out when it is past the
    /// document's bound or the decode bounds, past the budget, or
    /// undecodable. A body equal to one decoded before shares its decode.
    /// Nested SVG documents need no decode and are always kept.
    pub(crate) fn admit(&self, image: &usvg::ImageKind) -> bool {
        raster_body(image).is_none_or(|encoded| self.admit_body(encoded))
    }

    fn admit_body(&self, encoded: &Arc<Vec<u8>>) -> bool {
        let mut state = self.lock();
        let earlier = state
            .rasters
            .iter()
            .find(|raster| raster.encoded == *encoded)
            .map(|raster| raster.decoded.clone());
        if let Some(decoded) = earlier {
            state.rasters.push(NestedRaster {
                encoded: Arc::clone(encoded),
                decoded,
            });
            return true;
        }
        let Some(rgba) = crate::image_decode::bounded_rgba_bytes(encoded) else {
            return false;
        };
        let bytes = state.bytes.saturating_add(rgba);
        if bytes > MAX_NESTED_SVG_IMAGE_BYTES {
            return false;
        }
        if self.purpose == RasterPurpose::Measure {
            state.bytes = bytes;
            return true;
        }
        let Some(decoded) = crate::image_decode::decode_raster_charged(encoded, &self.budget)
        else {
            return false;
        };
        state.bytes = bytes;
        state.rasters.push(NestedRaster {
            encoded: Arc::clone(encoded),
            decoded,
        });
        true
    }

    /// The decodes of the rasters `tree` keeps. usvg resolves an image before
    /// deciding to keep it (an `<image>` without area is skipped, a nested
    /// document can still fail), so the decodes of images it dropped are
    /// released here rather than held, uncharged bodies and all. Rasters
    /// usvg made itself, the colour bitmap glyphs of flattened text, never
    /// reach the resolver and decode here, within the same bounds; one that
    /// is refused draws nothing.
    pub(crate) fn finish(&self, tree: &usvg::Tree) -> SvgNestedRasters {
        let mut kept: Vec<&Arc<Vec<u8>>> = Vec::new();
        crate::svg_bounds::walk(tree, |visit| {
            if let crate::svg_bounds::Visit::Node(usvg::Node::Image(image)) = visit {
                kept.extend(raster_body(image.kind()));
            }
        });
        for body in &kept {
            if !self.holds(body) {
                self.admit_body(body);
            }
        }
        let mut rasters = std::mem::take(&mut self.lock().rasters);
        rasters.retain(|raster| kept.iter().any(|body| Arc::ptr_eq(body, &raster.encoded)));
        let decoded_bytes = distinct_bytes(&rasters);
        SvgNestedRasters {
            rasters: rasters.into(),
            decoded_bytes,
        }
    }

    fn holds(&self, body: &Arc<Vec<u8>>) -> bool {
        let state = self.lock();
        state
            .rasters
            .iter()
            .any(|raster| Arc::ptr_eq(&raster.encoded, body))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, DecoderState> {
        // Only plain data is behind the lock; a panic leaves it usable.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The bytes of the distinct decodes of `rasters` (equal bodies share one).
fn distinct_bytes(rasters: &[NestedRaster]) -> usize {
    let mut bytes = 0usize;
    for (index, raster) in rasters.iter().enumerate() {
        let id = raster.decoded.data.id();
        if rasters[..index].iter().all(|r| r.decoded.data.id() != id) {
            bytes = bytes.saturating_add(raster.decoded.data.len());
        }
    }
    bytes
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
