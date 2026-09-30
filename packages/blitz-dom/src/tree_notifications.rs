//! Synchronous topology notifications for script-free embedder bookkeeping.

use std::rc::Rc;

use crate::{BaseDocument, NodeId};

/// An embedder may update native cursors and connection state here, but must not
/// run script, mutate the document, or reborrow its owning document cell.
pub trait TreeObserver {
    /// Called before a parented subtree loses its old topology.
    fn removing(&self, document: &BaseDocument, node: NodeId);
    /// Called after an inserted subtree has its new parent and connection flags.
    fn inserted(&self, document: &BaseDocument, node: NodeId);
}

impl BaseDocument {
    pub fn set_tree_observer(&mut self, observer: Rc<dyn TreeObserver>) {
        self.tree_observer = Some(observer);
    }

    pub(crate) fn notify_removing(&self, node: NodeId) {
        if self
            .get_node(node)
            .is_some_and(|node| node.parent.is_some())
            && let Some(observer) = &self.tree_observer
        {
            observer.removing(self, node);
        }
    }

    pub(crate) fn notify_inserted(&self, node: NodeId) {
        if let Some(observer) = &self.tree_observer {
            observer.inserted(self, node);
        }
    }

    /// DOM NodeIterator pre-removing steps. The returned position is independent
    /// of filtering, which must never run during a removal.
    /// <https://dom.spec.whatwg.org/#nodeiterator-pre-removing-steps>
    pub fn iterator_pre_removal(
        &self,
        root: NodeId,
        reference: NodeId,
        before: bool,
        removed: NodeId,
    ) -> Option<(NodeId, bool)> {
        if self.is_inclusive_ancestor(removed, root)
            || !self.is_inclusive_ancestor(removed, reference)
        {
            return None;
        }
        if before {
            let mut current = removed;
            while current != root {
                if let Some(next) = self.get_node(current)?.forward(1).map(|node| node.id) {
                    return Some((next, true));
                }
                current = self.get_node(current)?.parent?;
            }
        }
        let parent = self.get_node(removed)?.parent?;
        let Some(mut previous) = self.get_node(removed)?.backward(1).map(|node| node.id) else {
            return Some((parent, false));
        };
        while let Some(last) = self.get_node(previous)?.children.last() {
            previous = *last;
        }
        Some((previous, false))
    }
}
