//! A pending image fetch owns indexed waiters and a unique completion identity.
use crate::{BaseDocument, NodeId, util::ImageType};
use std::collections::HashSet;

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
    pub(crate) fn remove_image_waiter(&mut self, node: NodeId, source: &str) {
        let empty = self.pending_images.get_mut(source).is_some_and(|request| {
            request.waiters.remove(&(node, ImageType::Image));
            request.waiters.is_empty()
        });
        if empty {
            self.pending_images.remove(source);
        }
    }

    pub(crate) fn forget_image_input(&mut self, id: NodeId) {
        self.failed_image_inputs.remove(&id);
        let source = self
            .get_node(id)
            .and_then(|node| node.element_data())
            .and_then(|e| e.form_state.image_input_source.clone());
        if let Some(source) = source {
            self.remove_image_waiter(id, &source);
        }
    }

    pub(crate) fn take_image_waiters(
        &mut self,
        url: &str,
        request_id: usize,
    ) -> Option<HashSet<(NodeId, ImageType)>> {
        if self
            .pending_images
            .get(url)
            .is_none_or(|pending| pending.request_id != request_id)
        {
            return None;
        }
        self.pending_images
            .remove(url)
            .map(|pending| pending.waiters)
    }
}
