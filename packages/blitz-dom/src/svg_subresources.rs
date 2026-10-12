//! The images an SVG image loads itself, found while it is parsed.
//!
//! In Chrome an SVG image is a document of its own. Each `<image>` element
//! with an `href` loads, even a hidden one or one whose URL an SVG image may
//! not load (only `data:` URLs load), and the document's load event waits for
//! them: a fetched SVG image, such as an `<img>`, completes a task after it
//! arrives, and its `data:` images are then in the memory cache. A `data:`
//! image that no earlier load left there only appears in the document after a
//! task, so an `feImage` that draws a `data:` SVG image at once draws it
//! without those images. `feImage` elements inside an SVG image neither wait
//! nor delay it.
//!
//! usvg resolves the images it renders without saying which element named
//! them or by what URL, so an image is identified by a digest of its body: an
//! image the resolver is asked for is an `<image>` image when its body is the
//! `data:` `href` of an `<image>` element, at any nesting level that loads.
//! Chrome keys its cache by URL; two URLs encoding the same body count as one
//! image here.
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, PoisonError};
use usvg::roxmltree;

/// Distinct `<image>` bodies remembered per SVG image, across nesting levels.
/// Past this, further `<image>` elements are not recognized: their images
/// appear at once.
const MAX_IMAGE_HREFS: usize = 1024;

use crate::svg_expansion::{SVG_NS, XLINK_NS};

/// Identifies an image by its body (see the [module documentation](self)).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct BodyDigest(u64);

impl BodyDigest {
    pub(crate) fn of(body: &[u8]) -> Self {
        // SipHash with fixed keys: stable within the process, and a collision
        // only changes when a page's own image appears.
        let mut hasher = std::hash::DefaultHasher::new();
        body.hash(&mut hasher);
        Self(hasher.finish())
    }

    /// The digest of a `data:` URL's body; `None` for other URLs.
    pub(crate) fn of_data_url(url: &str) -> Option<Self> {
        let data = data_url::DataUrl::process(url).ok()?;
        let (body, _) = data.decode_to_vec().ok()?;
        Some(Self::of(&body))
    }
}

/// What an SVG image loads itself.
#[derive(Debug, Clone, Default)]
pub(crate) struct SvgSubresources {
    /// The document has an `<image>` element with an `href`, so its load
    /// event waits for a task.
    pub(crate) delays_load: bool,
    /// `<image>` bodies were left out, not yet loaded (see [`NestedImages`]).
    pub(crate) omits_images: bool,
    /// The `data:` bodies its `<image>` elements load, shown or not, sorted
    /// and distinct.
    bodies: Arc<[BodyDigest]>,
}

impl SvgSubresources {
    pub(crate) fn bodies(&self) -> impl Iterator<Item = BodyDigest> + '_ {
        self.bodies.iter().copied()
    }

    /// What keeping the bodies costs, charged with the SVG document.
    pub(crate) fn retained_bytes(&self) -> usize {
        std::mem::size_of_val(&*self.bodies)
    }
}

/// Which of an SVG image's `<image>` bodies to show.
#[derive(Clone)]
pub(crate) enum NestedImages {
    All,
    /// Only these, which the document has loaded.
    Loaded(Arc<HashSet<BodyDigest>>),
}

/// Collects, while one SVG image parses, the bodies its `<image>` elements
/// name and whether any was left out. Shared with the resolver, which usvg
/// requires to be `Send + Sync`.
#[derive(Clone)]
pub(crate) struct SubresourceRecorder {
    shown: NestedImages,
    recorded: Arc<Mutex<Recorded>>,
}

#[derive(Default)]
struct Recorded {
    delays_load: bool,
    omits_images: bool,
    image_hrefs: HashSet<BodyDigest>,
}

impl SubresourceRecorder {
    pub(crate) fn new(shown: NestedImages) -> Self {
        Self {
            shown,
            recorded: Default::default(),
        }
    }

    /// Whether the resolver may accept an image with this body: not when it
    /// is an `<image>` body that is not to be shown, which is then noted as
    /// omitted. Nested documents are scanned before they are parsed, so their
    /// elements are known here.
    pub(crate) fn admits(&self, body: BodyDigest) -> bool {
        let NestedImages::Loaded(loaded) = &self.shown else {
            return true;
        };
        let mut recorded = self.lock();
        let omitted = !loaded.contains(&body) && recorded.image_hrefs.contains(&body);
        recorded.omits_images |= omitted;
        !omitted
    }

    /// Note the `<image>` elements of the SVG image's own document.
    pub(crate) fn scan_document(&self, doc: &roxmltree::Document) {
        let mut recorded = self.lock();
        recorded.delays_load = doc
            .descendants()
            .filter_map(image_href)
            .any(|href| !href.is_empty());
        scan(doc, &mut recorded);
    }

    /// Note the `<image>` elements of a nested document, parsed apart from
    /// usvg, which keeps no element names.
    pub(crate) fn scan_nested(&self, doc: &roxmltree::Document) {
        scan(doc, &mut self.lock());
    }

    /// What the SVG image loads.
    pub(crate) fn finish(&self) -> SvgSubresources {
        let recorded = self.lock();
        let mut bodies: Vec<_> = recorded.image_hrefs.iter().copied().collect();
        bodies.sort_unstable();
        SvgSubresources {
            delays_load: recorded.delays_load,
            omits_images: recorded.omits_images,
            bodies: bodies.into(),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Recorded> {
        // Only plain data is behind the lock; a panic leaves it usable.
        self.recorded.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Note the `data:` bodies the `<image>` elements of `doc` name.
fn scan(doc: &roxmltree::Document, recorded: &mut Recorded) {
    for href in doc.descendants().filter_map(image_href) {
        if recorded.image_hrefs.len() >= MAX_IMAGE_HREFS {
            return;
        }
        if let Some(body) = BodyDigest::of_data_url(href) {
            recorded.image_hrefs.insert(body);
        }
    }
}

/// The `href` of an SVG `<image>` element; SVG 2 prefers it over
/// `xlink:href`, as usvg does.
fn image_href<'a>(node: roxmltree::Node<'a, 'a>) -> Option<&'a str> {
    let name = node.tag_name();
    if !node.is_element() || name.namespace() != Some(SVG_NS) || name.name() != "image" {
        return None;
    }
    node.attribute("href")
        .or_else(|| node.attribute((XLINK_NS, "href")))
}

#[cfg(test)]
mod tests;
