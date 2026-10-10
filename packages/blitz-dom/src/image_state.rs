//! The [image request] state of HTML `<img>` elements: what `complete`,
//! the natural dimensions and `currentSrc` report, HTML's [update the image
//! data] on [relevant mutations], and the `load`/`error` events an embedder
//! dispatches.
//!
//! A relevant mutation only queues an update. The embedder runs queued
//! updates in two stages, the way Chrome splits the algorithm: right after
//! the mutation, updates that need no fetch (no or empty `src`, or an image
//! already in the [list of available images]) complete; the rest wait for a
//! stable state, so several mutations in one task start one request. A
//! document without an embedder runs every update when its mutator flushes.
//!
//! Each update that replaces the current request advances the element's
//! generation. Events carry the generation they were queued in; an event
//! whose element has moved on is stale and must not be dispatched (Chrome
//! cancels it).
//!
//! [image request]: https://html.spec.whatwg.org/multipage/images.html#image-request
//! [update the image data]: https://html.spec.whatwg.org/multipage/images.html#update-the-image-data
//! [relevant mutations]: https://html.spec.whatwg.org/multipage/images.html#relevant-mutations
//! [list of available images]: https://html.spec.whatwg.org/multipage/images.html#list-of-available-images
use std::collections::{HashMap, HashSet, VecDeque};

use markup5ever::{QualName, local_name, ns};

use crate::document::DocumentEvent;
use crate::image_request::ImageKey;
use crate::node::{ImageData, SpecialElementData};
use crate::{BaseDocument, DocumentMutator, ElementData, NodeId};

/// Events waiting for the embedder. Each relevant mutation can queue one,
/// so a page could queue without end between drains; past this limit new
/// events are dropped.
const MAX_QUEUED_EVENTS: usize = 4096;

/// The state of an `<img>`'s current request. "Partially available" is
/// never observable: images decode whole.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ImageRequestState {
    /// No image yet: no source, or a fetch in flight.
    #[default]
    Unavailable,
    CompletelyAvailable,
    /// The fetch, the decode or the URL failed. Canvas refuses the image.
    Broken,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageEventKind {
    Load,
    Error,
}

/// A `load` or `error` event to fire at `node`, an element task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageEvent {
    pub node: NodeId,
    pub kind: ImageEventKind,
    /// The element's generation when the event was queued. The embedder
    /// drops the event if [`ImageStatus::generation`] differs at dispatch.
    pub generation: u64,
}

/// Which queued updates [`BaseDocument::run_image_updates`] completes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageUpdateStage {
    /// Right after a mutation: only updates that need no fetch, and only
    /// those queued since the last immediate run (the others needed one).
    Immediate,
    /// At a stable state (a microtask): all of them.
    StableState,
}

/// What script can observe of an `<img>`, except its URL
/// ([`BaseDocument::image_current_url`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageStatus {
    pub state: ImageRequestState,
    /// `src` is present and not empty.
    pub has_source: bool,
    /// An update is queued: a relevant mutation has not been processed.
    pub update_pending: bool,
    /// Advances whenever an update replaces the current request.
    pub generation: u64,
    /// Advances with every relevant mutation. `decode()` fails when it
    /// changes.
    pub revision: u64,
    /// Width and height of the current image, zero without one.
    pub natural_size: (u32, u32),
}

impl ImageStatus {
    /// HTML's [`complete`], as Chrome reports it: images that never load
    /// (no source, or a document without a browsing context) are complete.
    ///
    /// [`complete`]: https://html.spec.whatwg.org/multipage/embedded-content.html#dom-img-complete
    pub fn complete(&self, document_active: bool) -> bool {
        !document_active
            || !self.has_source
            || (!self.update_pending && self.state != ImageRequestState::Unavailable)
    }
}

/// One `<img>`'s request state.
#[derive(Debug, Default)]
pub(crate) struct ImageElementState {
    state: ImageRequestState,
    current_url: String,
    update_pending: bool,
    /// In `ImageElements::fresh`.
    fresh: bool,
    /// In `ImageElements::deferred`.
    deferred: bool,
    generation: u64,
    revision: u64,
}

/// The document's `<img>` request states, queued updates and events.
#[derive(Default)]
pub(crate) struct ImageElements {
    states: HashMap<NodeId, ImageElementState>,
    /// Elements mutated since the last immediate run, in mutation order.
    /// The immediate stage visits only these, so a mutation never revisits
    /// every deferred source.
    fresh: Vec<NodeId>,
    /// Elements whose update needs a fetch, for the stable state. An entry
    /// is stale once a later immediate run completed its update.
    deferred: Vec<NodeId>,
    events: VecDeque<ImageEvent>,
}

impl ImageElements {
    /// Elements the engine still owes work: queued updates and events.
    pub(crate) fn referenced_nodes(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.fresh
            .iter()
            .chain(&self.deferred)
            .copied()
            .chain(self.events.iter().map(|event| event.node))
    }
}

/// What a queued update did.
enum Outcome {
    Done,
    /// It started a fetch, which a provider may already have answered.
    Fetched(usize),
    /// It needs a fetch, which waits for the stable state.
    Deferred,
}

impl BaseDocument {
    /// The request state of the HTML `<img>` `node`, or `None` for any
    /// other node.
    pub fn image_status(&self, node: NodeId) -> Option<ImageStatus> {
        let element = self.get_node(node)?.element_data()?;
        if !is_html_img(element) {
            return None;
        }
        let natural_size = match &element.special_data {
            SpecialElementData::Image(image) => natural_size(image),
            _ => (0, 0),
        };
        let has_source = element
            .attr(local_name!("src"))
            .is_some_and(|src| !src.is_empty());
        let state = self.images.states.get(&node);
        Some(ImageStatus {
            state: state.map_or_else(Default::default, |s| s.state),
            has_source,
            update_pending: state.is_some_and(|s| s.update_pending),
            generation: state.map_or(0, |s| s.generation),
            revision: state.map_or(0, |s| s.revision),
            natural_size,
        })
    }

    /// The current request's URL (`currentSrc`). Empty while a fetch is in
    /// flight, as in Chrome.
    pub fn image_current_url(&self, node: NodeId) -> &str {
        self.images
            .states
            .get(&node)
            .map_or("", |state| &state.current_url)
    }

    /// Whether [`Self::run_image_updates`] has work at `stage`.
    pub fn has_image_updates(&self, stage: ImageUpdateStage) -> bool {
        #[cfg(feature = "svg")]
        if self.fe_images.has_updates() {
            return true;
        }
        match stage {
            ImageUpdateStage::Immediate => !self.images.fresh.is_empty(),
            ImageUpdateStage::StableState => {
                !self.images.fresh.is_empty() || !self.images.deferred.is_empty()
            }
        }
    }

    /// Run queued updates (see the [module documentation](self)).
    /// `is_active(document)` says whether a document has a browsing
    /// context: images in other documents (`<template>` contents,
    /// `DOMParser` results) never load and fire nothing.
    ///
    /// A provider may answer a request synchronously (`data:` URLs); those
    /// answers apply before this returns, as HTML decodes them within the
    /// update.
    pub fn run_image_updates(
        &mut self,
        stage: ImageUpdateStage,
        is_active: impl Fn(NodeId) -> bool,
    ) {
        // `feImage` requests wait for no microtask: they run at either stage.
        #[cfg(feature = "svg")]
        self.run_fe_image_updates(&is_active);
        let mut queued = std::mem::take(&mut self.images.fresh);
        if stage == ImageUpdateStage::StableState {
            queued.splice(0..0, std::mem::take(&mut self.images.deferred));
        }
        let mut fetches = HashSet::new();
        for node in queued {
            let Some(state) = self.images.states.get_mut(&node) else {
                continue;
            };
            state.fresh = false;
            if stage == ImageUpdateStage::StableState {
                state.deferred = false;
            }
            if !state.update_pending {
                continue;
            }
            match self.update_image_data(node, stage, &is_active) {
                Outcome::Deferred => {
                    let state = self.images.states.entry(node).or_default();
                    if !state.deferred {
                        state.deferred = true;
                        self.images.deferred.push(node);
                    }
                    continue;
                }
                Outcome::Fetched(request) => {
                    fetches.insert(request);
                }
                Outcome::Done => {}
            }
            if let Some(state) = self.images.states.get_mut(&node) {
                state.update_pending = false;
            }
        }
        self.apply_synchronous_answers(&fetches);
    }

    /// Take the events queued since the last call, oldest first.
    pub fn take_image_events(&mut self) -> Vec<ImageEvent> {
        self.images.events.drain(..).collect()
    }

    /// Whether `<img>` elements of `document` [delay its load event]: an
    /// update is queued or a fetch is in flight.
    ///
    /// [delay its load event]: https://html.spec.whatwg.org/multipage/images.html#delay-the-load-event
    pub fn images_delay_load(&self, document: NodeId) -> bool {
        let in_document = |node: &NodeId| {
            self.get_node(*node)
                .is_some_and(|n| n.owner_document == document)
        };
        let pending = |node: &&NodeId| {
            self.images
                .states
                .get(node)
                .is_some_and(|state| state.update_pending)
        };
        self.images
            .fresh
            .iter()
            .chain(&self.images.deferred)
            .filter(pending)
            .any(in_document)
            || self
                .current_image_requests
                .keys()
                .filter(|node| self.images.states.contains_key(node))
                .any(in_document)
    }

    /// The fetch for `node`'s current request completed.
    pub(crate) fn image_loaded(&mut self, node: NodeId, key: &ImageKey) {
        self.finish_request(node, &key.url, ImageRequestState::CompletelyAvailable);
    }

    /// The fetch for `node`'s current request failed.
    pub(crate) fn image_failed(&mut self, node: NodeId, key: &ImageKey) {
        self.finish_request(node, &key.url, ImageRequestState::Broken);
    }

    /// Release the state of a node being dropped.
    pub(crate) fn forget_image_element(&mut self, node: NodeId) {
        if self.images.states.remove(&node).is_none() {
            return;
        }
        if let Some(key) = self.current_image_requests.get(&node).cloned() {
            self.remove_image_waiter(node, &key);
        }
    }

    fn finish_request(&mut self, node: NodeId, url: &str, state: ImageRequestState) {
        let Some(element) = self.images.states.get_mut(&node) else {
            return;
        };
        element.state = state;
        element.current_url = url.to_owned();
        let kind = match state {
            ImageRequestState::CompletelyAvailable => ImageEventKind::Load,
            _ => ImageEventKind::Error,
        };
        self.queue_image_event(node, kind);
    }

    fn update_image_data(
        &mut self,
        node: NodeId,
        stage: ImageUpdateStage,
        is_active: &impl Fn(NodeId) -> bool,
    ) -> Outcome {
        let Some(element) = self
            .get_node(node)
            .and_then(|n| n.element_data())
            .filter(|e| is_html_img(e))
        else {
            self.images.states.remove(&node);
            return Outcome::Done;
        };
        if !is_active(self.node_document(node)) {
            return Outcome::Done;
        }
        let Some(source) = element.attr(local_name!("src")).map(str::to_owned) else {
            self.replace_current_request(node, ImageRequestState::Unavailable, String::new());
            return Outcome::Done;
        };
        if source.is_empty() {
            // Chrome reports an error but, unlike a failed fetch, does not
            // treat the image as broken for Canvas.
            self.replace_current_request(node, ImageRequestState::Unavailable, String::new());
            self.queue_image_event(node, ImageEventKind::Error);
            return Outcome::Done;
        }
        let request = self.image_request(node);
        if let Some((key, _)) = &request
            && let Some(image) = self.image_cache.get(key).cloned()
        {
            let url = key.url.clone();
            self.replace_current_request(node, ImageRequestState::CompletelyAvailable, url);
            self.set_image_data(node, image);
            self.queue_image_event(node, ImageEventKind::Load);
            return Outcome::Done;
        }
        if stage == ImageUpdateStage::Immediate {
            return Outcome::Deferred;
        }
        let Some((key, url)) = request else {
            self.replace_current_request(node, ImageRequestState::Broken, source);
            self.queue_image_event(node, ImageEventKind::Error);
            return Outcome::Done;
        };
        // HTML step 14: the request in flight is already for this URL.
        if self.current_image_requests.get(&node) == Some(&key) {
            return Outcome::Done;
        }
        self.replace_current_request(node, ImageRequestState::Unavailable, String::new());
        Outcome::Fetched(self.start_image_fetch(node, key, url))
    }

    /// Abort the request in flight and replace the current request, without
    /// an image.
    fn replace_current_request(&mut self, node: NodeId, state: ImageRequestState, url: String) {
        if let Some(key) = self.current_image_requests.get(&node).cloned() {
            self.remove_image_waiter(node, &key);
        }
        self.break_image(node);
        let element = self.images.states.entry(node).or_default();
        element.state = state;
        element.current_url = url;
        element.generation += 1;
    }

    fn set_image_data(&mut self, node: NodeId, image: ImageData) {
        let node = &mut self.nodes[node];
        let element = node
            .element_data_mut()
            .expect("image updates run on elements");
        element.special_data = SpecialElementData::Image(Box::new(image));
        node.clear_layout_cache();
        node.insert_damage(crate::layout::damage::ALL_DAMAGE);
    }

    fn queue_image_event(&mut self, node: NodeId, kind: ImageEventKind) {
        if self.images.events.len() >= MAX_QUEUED_EVENTS {
            return;
        }
        let generation = self.images.states.get(&node).map_or(0, |s| s.generation);
        self.images.events.push_back(ImageEvent {
            node,
            kind,
            generation,
        });
    }

    /// Apply the answers to `requests` that a provider already sent, and
    /// only those: other completions still wait for the embedder.
    ///
    /// Providers answer on the document's thread (the embedder's queue
    /// runs there), so re-sending the others keeps their order.
    fn apply_synchronous_answers(&mut self, requests: &HashSet<usize>) {
        if requests.is_empty() {
            return;
        }
        let Some(rx) = self.rx.take() else {
            return;
        };
        let mut others = Vec::new();
        while let Ok(message) = rx.try_recv() {
            match message {
                DocumentEvent::ResourceLoad(load) if requests.contains(&load.request_id) => {
                    self.load_resource(load);
                }
                other => others.push(other),
            }
        }
        self.rx = Some(rx);
        for message in others {
            let _ = self.tx.send(message);
        }
    }
}

impl DocumentMutator<'_> {
    /// A [relevant mutation] of the HTML `<img>` `node`: queue an update.
    ///
    /// [relevant mutation]: https://html.spec.whatwg.org/multipage/images.html#relevant-mutations
    pub(crate) fn queue_image_update(&mut self, node: NodeId) {
        if !self
            .doc
            .get_node(node)
            .and_then(|n| n.element_data())
            .is_some_and(is_html_img)
        {
            return;
        }
        let images = &mut self.doc.images;
        let state = images.states.entry(node).or_default();
        state.revision += 1;
        state.update_pending = true;
        if !state.fresh {
            state.fresh = true;
            images.fresh.push(node);
        }
    }

    /// Setting `src` (even to its value) is a relevant mutation.
    pub(crate) fn img_source_changed(&mut self, node: NodeId, name: &QualName) {
        if name.ns == ns!() && name.local == local_name!("src") {
            self.queue_image_update(node);
        }
    }

    /// An HTML `<img>`'s `referrerpolicy` before `name` changes, when
    /// `name` is that attribute.
    pub(crate) fn img_referrer_policy(
        &self,
        node: NodeId,
        name: &QualName,
    ) -> Option<&'static str> {
        if name.ns != ns!() || name.local != local_name!("referrerpolicy") {
            return None;
        }
        let element = self
            .doc
            .get_node(node)?
            .element_data()
            .filter(|e| is_html_img(e))?;
        Some(referrer_policy(element))
    }

    /// A changed referrer policy state is a relevant mutation.
    pub(crate) fn img_referrer_policy_changed(
        &mut self,
        node: NodeId,
        before: Option<&'static str>,
    ) {
        let Some(before) = before else {
            return;
        };
        let after = self
            .doc
            .get_node(node)
            .and_then(|n| n.element_data())
            .map(referrer_policy);
        if after.is_some_and(|after| after != before) {
            self.queue_image_update(node);
        }
    }

    /// An `<img>` that gains a node document with a `src` (created by the
    /// parser, cloned, imported or adopted) updates its image data.
    pub(crate) fn queue_image_update_if_sourced(&mut self, node: NodeId) {
        if self
            .doc
            .get_node(node)
            .and_then(|n| n.element_data())
            .is_some_and(|e| e.attr(local_name!("src")).is_some())
        {
            self.queue_image_update(node);
        }
    }

    /// Without an embedder, updates run when the mutator flushes, in the
    /// main document only.
    pub(crate) fn run_image_updates_unembedded(&mut self) {
        if self.doc.embedder_updates_images
            || !self.doc.has_image_updates(ImageUpdateStage::StableState)
        {
            return;
        }
        let root = self.doc.root_node_id;
        self.doc
            .run_image_updates(ImageUpdateStage::StableState, |document| document == root);
    }
}

fn natural_size(image: &ImageData) -> (u32, u32) {
    match image {
        ImageData::Raster(raster) => (raster.width, raster.height),
        #[cfg(feature = "svg")]
        ImageData::Svg(svg) => {
            let size = svg.tree.size();
            (size.width().round() as u32, size.height().round() as u32)
        }
        ImageData::None => (0, 0),
    }
}

/// The [referrer policy attribute]'s state: a keyword, matched ASCII
/// case-insensitively, or the empty string for a missing or invalid value.
///
/// [referrer policy attribute]: https://html.spec.whatwg.org/multipage/urls-and-fetching.html#referrer-policy-attribute
fn referrer_policy(element: &ElementData) -> &'static str {
    const POLICIES: [&str; 8] = [
        "no-referrer",
        "no-referrer-when-downgrade",
        "same-origin",
        "origin",
        "strict-origin",
        "origin-when-cross-origin",
        "strict-origin-when-cross-origin",
        "unsafe-url",
    ];
    element
        .attr(local_name!("referrerpolicy"))
        .and_then(|value| {
            POLICIES
                .into_iter()
                .find(|policy| value.eq_ignore_ascii_case(policy))
        })
        .unwrap_or("")
}

pub(crate) fn is_html_img(element: &ElementData) -> bool {
    element.name.ns == ns!(html) && element.name.local == local_name!("img")
}

#[cfg(test)]
mod tests;
