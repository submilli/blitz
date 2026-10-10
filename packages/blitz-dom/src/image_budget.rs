//! The decoded image memory that documents sharing one memory retain, across
//! `<img>` elements, CSS image layers, `feImage` requests and the image cache.
//!
//! A decode reserves its decoded size first, read from the image header for
//! rasters, and is refused when retained images and decodes in progress would
//! pass the budget's limit: the image is then broken, as one that failed to
//! decode. An image is charged until its last holder drops it. The ledger
//! keeps weak references, so a decode shared by many holders counts once and
//! nothing has to report a release.
//!
//! Chrome has no such limit: it discards decoded images under memory pressure
//! and decodes them again when they are painted. A guest cannot do that
//! without keeping every encoded body, so past the budget images break instead.
use crate::node::RasterImageData;
use linebender_resource_handle::WeakBlob;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// Decoded bytes all documents sharing a budget may retain. Each image costs
/// up to twice its charge at its peak: a decode's own working memory (at most
/// 128 MiB besides the RGBA image it reserves, see `image_decode::decode_bounded`), or
/// the premultiplied copy a CPU renderer makes of every raster a frame paints.
/// Images therefore stay within 256 MiB of a 512 MiB guest.
pub const MAX_DECODED_IMAGE_BYTES: usize = 128 * 1024 * 1024;

/// The least an image is charged: its allocations and ledger entry, so many
/// tiny images still count.
pub(crate) const MIN_CHARGE: usize = 64;

/// Entries tracked before the first prune of dropped images; later prunes run
/// when the entries have doubled since the last, so pruning is amortized.
const MIN_PRUNE_ENTRIES: usize = 64;

/// A budget of decoded image bytes. Clones share it: give every document of
/// one guest the same budget (see [`DocumentConfig`](crate::DocumentConfig)),
/// since they share its memory.
#[derive(Clone)]
pub struct DecodedImageBudget(Arc<Mutex<Ledger>>);

impl Default for DecodedImageBudget {
    fn default() -> Self {
        Self::with_limit(MAX_DECODED_IMAGE_BYTES)
    }
}

impl std::fmt::Debug for DecodedImageBudget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ledger = self.ledger();
        f.debug_struct("DecodedImageBudget")
            .field("limit", &ledger.limit)
            .field("charged", &ledger.charged)
            .field("pressure", &ledger.pressure)
            .finish_non_exhaustive()
    }
}

impl DecodedImageBudget {
    /// A budget of `limit` decoded bytes.
    pub fn with_limit(limit: usize) -> Self {
        Self(Arc::new(Mutex::new(Ledger {
            limit,
            charged: 0,
            reserved: 0,
            entries: Vec::new(),
            pruned_len: 0,
            pressure: 0,
        })))
    }

    /// The bytes this budget admits.
    pub fn limit(&self) -> usize {
        self.ledger().limit
    }

    /// Bytes ever asked of this budget, admitted or refused. It only grows,
    /// so an embedder that can release images (a garbage collector dropping
    /// detached elements) can pace its collections by it.
    pub fn pressure(&self) -> u64 {
        self.ledger().pressure
    }

    /// Bytes charged to images still held, after dropping released ones.
    pub fn retained_bytes(&self) -> usize {
        let mut ledger = self.ledger();
        ledger.prune();
        ledger.charged
    }

    /// Reserve `bytes` for a decode in progress; `None` past the limit.
    pub(crate) fn reserve(&self, bytes: usize) -> Option<Reservation> {
        let bytes = bytes.max(MIN_CHARGE);
        let mut ledger = self.ledger();
        ledger.pressure = ledger.pressure.saturating_add(bytes as u64);
        if !ledger.fits(bytes) {
            ledger.prune();
            if !ledger.fits(bytes) {
                return None;
            }
        }
        ledger.reserved += bytes;
        Some(Reservation {
            budget: self.clone(),
            bytes,
        })
    }

    /// Charge a parsed SVG document, whose cost is known only once it
    /// exists. `false` when it does not fit, and it must then be dropped.
    #[cfg(feature = "svg")]
    pub(crate) fn admit_svg(&self, svg: &crate::node::SvgImageData) -> bool {
        let Some(reservation) = self.reserve(svg_retained_bytes(&svg.tree)) else {
            return false;
        };
        reservation.charge(WeakImage::Svg(Arc::downgrade(&svg.tree)));
        true
    }

    fn ledger(&self) -> MutexGuard<'_, Ledger> {
        // Nothing panics while holding the lock, and the ledger's counts are
        // valid between statements, so a poisoned lock is still usable.
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Bytes reserved for one decode, released unless [`Self::track_raster`]
/// charges them to the decoded image.
pub(crate) struct Reservation {
    budget: DecodedImageBudget,
    bytes: usize,
}

impl Reservation {
    /// Charge the reserved bytes to `raster` until its last holder drops it.
    pub(crate) fn track_raster(self, raster: &RasterImageData) {
        self.charge(WeakImage::Raster(raster.data.downgrade()));
    }

    fn charge(mut self, image: WeakImage) {
        let bytes = std::mem::take(&mut self.bytes);
        let mut ledger = self.budget.ledger();
        ledger.reserved -= bytes;
        ledger.charged += bytes;
        ledger.entries.push(Tracked { image, bytes });
        if ledger.entries.len() >= MIN_PRUNE_ENTRIES.max(ledger.pruned_len * 2) {
            ledger.prune();
        }
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        if self.bytes > 0 {
            self.budget.ledger().reserved -= self.bytes;
        }
    }
}

struct Ledger {
    limit: usize,
    /// Bytes of tracked images, including dropped ones until the next prune.
    charged: usize,
    /// Bytes reserved by decodes in progress.
    reserved: usize,
    entries: Vec<Tracked>,
    /// Entries left by the last prune.
    pruned_len: usize,
    /// See [`DecodedImageBudget::pressure`].
    pressure: u64,
}

impl Ledger {
    fn fits(&self, bytes: usize) -> bool {
        self.charged
            .saturating_add(self.reserved)
            .saturating_add(bytes)
            <= self.limit
    }

    /// Forget the images whose holders have all dropped them.
    fn prune(&mut self) {
        let mut charged = 0usize;
        self.entries.retain(|entry| {
            let live = entry.image.is_live();
            if live {
                charged += entry.bytes;
            }
            live
        });
        self.charged = charged;
        self.pruned_len = self.entries.len();
    }
}

struct Tracked {
    image: WeakImage,
    bytes: usize,
}

/// A decoded image that does not keep its pixels or tree alive.
enum WeakImage {
    Raster(WeakBlob<u8>),
    #[cfg(feature = "svg")]
    Svg(std::sync::Weak<usvg::Tree>),
}

impl WeakImage {
    fn is_live(&self) -> bool {
        match self {
            Self::Raster(data) => data.upgrade().is_some(),
            #[cfg(feature = "svg")]
            Self::Svg(tree) => tree.strong_count() > 0,
        }
    }
}

/// What retaining a parsed SVG document costs, estimated from its nodes
/// (including clip paths, masks, patterns, filters and gradient stops), path
/// points, flattened text and nested documents. A nested raster stays encoded
/// in the tree, but the renderer decodes it and copies the decode on every
/// paint, so it costs twice its decoded size.
#[cfg(feature = "svg")]
pub(crate) fn svg_retained_bytes(tree: &usvg::Tree) -> usize {
    const BYTES_PER_NODE: usize = 1024;
    const BYTES_PER_POINT: usize = 16;
    const BYTES_PER_STOP: usize = 64;
    let mut bytes = 0usize;
    let mut add = |more: usize| bytes = bytes.saturating_add(more);
    let mut trees = vec![tree];
    let mut groups: Vec<&usvg::Group> = Vec::new();
    while let Some(tree) = trees.pop() {
        add(BYTES_PER_NODE);
        groups.push(tree.root());
        groups.extend(tree.clip_paths().iter().map(|clip| clip.root()));
        groups.extend(tree.masks().iter().map(|mask| mask.root()));
        groups.extend(tree.patterns().iter().map(|pattern| pattern.root()));
        for primitive in tree.filters().iter().flat_map(|filter| filter.primitives()) {
            add(BYTES_PER_NODE);
            if let usvg::filter::Kind::Image(image) = primitive.kind() {
                groups.push(image.root());
            }
        }
        let linear = tree.linear_gradients().iter().map(|g| g.stops().len());
        let radial = tree.radial_gradients().iter().map(|g| g.stops().len());
        for stops in linear.chain(radial) {
            add(BYTES_PER_NODE.saturating_add(stops.saturating_mul(BYTES_PER_STOP)));
        }
        while let Some(group) = groups.pop() {
            for node in group.children() {
                add(BYTES_PER_NODE);
                match node {
                    usvg::Node::Group(child) => groups.push(child),
                    usvg::Node::Path(path) => {
                        add(path.data().points().len().saturating_mul(BYTES_PER_POINT));
                    }
                    usvg::Node::Text(text) => groups.push(text.flattened()),
                    usvg::Node::Image(image) => match image.kind() {
                        usvg::ImageKind::SVG(nested) => trees.push(nested),
                        usvg::ImageKind::JPEG(data)
                        | usvg::ImageKind::PNG(data)
                        | usvg::ImageKind::GIF(data)
                        | usvg::ImageKind::WEBP(data) => add(nested_raster_bytes(data)),
                    },
                }
            }
        }
    }
    bytes
}

/// A nested raster's decode and its renderer copy; its encoded size when its
/// header is unreadable, as the renderer then fails to decode it.
#[cfg(feature = "svg")]
fn nested_raster_bytes(encoded: &[u8]) -> usize {
    crate::image_decode::bounded_rgba_bytes(encoded)
        .and_then(|rgba| usize::try_from(rgba).ok())
        .map_or(encoded.len(), |rgba| rgba.saturating_mul(2))
}

#[cfg(test)]
mod tests;
