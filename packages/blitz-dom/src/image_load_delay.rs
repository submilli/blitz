//! Native image dependencies, including shared requests and non-HTML images.
use crate::{BaseDocument, NodeId};

impl BaseDocument {
    /// Whether queued HTML image updates or pending native image waiters belong
    /// to `document`. This includes image inputs, CSS layers and SVG filter
    /// images sharing a fetch; the request's original initiator cannot identify
    /// every client. Image events already transferred to an embedder are its
    /// own load-delay responsibility.
    pub fn image_resources_delay_load(&self, document: NodeId) -> bool {
        self.images_delay_load(document)
            || self.pending_images.values().any(|pending| {
                pending.waiters.iter().any(|&(node, _)| {
                    self.get_node(node)
                        .is_some_and(|node| node.owner_document == document)
                })
            })
    }
}

#[cfg(test)]
mod tests;
