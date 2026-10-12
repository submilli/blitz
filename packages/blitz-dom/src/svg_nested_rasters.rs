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
pub struct SvgNestedRasters(Arc<[NestedRaster]>);

#[derive(Debug)]
struct NestedRaster {
    /// The body in the usvg tree, which the tree holds as long as these.
    encoded: Arc<Vec<u8>>,
    decoded: RasterImageData,
}

impl Default for SvgNestedRasters {
    fn default() -> Self {
        Self(Arc::new([]))
    }
}

impl SvgNestedRasters {
    /// The decode of `image`, a raster of the document these belong to.
    /// `None` for a nested SVG document and for an image of another tree,
    /// such as one parsed without bounds.
    pub fn decoded(&self, image: &usvg::ImageKind) -> Option<&RasterImageData> {
        let encoded = raster_body(image)?;
        self.0
            .iter()
            .find(|raster| Arc::ptr_eq(&raster.encoded, encoded))
            .map(|raster| &raster.decoded)
    }

    /// Bytes of the distinct decodes, as the decoded image budget charges them.
    pub(crate) fn decoded_bytes(&self) -> usize {
        self.0
            .iter()
            .enumerate()
            .filter(|(index, raster)| {
                let id = raster.decoded.data.id();
                !self.0[..*index].iter().any(|r| r.decoded.data.id() == id)
            })
            .map(|(_, raster)| raster.decoded.data.len())
            .fold(0, usize::saturating_add)
    }

    /// A reference that keeps no decode alive, for the image cache.
    pub(crate) fn downgrade(&self) -> WeakSvgNestedRasters {
        WeakSvgNestedRasters(Arc::downgrade(&self.0))
    }
}

/// See [`SvgNestedRasters::downgrade`].
#[derive(Debug)]
pub(crate) struct WeakSvgNestedRasters(Weak<[NestedRaster]>);

impl WeakSvgNestedRasters {
    pub(crate) fn upgrade(&self) -> Option<SvgNestedRasters> {
        self.0.upgrade().map(SvgNestedRasters)
    }
}

/// Decodes the nested rasters of one SVG document as usvg's resolver meets
/// them. Shared with the resolver, which usvg requires to be `Send + Sync`.
#[derive(Clone)]
pub(crate) struct NestedRasterDecoder {
    budget: DecodedImageBudget,
    state: Arc<Mutex<DecoderState>>,
}

#[derive(Default)]
struct DecoderState {
    rasters: Vec<NestedRaster>,
    /// RGBA bytes of the distinct decodes, against
    /// [`MAX_NESTED_SVG_IMAGE_BYTES`].
    bytes: u64,
}

impl NestedRasterDecoder {
    pub(crate) fn new(budget: &DecodedImageBudget) -> Self {
        Self {
            budget: budget.clone(),
            state: Default::default(),
        }
    }

    /// Decode the raster `image` usvg resolved; `false` when it is refused
    /// and must be left out: past the document's bound or the decode bounds,
    /// past the budget, or undecodable. A body equal to one decoded before
    /// shares its decode. Nested SVG documents need no decode.
    pub(crate) fn decode(&self, image: &usvg::ImageKind) -> bool {
        let Some(encoded) = raster_body(image) else {
            return true;
        };
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

    /// The rasters decoded so far.
    pub(crate) fn finish(&self) -> SvgNestedRasters {
        SvgNestedRasters(std::mem::take(&mut self.lock().rasters).into())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, DecoderState> {
        // Only plain data is behind the lock; a panic leaves it usable.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
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
