//! Resources of SVG `feImage` elements, which Canvas filter snapshots read.
//!
//! Like Blink's `SVGFEImageElement`, an element requests its `href` while it is
//! connected: setting `href`, insertion and removal queue an update. Updates
//! run with `<img>` updates, only for elements connected in an active document
//! (never in template contents or other inert documents). Other URLs then load
//! No CORS through the document's image fetches (host policy and readback
//! provenance) and draw nothing until they arrive, while `data:` URLs decode in
//! the engine: a draw before the queued update decodes them itself, within a
//! per-draw budget, as Chrome shows them on the first synchronous draw. Equal
//! URLs share one decode through the document's image cache, and every decode
//! is charged to the document's decoded image budget. A same-document fragment
//! names an element instead and fetches nothing.
use crate::image_request::{ImageKey, PendingImage};
use crate::net::ResourceHandler;
use crate::node::ImageData;
use crate::util::ImageType;
use crate::{BaseDocument, DocumentMutator, NodeId};
use blitz_traits::net::{CorsSettings, ResourceInitiator};
use markup5ever::{QualName, local_name, ns};
use std::collections::{HashMap, HashSet};

/// Decoded bytes a draw may decode itself, for `data:` images whose queued
/// update has not run yet.
const MAX_INLINE_DECODE_BYTES: u64 = 64 * 1024 * 1024;

/// Each element's request, and the elements whose request must be updated.
#[derive(Default)]
pub(crate) struct FeImages {
    requests: HashMap<NodeId, FeImageRequest>,
    queue: Vec<NodeId>,
    queued: HashSet<NodeId>,
}

impl FeImages {
    pub(crate) fn has_updates(&self) -> bool {
        !self.queue.is_empty()
    }
}

/// An element's current request: the resolved URL it is for and, once loaded,
/// the decoded image. A failed or unsupported image, or one past the decoded
/// image budget, stays `None`.
struct FeImageRequest {
    url: String,
    image: Option<ImageData>,
}

/// What an `href` names.
enum Href {
    Fragment(String),
    Url(url::Url),
}

/// What one draw may still decode itself; see [`MAX_INLINE_DECODE_BYTES`].
pub(crate) struct InlineDecodes(u64);

impl Default for InlineDecodes {
    fn default() -> Self {
        Self(MAX_INLINE_DECODE_BYTES)
    }
}

/// What an `feImage` element's `href` names. As in Blink, only a fetched
/// image can leave a canvas readable: every other target taints.
pub(crate) enum FeImageTarget {
    /// No `href`, or one that does not resolve: nothing is fetched or drawn.
    Unfetched,
    /// An element of the same document, by its decoded ID.
    Fragment(String),
    /// A fetched image, once loaded for the current URL; `None` while loading
    /// or when it failed or could not be decoded.
    Fetched(Option<ImageData>),
}

impl BaseDocument {
    /// What `node`'s `href` names now. Images are read from the element's
    /// request, so an image counts only once it loaded for the current URL.
    pub(crate) fn fe_image_target(
        &self,
        node: NodeId,
        inline: &mut InlineDecodes,
    ) -> FeImageTarget {
        let url = match self.fe_image_href(node) {
            None => return FeImageTarget::Unfetched,
            Some(Href::Fragment(id)) => return FeImageTarget::Fragment(id),
            Some(Href::Url(url)) => url,
        };
        let key = ImageKey::no_cors(url.as_str());
        let image = match self.fe_images.requests.get(&node) {
            Some(request) if request.url == url.as_str() => request.image.clone(),
            _ if url.scheme() == "data" => self
                .image_cache
                .get(&key)
                .or_else(|| self.decode_data_url(&url, Some(inline))),
            _ => None,
        };
        FeImageTarget::Fetched(image)
    }

    /// Queue an update of `node`'s request, once.
    pub(crate) fn queue_fe_image_update(&mut self, node: NodeId) {
        if self.is_fe_image(node) && self.fe_images.queued.insert(node) {
            self.fe_images.queue.push(node);
        }
    }

    /// Run the queued updates. `is_active(document)` says whether a document
    /// has a browsing context: only connected elements in those fetch.
    pub(crate) fn run_fe_image_updates(&mut self, is_active: &impl Fn(NodeId) -> bool) {
        self.fe_images.queued.clear();
        for node in std::mem::take(&mut self.fe_images.queue) {
            let Some(element) = self.get_node(node) else {
                continue;
            };
            if self.is_fe_image(node) {
                let active = element.flags.is_in_document() && is_active(element.owner_document);
                self.update_fe_image(node, active);
            }
        }
    }

    /// Start `node`'s request for its current `href`, replacing any earlier one.
    fn update_fe_image(&mut self, node: NodeId, active: bool) {
        let url = match active.then(|| self.fe_image_href(node)).flatten() {
            Some(Href::Url(url)) => Some(url),
            _ => None,
        };
        // A loaded or pending request for the same URL stands: re-inserting or
        // re-setting `href` finds it as Blink finds its memory cache. A failed
        // one is fetched again.
        let standing = self.fe_images.requests.get(&node).is_some_and(|request| {
            url.as_ref().is_some_and(|url| request.url == url.as_str())
                && (request.image.is_some()
                    || self
                        .pending_images
                        .get(&ImageKey::no_cors(&request.url))
                        .is_some_and(|pending| {
                            pending.waiters.contains(&(node, ImageType::FilterImage))
                        }))
        });
        if standing {
            return;
        }
        self.forget_fe_image(node);
        let Some(url) = url else {
            return;
        };
        let key = ImageKey::no_cors(url.as_str());
        let cached = self.image_cache.get(&key);
        let data = url.scheme() == "data";
        let image = match cached {
            Some(image) => Some(image),
            // The URL is its own body, with nothing forbidding storage.
            None if data => self.decode_data_url(&url, None).inspect(|image| {
                self.image_cache
                    .insert(key.clone(), image, crate::ImageRetention::Store);
            }),
            None => None,
        };
        let loaded = data || image.is_some();
        let request = FeImageRequest {
            url: key.url.clone(),
            image,
        };
        self.fe_images.requests.insert(node, request);
        if !loaded {
            self.start_fe_image_fetch(node, key, url);
        }
    }

    /// Drop `node`'s request and stop waiting for its fetch.
    pub(crate) fn forget_fe_image(&mut self, node: NodeId) {
        let Some(request) = self.fe_images.requests.remove(&node) else {
            return;
        };
        let key = ImageKey::no_cors(request.url);
        let waiter = (node, ImageType::FilterImage);
        let empty = self.pending_images.get_mut(&key).is_some_and(|pending| {
            pending.waiters.remove(&waiter);
            pending.waiters.is_empty()
        });
        if empty {
            self.pending_images.remove(&key);
        }
    }

    /// A fetch for `key` completed for `node`; `None` when it failed. A
    /// request since replaced by another `href` ignores it.
    pub(crate) fn fe_image_loaded(
        &mut self,
        node: NodeId,
        key: &ImageKey,
        image: Option<ImageData>,
    ) {
        if key.mode != CorsSettings::NoCors {
            return;
        }
        let current = self
            .fe_images
            .requests
            .get(&node)
            .is_some_and(|request| request.url == key.url && request.image.is_none());
        let Some(image) = image.filter(|_| current) else {
            return;
        };
        if let Some(request) = self.fe_images.requests.get_mut(&node) {
            request.image = Some(image);
        }
    }

    /// A `data:` image, decoded like a fetched one and always readable. `None`
    /// when the URL or image is invalid, or a raster would pass the decoded
    /// image budget or what an `inline` draw may still decode.
    fn decode_data_url(
        &self,
        url: &url::Url,
        inline: Option<&mut InlineDecodes>,
    ) -> Option<ImageData> {
        let data = data_url::DataUrl::process(url.as_str()).ok()?;
        let (body, _) = data.decode_to_vec().ok()?;
        if let (Some(inline), Some(bytes)) =
            (inline, crate::image_decode::bounded_rgba_bytes(&body))
        {
            inline.0 = inline.0.checked_sub(bytes)?;
        }
        let body = blitz_traits::net::Bytes::from(body);
        crate::image_decode::decode_image(&body, true, &self.svg_fonts, &self.image_budget).ok()
    }

    fn start_fe_image_fetch(&mut self, node: NodeId, key: ImageKey, url: url::Url) {
        let waiter = (node, ImageType::FilterImage);
        if let Some(pending) = self.pending_images.get_mut(&key) {
            pending.waiters.insert(waiter);
            return;
        }
        let handler = ResourceHandler::new(
            self.tx.clone(),
            self.id(),
            None,
            self.shell_provider.clone(),
            self.image_handler(ImageType::FilterImage, &key.url),
        );
        let request_id = handler.request_id();
        // Resource Timing reports `feImage` fetches as `other`, as Chrome does.
        let request = self
            .build_request(url)
            .image(CorsSettings::NoCors)
            .initiator(ResourceInitiator::Other);
        self.pending_images.insert(
            key,
            PendingImage::new(request_id, node, ImageType::FilterImage),
        );
        self.net_provider
            .fetch(self.id(), request, Box::new(handler));
    }

    fn is_fe_image(&self, node: NodeId) -> bool {
        self.nodes[node].element_data().is_some_and(|element| {
            element.name.ns == ns!(svg) && element.name.local.as_ref() == "feImage"
        })
    }

    /// What the `href` (preferred by SVG 2 over `xlink:href`) names. A bare
    /// fragment names an element of the document whatever its base URL, as in
    /// Blink's `SVGURIReference::IsExternalURIReference`.
    fn fe_image_href(&self, node: NodeId) -> Option<Href> {
        let element = self.nodes[node].element_data()?;
        let href = element.attr(local_name!("href")).or_else(|| {
            element
                .attrs()
                .iter()
                .find(|attr| is_xlink_href(&attr.name))
                .map(|attr| attr.value.as_str_lossy())
        })?;
        if href.is_empty() {
            return None;
        }
        if let Some(fragment) = href.strip_prefix('#') {
            return Some(Href::Fragment(decode_fragment(fragment)));
        }
        let url = self.resolve_url(node, href)?;
        Some(match self.same_document_fragment(node, &url) {
            Some(id) => Href::Fragment(id),
            None => Href::Url(url),
        })
    }

    /// The decoded fragment when `url` names a fragment of `node`'s own document.
    fn same_document_fragment(&self, node: NodeId, url: &url::Url) -> Option<String> {
        let document = self.document_url(node);
        if url[..url::Position::AfterQuery] != document[..url::Position::AfterQuery] {
            return None;
        }
        url.fragment().map(decode_fragment)
    }
}

fn decode_fragment(fragment: &str) -> String {
    percent_encoding::percent_decode_str(fragment)
        .decode_utf8_lossy()
        .into_owned()
}

fn is_xlink_href(name: &QualName) -> bool {
    name.ns == ns!(xlink) && name.local == local_name!("href")
}

impl DocumentMutator<'_> {
    /// Setting an `feImage`'s `href` or `xlink:href` queues an update.
    pub(crate) fn fe_image_source_changed(&mut self, node: NodeId, name: &QualName) {
        let href = (name.ns == ns!() && name.local == local_name!("href")) || is_xlink_href(name);
        if href {
            self.doc.queue_fe_image_update(node);
        }
    }

    /// An `feImage` that gains a node document (created by the parser, cloned
    /// or adopted) queues an update.
    pub(crate) fn fe_image_inserted(&mut self, node: NodeId) {
        self.doc.queue_fe_image_update(node);
    }
}

#[cfg(test)]
mod tests;
