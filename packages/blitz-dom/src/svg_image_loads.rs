//! SVG images whose own images load as Chrome's do (see
//! [`crate::svg_subresources`]).
//!
//! The document remembers the `data:` bodies it has loaded, Chrome's memory
//! cache for the only images SVG images can load. Two things wait a task:
//!
//! - A fetched SVG image (for `<img>`, CSS images or `feImage`) whose
//!   document has `<image>` elements completes its fetch a task after it
//!   arrives: the pending fetch keeps the image, new waiters join it, and
//!   `<img>` elements stay incomplete and delay the load event meanwhile.
//! - An `feImage` `data:` SVG image shows at once without the `<image>`
//!   images not loaded yet, and completes with them a task later.
//!
//! The embedder runs these completions as tasks
//! ([`BaseDocument::take_svg_image_loads`],
//! [`BaseDocument::complete_svg_image_load`]); an embedder that sets
//! [`BaseDocument::embedder_updates_images`] must, or those fetches never end.
//! A document whose embedder does not run image updates waits for nothing.
use crate::image_cache::ImageRetention;
use crate::image_request::{ImageKey, PendingImage};
use crate::node::ImageData;
use crate::svg_subresources::{BodyDigest, NestedImages};
use crate::util::ImageType;
use crate::{BaseDocument, NodeId};
use std::collections::{HashSet, VecDeque};
use std::sync::Arc;

/// Loaded bodies remembered. Past this the oldest is forgotten, and an SVG
/// image showing it again waits a task for it.
const MAX_LOADED_BODIES: usize = 4096;

/// The bodies a document has loaded, and the completions waiting for the
/// embedder.
#[derive(Default)]
pub(crate) struct SvgImageLoads {
    /// Shared with the parses that show only loaded images, copied on write.
    loaded: Arc<HashSet<BodyDigest>>,
    /// `loaded` in insertion order, to forget the oldest.
    order: VecDeque<BodyDigest>,
    queued: Vec<SvgImageLoad>,
    /// Identity changes on cancellation; retained tasks prevent address reuse.
    generation: Arc<()>,
}

impl SvgImageLoads {
    fn queue(&mut self, kind: Load) {
        self.queued.push(SvgImageLoad {
            kind,
            generation: Arc::clone(&self.generation),
        });
    }
}

/// What a deferred fetch completes with.
pub(crate) enum Deferred {
    /// The fetched image.
    Image(ImageData, ImageRetention),
    /// The `data:` URL's image, decoded with all its `<image>` images then.
    DataUrl,
}

/// A completion for the embedder to run as a task.
#[derive(Debug, Clone)]
pub struct SvgImageLoad {
    kind: Load,
    generation: Arc<()>,
}

#[derive(Debug, Clone)]
enum Load {
    /// A deferred fetch.
    Fetch { key: ImageKey, request_id: usize },
    /// The `<image>` bodies of an SVG image that was shown at once: Chrome's
    /// load of them, even hidden ones, ends a task later.
    Bodies(Vec<BodyDigest>),
}

impl BaseDocument {
    /// Take the completions queued since the last call, oldest first.
    pub fn take_svg_image_loads(&mut self) -> Vec<SvgImageLoad> {
        std::mem::take(&mut self.svg_image_loads.queued)
    }

    /// Run a completion: a deferred fetch's waiters get the image, as when a
    /// fetch completes (nothing happens when every waiter has since gone), or
    /// an SVG image shown at once has loaded its `<image>` images.
    pub fn complete_svg_image_load(&mut self, load: SvgImageLoad) {
        if !Arc::ptr_eq(&load.generation, &self.svg_image_loads.generation) {
            return;
        }
        match load.kind {
            Load::Fetch { key, request_id } => self.complete_fetch(key, request_id),
            Load::Bodies(bodies) => {
                for body in bodies {
                    self.note_loaded_body(body);
                }
            }
        }
    }

    /// Cancel deferred SVG responses when their embedder tasks are discarded.
    /// This also cancels completions already transferred to the embedder; running
    /// one afterward is harmless. Requests still awaiting a response and the
    /// cache of bodies already loaded are unchanged. No author code is invoked.
    pub fn cancel_svg_image_loads(&mut self) {
        self.svg_image_loads.generation = Arc::new(());
        self.svg_image_loads.queued.clear();
        let canceled: Vec<_> = self
            .pending_images
            .iter()
            .filter(|(_, pending)| pending.deferred.is_some())
            .map(|(key, pending)| (key.url.clone(), pending.request_id))
            .collect();
        for (url, request_id) in canceled {
            self.fail_image_request(&url, request_id);
        }
    }

    fn complete_fetch(&mut self, key: ImageKey, request_id: usize) {
        let Some(pending) = self.pending_images.get_mut(&key) else {
            return;
        };
        if pending.request_id != request_id {
            return;
        }
        // A fetch that was never deferred completes by its response.
        let Some(deferred) = pending.deferred.take() else {
            return;
        };
        let url = key.url;
        match deferred {
            Deferred::Image(image, retention) => {
                self.apply_loaded_image(&url, request_id, image, retention);
            }
            Deferred::DataUrl => match self.decode_data_url(&url, NestedImages::All, None) {
                Some(image) => {
                    let retention = ImageRetention::Store;
                    self.apply_loaded_image(&url, request_id, image, retention);
                }
                None => {
                    self.fail_image_request(&url, request_id);
                }
            },
        }
    }

    /// The `<image>` images an SVG image decoded now may show: those loaded,
    /// unless no embedder runs the tasks that load the others.
    pub(crate) fn shown_nested_images(&self) -> NestedImages {
        if self.embedder_updates_images {
            NestedImages::Loaded(Arc::clone(&self.svg_image_loads.loaded))
        } else {
            NestedImages::All
        }
    }

    /// Defer the fetch `request_id` for `url` when `image` is an SVG image
    /// whose load event waits; returns whether it did.
    pub(crate) fn defer_svg_image(
        &mut self,
        url: &str,
        request_id: usize,
        image: &ImageData,
        retention: ImageRetention,
    ) -> bool {
        let delays_load = matches!(image, ImageData::Svg(svg) if svg.subresources.delays_load);
        if !delays_load || !self.embedder_updates_images {
            return false;
        }
        let Some(key) = self.pending_image_key(url, request_id) else {
            return false;
        };
        let Some(pending) = self.pending_images.get_mut(&key) else {
            return false;
        };
        pending.deferred = Some(Deferred::Image(image.clone(), retention));
        self.svg_image_loads.queue(Load::Fetch { key, request_id });
        true
    }

    /// Start a pending fetch of the `data:` image `key` for `waiter`, which
    /// shows it without the `<image>` images it left out; a task completes it.
    /// There must be no pending fetch of `key`.
    pub(crate) fn defer_data_image(&mut self, key: ImageKey, waiter: (NodeId, ImageType)) {
        let request_id = crate::net::next_request_id();
        let mut pending = PendingImage::new(request_id, waiter.0, waiter.1);
        pending.deferred = Some(Deferred::DataUrl);
        self.pending_images.insert(key.clone(), pending);
        self.svg_image_loads.queue(Load::Fetch { key, request_id });
    }

    /// `image` loaded from `url`: remember its body when it is a `data:` URL,
    /// and the `<image>` bodies of an SVG image, a task later when it is shown
    /// at once (`at_once`). A cross-origin SVG image's bodies are not
    /// remembered: whether its `<image>` images appear at once would tell the
    /// page what the opaque document contains.
    pub(crate) fn note_image_loaded(&mut self, url: &str, image: &ImageData, at_once: bool) {
        if let Some(body) = BodyDigest::of_data_url(url) {
            self.note_loaded_body(body);
        }
        let ImageData::Svg(svg) = image else {
            return;
        };
        if !svg.origin_clean {
            return;
        }
        // Without an embedder no task would run, and nothing waits anyway.
        if !at_once || !self.embedder_updates_images {
            for body in svg.subresources.bodies() {
                self.note_loaded_body(body);
            }
            return;
        }
        let loaded = &self.svg_image_loads.loaded;
        let unloaded: Vec<_> = svg
            .subresources
            .bodies()
            .filter(|body| !loaded.contains(body))
            .collect();
        if !unloaded.is_empty() {
            let bodies = Load::Bodies(unloaded);
            self.svg_image_loads.queue(bodies);
        }
    }

    fn note_loaded_body(&mut self, body: BodyDigest) {
        let loads = &mut self.svg_image_loads;
        if loads.loaded.contains(&body) {
            return;
        }
        let loaded = Arc::make_mut(&mut loads.loaded);
        loaded.insert(body);
        loads.order.push_back(body);
        if loads.order.len() > MAX_LOADED_BODIES {
            if let Some(oldest) = loads.order.pop_front() {
                loaded.remove(&oldest);
            }
        }
    }
}

#[cfg(test)]
mod tests;
