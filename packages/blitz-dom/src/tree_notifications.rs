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
    /// Adjust live range state without running script or reborrowing the document.
    fn range_mutation(&self, _document: &BaseDocument, _mutation: crate::ranges::RangeMutation) {}
    /// Text-control selection changed or was activated by focus; no script runs here.
    fn control_selection(&self, _document: &BaseDocument, _node: NodeId, _focused: bool) {}
}

impl BaseDocument {
    pub fn set_tree_observer(&mut self, observer: Rc<dyn TreeObserver>) {
        self.tree_observer = Some(observer);
    }

    pub(crate) fn notify_removing(&mut self, node: NodeId) {
        self.base_subtree_removing(node);
        self.discard_frame_bases(node);
        if self
            .get_node(node)
            .is_some_and(|node| node.parent.is_some())
            && let Some(observer) = &self.tree_observer
        {
            observer.removing(self, node);
        }
    }

    pub(crate) fn notify_inserted(&mut self, node: NodeId) {
        self.base_subtree_inserted(node);
        self.capture_frame_bases(node);
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
