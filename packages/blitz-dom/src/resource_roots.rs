//! Native node references carried by resource work, parsers and continuations.
//!
//! Handlers and queued completions share a small token, not the document. The
//! document keeps weak tokens so cancellation and consumed responses release
//! roots without locking or calling back into the document on another thread.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::{Arc, Weak};

use crate::{BaseDocument, NodeId};

/// A native continuation's claim on a node during detached-node reclamation.
///
/// Clones share one token. Dropping the last clone releases the claim without
/// borrowing or retaining the document. This does not prevent explicit removal
/// with `remove_and_drop_node`; generation checks still apply after such removal.
#[derive(Clone, Debug)]
pub struct NodeLease(Arc<NodeId>);

impl NodeLease {
    pub fn id(&self) -> NodeId {
        *self.0
    }
}

#[derive(Default)]
pub(crate) struct ResourceRoots {
    entries: RefCell<HashMap<NodeId, Weak<NodeId>>>,
    next_prune: Cell<usize>,
}

impl ResourceRoots {
    fn prune(&self, roots: &mut HashMap<NodeId, Weak<NodeId>>) {
        roots.retain(|_, pin| pin.strong_count() != 0);
        // Amortize scanning when many responses remain queued. Live tokens can
        // grow the map geometrically; stale bookkeeping stays within that bound.
        self.next_prune.set(roots.len().saturating_mul(2).max(1024));
    }
}

impl BaseDocument {
    /// Preserve this node and its native topology while a parser or other
    /// native continuation may use it. A stale ID never preserves a new slot.
    pub fn lease_node(&self, node: NodeId) -> NodeLease {
        NodeLease(self.resource_pin(node))
    }

    pub(crate) fn resource_pin(&self, node: NodeId) -> Arc<NodeId> {
        let mut roots = self.resource_roots.entries.borrow_mut();
        if let Some(pin) = roots.get(&node).and_then(Weak::upgrade) {
            return pin;
        }
        // Bound stale bookkeeping even when no reclamation checkpoint runs.
        if roots.len() >= self.resource_roots.next_prune.get() {
            self.resource_roots.prune(&mut roots);
        }
        let pin = Arc::new(node);
        roots.insert(node, Arc::downgrade(&pin));
        pin
    }

    /// Nodes used by resource completions or registered native/parser leases. A
    /// reclamation checkpoint must preserve these nodes and their topology.
    /// Ordinary synchronous removal remains the embedder's responsibility.
    pub fn pending_resource_nodes(&self) -> Vec<NodeId> {
        let mut roots = self.resource_roots.entries.borrow_mut();
        self.resource_roots.prune(&mut roots);
        roots.keys().copied().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DocumentConfig;

    #[test]
    fn dropped_resource_tokens_release_roots_and_stale_metadata() {
        let mut doc = BaseDocument::new(DocumentConfig::default());
        for _ in 0..5_000 {
            let id = doc.mutate().try_create_text_node("x").unwrap();
            let pin = doc.resource_pin(id);
            assert_eq!(doc.pending_resource_nodes(), [id]);
            drop(pin);
            doc.mutate().remove_and_drop_node(id);
        }
        assert!(doc.pending_resource_nodes().is_empty());
        assert!(doc.resource_roots.entries.borrow().is_empty());
    }
    #[test]
    fn stale_bookkeeping_stays_bounded_with_many_live_tokens() {
        let mut doc = BaseDocument::new(DocumentConfig::default());
        let live: Vec<_> = (0..2_048)
            .map(|_| {
                let id = doc.mutate().try_create_text_node("x").unwrap();
                doc.resource_pin(id)
            })
            .collect();
        for _ in 0..10_000 {
            let id = doc.mutate().try_create_text_node("x").unwrap();
            drop(doc.resource_pin(id));
            doc.mutate().remove_and_drop_node(id);
            assert!(doc.resource_roots.entries.borrow().len() <= live.len() * 2);
        }
        assert_eq!(doc.pending_resource_nodes().len(), live.len());
        drop(live);
        assert!(doc.pending_resource_nodes().is_empty());
    }
}
