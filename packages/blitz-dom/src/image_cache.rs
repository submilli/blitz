//! The decoded images of a document's [list of available images].
//!
//! A response whose `Cache-Control` forbids storing it ([RFC 9111 §5.2.2.5])
//! is kept only while something still holds its decoded image, as Chrome
//! does: the entry is a weak reference, so once the last element lets go
//! of the image, a later request fetches it again.
//!
//! [list of available images]: https://html.spec.whatwg.org/multipage/images.html#list-of-available-images
//! [RFC 9111 §5.2.2.5]: https://www.rfc-editor.org/rfc/rfc9111#section-5.2.2.5
use crate::DecodedImageBudget;
use crate::image_request::ImageKey;
use crate::node::{ImageData, RasterImageData};
use linebender_resource_handle::WeakBlob;
use std::collections::HashMap;

/// How long a document may keep a decoded image for other requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageRetention {
    /// Kept until the cache's bounds refuse it, or the decoded image budget
    /// needs its memory for an image something shows.
    Store,
    /// The response is `no-store`: available only while held elsewhere.
    WhileHeld,
}
impl ImageRetention {
    pub(crate) fn of(metadata: &blitz_traits::net::ResponseMetadata) -> Self {
        if metadata.no_store {
            Self::WhileHeld
        } else {
            Self::Store
        }
    }
}

/// Decoded images kept for reuse. A page can reference unboundedly many
/// URLs, each under three CORS settings; past these limits images are not
/// cached, and later elements fetch them again.
pub(crate) const MAX_CACHED_IMAGES: usize = 512;
const MAX_CACHED_IMAGE_BYTES: usize = 64 * 1024 * 1024;

/// Entries are weak. A storable image is kept alive by the decoded image
/// budget instead ([`DecodedImageBudget::pin`]), which lets go of it before
/// it refuses a decode, so unshown cached images never break shown ones.
#[derive(Default)]
pub(crate) struct ImageCache {
    entries: HashMap<ImageKey, CacheEntry>,
}

struct CacheEntry {
    image: HeldImage,
    /// The image's [`decoded_bytes`], computed once.
    bytes: usize,
}

impl ImageCache {
    /// The decoded image for `key`, if it is still available.
    pub fn get(&self, key: &ImageKey) -> Option<ImageData> {
        self.entries.get(key)?.image.upgrade()
    }

    #[cfg(test)]
    pub fn contains_key(&self, key: &ImageKey) -> bool {
        self.get(key).is_some()
    }

    /// Offer `image` for reuse under `key`, a storable one pinned by
    /// `budget`. Entries whose holders have all let go are dropped first; live
    /// entries count toward both bounds.
    pub fn insert(
        &mut self,
        key: ImageKey,
        image: &ImageData,
        retention: ImageRetention,
        budget: &DecodedImageBudget,
    ) {
        let mut cached = 0usize;
        self.entries.retain(|_, entry| {
            let live = entry.image.upgrade().is_some();
            if live {
                cached = cached.saturating_add(entry.bytes);
            }
            live
        });
        let bytes = decoded_bytes(image);
        if self.entries.len() >= MAX_CACHED_IMAGES
            || cached.saturating_add(bytes) > MAX_CACHED_IMAGE_BYTES
        {
            return;
        }
        // Nothing can hold an image that did not decode.
        let Some(held) = HeldImage::new(image) else {
            return;
        };
        if retention == ImageRetention::Store {
            budget.pin(image);
        }
        self.entries.insert(key, CacheEntry { image: held, bytes });
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[cfg(test)]
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

/// A decoded image that does not keep its pixels or tree alive.
pub(crate) enum HeldImage {
    Raster {
        origin_clean: bool,
        width: u32,
        height: u32,
        data: WeakBlob<u8>,
    },
    #[cfg(feature = "svg")]
    Svg {
        tree: std::sync::Weak<usvg::Tree>,
        intrinsic_dimensions: crate::node::SvgIntrinsicDimensions,
        origin_clean: bool,
        retained_bytes: usize,
        subresources: crate::svg_subresources::SvgSubresources,
        nested_rasters: crate::svg_nested_rasters::WeakSvgNestedRasters,
    },
}
impl HeldImage {
    pub(crate) fn new(image: &ImageData) -> Option<Self> {
        match image {
            ImageData::Raster(raster) => Some(Self::Raster {
                origin_clean: raster.origin_clean,
                width: raster.width,
                height: raster.height,
                data: raster.data.downgrade(),
            }),
            #[cfg(feature = "svg")]
            ImageData::Svg(svg) => Some(Self::Svg {
                tree: std::sync::Arc::downgrade(&svg.tree),
                intrinsic_dimensions: svg.intrinsic_dimensions,
                origin_clean: svg.origin_clean,
                retained_bytes: svg.retained_bytes,
                subresources: svg.subresources.clone(),
                nested_rasters: svg.nested_rasters.downgrade(),
            }),
            ImageData::None => None,
        }
    }

    pub(crate) fn upgrade(&self) -> Option<ImageData> {
        match self {
            Self::Raster {
                origin_clean,
                width,
                height,
                data,
            } => Some(ImageData::Raster(RasterImageData {
                origin_clean: *origin_clean,
                width: *width,
                height: *height,
                data: data.upgrade()?,
            })),
            #[cfg(feature = "svg")]
            Self::Svg {
                tree,
                intrinsic_dimensions,
                origin_clean,
                retained_bytes,
                subresources,
                nested_rasters,
            } => Some(ImageData::Svg(crate::node::SvgImageData {
                tree: tree.upgrade()?,
                intrinsic_dimensions: *intrinsic_dimensions,
                origin_clean: *origin_clean,
                retained_bytes: *retained_bytes,
                subresources: subresources.clone(),
                nested_rasters: nested_rasters.upgrade()?,
            })),
        }
    }
}

/// What a cached image retains, as the decoded image budget charges it, so
/// stored entries can pin at most [`MAX_CACHED_IMAGE_BYTES`] of that budget.
fn decoded_bytes(image: &ImageData) -> usize {
    match image {
        ImageData::Raster(raster) => raster.data.data().len(),
        #[cfg(feature = "svg")]
        ImageData::Svg(svg) => svg.decoded_bytes(),
        ImageData::None => 0,
    }
}

#[cfg(test)]
mod tests;
