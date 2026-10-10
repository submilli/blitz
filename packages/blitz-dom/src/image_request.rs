//! Image fetch identity. A pending fetch owns ordered waiters and a unique
//! completion identity; decoded images are shared per URL and CORS setting.
use crate::image_state::is_html_img;
use crate::net::{ImageHandler, ResourceHandler};
use crate::node::SpecialElementData;
use crate::{BaseDocument, DocumentMutator, NodeId, util::ImageType};
use blitz_traits::net::{CorsSettings, ResourceInitiator};
use markup5ever::{QualName, local_name, ns};
use std::collections::HashMap;

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

pub(crate) struct PendingImage {
    pub request_id: usize,
    pub waiters: Waiters,
}
impl PendingImage {
    pub fn new(request_id: usize, node: NodeId, kind: ImageType) -> Self {
        let mut waiters = Waiters::default();
        waiters.insert((node, kind));
        Self {
            request_id,
            waiters,
        }
    }
}

/// The nodes waiting for one fetch. A page can attach many elements to one
/// URL and detach them again, so removal is constant time; completion
/// takes them in the order they started waiting, so their events are
/// deterministic.
#[derive(Default)]
pub(crate) struct Waiters {
    next: u64,
    entries: HashMap<(NodeId, ImageType), u64>,
}
impl Waiters {
    pub fn insert(&mut self, waiter: (NodeId, ImageType)) {
        let next = &mut self.next;
        self.entries.entry(waiter).or_insert_with(|| {
            *next += 1;
            *next
        });
    }

    pub fn contains(&self, waiter: &(NodeId, ImageType)) -> bool {
        self.entries.contains_key(waiter)
    }

    pub fn remove(&mut self, waiter: &(NodeId, ImageType)) {
        self.entries.remove(waiter);
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = &(NodeId, ImageType)> {
        self.entries.keys()
    }

    /// The waiters, oldest first.
    pub fn into_ordered(self) -> Vec<(NodeId, ImageType)> {
        let mut waiters: Vec<_> = self.entries.into_iter().collect();
        waiters.sort_unstable_by_key(|&(_, order)| order);
        waiters.into_iter().map(|(waiter, _)| waiter).collect()
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
    ) -> Option<(ImageKey, Vec<(NodeId, ImageType)>)> {
        let key = CorsSettings::ALL
            .into_iter()
            .map(|mode| ImageKey::new(url, mode))
            .find(|key| {
                self.pending_images
                    .get(key)
                    .is_some_and(|pending| pending.request_id == request_id)
            })?;
        let waiters = self.pending_images.remove(&key)?.waiters.into_ordered();
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
    ) -> Vec<(NodeId, ImageType)> {
        let Some((key, waiters)) = self.take_image_waiters(url, request_id) else {
            return Vec::new();
        };
        for &(id, kind) in &waiters {
            if matches!(kind, ImageType::Image) && self.take_current_request(id, &key) {
                self.break_image(id);
                self.image_failed(id, &key);
            }
            #[cfg(feature = "svg")]
            if matches!(kind, ImageType::FilterImage) {
                self.fe_image_loaded(id, &key, None);
            }
        }
        waiters
    }

    /// Fetch `key` for the `<img>` or image button `node`, joining a fetch
    /// already in flight for the same key. Returns the request's id.
    pub(crate) fn start_image_fetch(
        &mut self,
        node: NodeId,
        key: ImageKey,
        url: url::Url,
    ) -> usize {
        self.current_image_requests.insert(node, key.clone());
        if let Some(pending) = self.pending_images.get_mut(&key) {
            pending.waiters.insert((node, ImageType::Image));
            return pending.request_id;
        }
        let handler = ResourceHandler::new(
            self.tx.clone(),
            self.id(),
            None,
            self.shell_provider.clone(),
            self.image_handler(ImageType::Image, &key.url),
        );
        let request_id = handler.request_id();
        let initiator = if self.nodes[node]
            .element_data()
            .is_some_and(crate::ElementData::is_image_input)
        {
            ResourceInitiator::Input
        } else {
            ResourceInitiator::Img
        };
        let request = self.build_request(url).image(key.mode).initiator(initiator);
        self.pending_images
            .insert(key, PendingImage::new(request_id, node, ImageType::Image));
        self.net_provider
            .fetch(self.id(), request, Box::new(handler));
        request_id
    }

    /// A handler decoding a fetched image for `kind` within the document's
    /// fonts and decoded image budget.
    pub(crate) fn image_handler(&self, kind: ImageType, request_url: &str) -> ImageHandler {
        ImageHandler::new(
            kind,
            request_url,
            self.svg_fonts.clone(),
            self.image_budget.clone(),
        )
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
    /// The CORS setting of an HTML `<img>` before `name` changes, when
    /// `name` is its `crossorigin` attribute.
    pub(crate) fn img_cors_setting(&self, node: NodeId, name: &QualName) -> Option<CorsSettings> {
        if name.ns != ns!() || name.local != local_name!("crossorigin") {
            return None;
        }
        let element = self
            .doc
            .get_node(node)?
            .element_data()
            .filter(|e| is_html_img(e))?;
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
            self.queue_image_update(node);
        }
    }
}

#[cfg(test)]
mod tests;
