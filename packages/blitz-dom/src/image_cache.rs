//! The decoded images of a document's [list of available images].
//!
//! A response whose `Cache-Control` forbids storing it ([RFC 9111 §5.2.2.5])
//! is kept only while something still holds its decoded image, as Chrome
//! does: the entry is a weak reference, so once the last element lets go
//! of the image, a later request fetches it again.
//!
//! [list of available images]: https://html.spec.whatwg.org/multipage/images.html#list-of-available-images
//! [RFC 9111 §5.2.2.5]: https://www.rfc-editor.org/rfc/rfc9111#section-5.2.2.5
use crate::image_request::ImageKey;
use crate::node::{ImageData, RasterImageData};
use linebender_resource_handle::WeakBlob;
use std::collections::HashMap;

/// How long a document may keep a decoded image for other requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageRetention {
    /// Kept until the cache's bounds refuse it.
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

#[derive(Default)]
pub(crate) struct ImageCache {
    /// Each entry with its [`decoded_bytes`], computed once.
    entries: HashMap<ImageKey, (Entry, usize)>,
}
impl ImageCache {
    /// The decoded image for `key`, if it is still available.
    pub fn get(&self, key: &ImageKey) -> Option<ImageData> {
        self.entries.get(key)?.0.upgrade()
    }

    #[cfg(test)]
    pub fn contains_key(&self, key: &ImageKey) -> bool {
        self.get(key).is_some()
    }

    /// Offer `image` for reuse under `key`. Entries whose holders have all
    /// let go are dropped first; live entries count toward both bounds.
    pub fn insert(&mut self, key: ImageKey, image: &ImageData, retention: ImageRetention) {
        let mut cached = 0usize;
        self.entries.retain(|_, (entry, bytes)| {
            let live = entry.upgrade().is_some();
            if live {
                cached = cached.saturating_add(*bytes);
            }
            live
        });
        let bytes = decoded_bytes(image);
        if self.entries.len() >= MAX_CACHED_IMAGES
            || cached.saturating_add(bytes) > MAX_CACHED_IMAGE_BYTES
        {
            return;
        }
        let entry = match retention {
            ImageRetention::Store => Entry::Stored(image.clone()),
            ImageRetention::WhileHeld => match HeldImage::new(image) {
                Some(held) => Entry::Held(held),
                // Nothing can hold an image that did not decode.
                None => return,
            },
        };
        self.entries.insert(key, (entry, bytes));
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

enum Entry {
    Stored(ImageData),
    Held(HeldImage),
}
impl Entry {
    fn upgrade(&self) -> Option<ImageData> {
        match self {
            Self::Stored(image) => Some(image.clone()),
            Self::Held(held) => held.upgrade(),
        }
    }
}

/// A decoded image that does not keep its pixels or tree alive.
enum HeldImage {
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
    },
}
impl HeldImage {
    fn new(image: &ImageData) -> Option<Self> {
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
            }),
            ImageData::None => None,
        }
    }

    fn upgrade(&self) -> Option<ImageData> {
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
            } => Some(ImageData::Svg(crate::node::SvgImageData {
                tree: tree.upgrade()?,
                intrinsic_dimensions: *intrinsic_dimensions,
                origin_clean: *origin_clean,
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
        ImageData::Svg(svg) => crate::image_budget::svg_retained_bytes(&svg.tree),
        ImageData::None => 0,
    }
}

#[cfg(test)]
mod tests;
