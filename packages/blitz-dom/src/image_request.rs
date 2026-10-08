//! Image fetch identity. A pending fetch owns indexed waiters and a unique
//! completion identity; decoded images are shared per URL and CORS setting.
use crate::node::{ImageData, SpecialElementData};
use crate::{BaseDocument, DocumentMutator, NodeId, util::ImageType};
use blitz_traits::net::CorsSettings;
use markup5ever::{QualName, local_name, ns};
use std::collections::HashSet;

/// The key of the [list of available images]: the same URL fetched with
/// another CORS setting is a different image with different authority.
///
/// [list of available images]: https://html.spec.whatwg.org/multipage/images.html#list-of-available-images
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ImageKey {
    pub url: String,
    pub mode: CorsSettings,
}
impl ImageKey {
    pub fn new(url: impl Into<String>, mode: CorsSettings) -> Self {
        Self {
            url: url.into(),
            mode,
        }
    }

    /// CSS images and image buttons have no CORS setting and load No CORS.
    pub fn no_cors(url: impl Into<String>) -> Self {
        Self::new(url, CorsSettings::NoCors)
    }
}

/// Decoded images kept for reuse. A page can reference unboundedly many
/// URLs, each under three CORS settings; past these limits images are not
/// cached, and later elements fetch them again.
const MAX_CACHED_IMAGES: usize = 512;
const MAX_CACHED_IMAGE_BYTES: usize = 64 * 1024 * 1024;

pub(crate) struct PendingImage {
    pub request_id: usize,
    pub waiters: HashSet<(NodeId, ImageType)>,
}
impl PendingImage {
    pub fn new(request_id: usize, node: NodeId, kind: ImageType) -> Self {
        Self {
            request_id,
            waiters: HashSet::from([(node, kind)]),
        }
    }
}
impl BaseDocument {
    /// The image an HTML `<img>` or image button currently asks for, if any,
    /// with its resolved URL.
    pub(crate) fn image_request(&self, node: NodeId) -> Option<(ImageKey, url::Url)> {
        let element = self.get_node(node)?.element_data()?;
        let image_input = element.is_image_input();
        if !is_html_img(element) && !image_input {
            return None;
        }
        let raw = element.attr(local_name!("src")).filter(|s| !s.is_empty())?;
        let url = self.resolve_url(node, raw)?;
        let key = if image_input {
            ImageKey::no_cors(url.as_str())
        } else {
            ImageKey::new(
                url.as_str(),
                CorsSettings::from_attribute(element.attr(local_name!("crossorigin"))),
            )
        };
        Some((key, url))
    }

    pub(crate) fn cache_image(&mut self, key: ImageKey, image: &ImageData) {
        let cached: usize = self.image_cache.values().map(decoded_bytes).sum();
        if self.image_cache.len() >= MAX_CACHED_IMAGES
            || cached.saturating_add(decoded_bytes(image)) > MAX_CACHED_IMAGE_BYTES
        {
            return;
        }
        self.image_cache.insert(key, image.clone());
    }

    pub(crate) fn remove_image_waiter(&mut self, node: NodeId, key: &ImageKey) {
        if self.current_image_requests.get(&node) == Some(key) {
            self.current_image_requests.remove(&node);
        }
        let empty = self.pending_images.get_mut(key).is_some_and(|request| {
            request.waiters.remove(&(node, ImageType::Image));
            request.waiters.is_empty()
        });
        if empty {
            self.pending_images.remove(key);
        }
    }

    pub(crate) fn forget_image_input(&mut self, id: NodeId) {
        self.failed_image_inputs.remove(&id);
        let source = self
            .get_node(id)
            .and_then(|node| node.element_data())
            .and_then(|e| e.form_state.image_input_source.clone());
        if let Some(source) = source {
            self.remove_image_waiter(id, &ImageKey::no_cors(source));
        }
    }

    /// Complete the pending fetch that `request_id` started for `url`, the
    /// URL it requested. Completions carry only that URL, so the request id
    /// selects which CORS setting's entry it started.
    pub(crate) fn take_image_waiters(
        &mut self,
        url: &str,
        request_id: usize,
    ) -> Option<(ImageKey, HashSet<(NodeId, ImageType)>)> {
        let key = CorsSettings::ALL
            .into_iter()
            .map(|mode| ImageKey::new(url, mode))
            .find(|key| {
                self.pending_images
                    .get(key)
                    .is_some_and(|pending| pending.request_id == request_id)
            })?;
        let waiters = self.pending_images.remove(&key)?.waiters;
        Some((key, waiters))
    }

    /// A failed fetch, including a CORS failure, leaves its current `<img>`
    /// waiters [broken] rather than showing an image fetched for another
    /// request. Returns the waiters it took.
    ///
    /// [broken]: https://html.spec.whatwg.org/multipage/images.html#img-error
    pub(crate) fn fail_image_request(
        &mut self,
        url: &str,
        request_id: usize,
    ) -> HashSet<(NodeId, ImageType)> {
        let Some((key, waiters)) = self.take_image_waiters(url, request_id) else {
            return HashSet::new();
        };
        for &(id, kind) in &waiters {
            if matches!(kind, ImageType::Image) && self.take_current_request(id, &key) {
                self.break_image(id);
            }
        }
        waiters
    }

    /// Drop an image element's decoded image, including an image button's
    /// retained one.
    pub(crate) fn break_image(&mut self, id: NodeId) {
        let Some(node) = self.nodes.get_mut(id) else {
            return;
        };
        let Some(element) = node.element_data_mut() else {
            return;
        };
        if element.is_image_input() {
            element.form_state.image_input_image = None;
        } else if !matches!(element.special_data, SpecialElementData::Image(_)) {
            return;
        }
        element.special_data = SpecialElementData::None;
        node.clear_layout_cache();
        node.insert_damage(crate::layout::damage::ALL_DAMAGE);
    }

    /// Whether a completed fetch still answers the node's current request,
    /// which then ends: `src` or `crossorigin` may have started another
    /// while it was in flight.
    pub(crate) fn take_current_request(&mut self, node: NodeId, key: &ImageKey) -> bool {
        if self.current_image_requests.get(&node) != Some(key) {
            return false;
        }
        self.current_image_requests.remove(&node);
        true
    }
}

impl DocumentMutator<'_> {
    /// Any change to an `<img>`'s `src` supersedes its request in flight,
    /// including a removal or a change while disconnected, which start no
    /// new request. A connected `<img>` then loads its new source.
    pub(crate) fn img_source_changed(&mut self, node: NodeId, name: &QualName) {
        if name.ns != ns!() || name.local != local_name!("src") {
            return;
        }
        if self
            .doc
            .get_node(node)
            .and_then(|n| n.element_data())
            .is_some_and(is_html_img)
        {
            self.doc.current_image_requests.remove(&node);
        }
    }

    /// The CORS setting of a connected `<img>` before `name` changes, when
    /// `name` is its `crossorigin` attribute.
    pub(crate) fn img_cors_setting(&self, node: NodeId, name: &QualName) -> Option<CorsSettings> {
        if name.ns != ns!() || name.local != local_name!("crossorigin") {
            return None;
        }
        let node = self.doc.get_node(node)?;
        let element = node.element_data().filter(|e| is_html_img(e))?;
        if !node.flags.is_in_document() {
            return None;
        }
        Some(CorsSettings::from_attribute(
            element.attr(local_name!("crossorigin")),
        ))
    }

    /// A changed CORS setting is a [relevant mutation]: the image reloads
    /// under its new request identity.
    ///
    /// [relevant mutation]: https://html.spec.whatwg.org/multipage/images.html#relevant-mutations
    pub(crate) fn img_cors_setting_changed(&mut self, node: NodeId, before: Option<CorsSettings>) {
        let Some(before) = before else {
            return;
        };
        let after = self
            .doc
            .get_node(node)
            .and_then(|n| n.element_data())
            .map(|e| CorsSettings::from_attribute(e.attr(local_name!("crossorigin"))));
        if after.is_some_and(|after| after != before) {
            self.load_image(node);
        }
    }
}

fn decoded_bytes(image: &ImageData) -> usize {
    match image {
        ImageData::Raster(raster) => raster.data.data().len(),
        // Parsed SVG trees are bounded by the SVG parser and count toward
        // the entry limit only.
        #[cfg(feature = "svg")]
        ImageData::Svg(_) => 0,
        ImageData::None => 0,
    }
}

fn is_html_img(element: &crate::ElementData) -> bool {
    element.name.ns == ns!(html) && element.name.local == local_name!("img")
}

#[cfg(test)]
mod tests;
