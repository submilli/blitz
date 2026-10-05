//! Initial blank-document fallback is captured at native navigable creation,
//! even when an embedder materializes the JavaScript window much later.
use crate::{BaseDocument, NodeId};
use std::rc::Rc;

impl BaseDocument {
    pub fn initial_frame_base_url(&self, node: NodeId) -> Option<Rc<url::Url>> {
        self.initial_frame_bases.get(&node).cloned()
    }

    pub(crate) fn capture_frame_bases(&mut self, root: NodeId) {
        if !self.nodes[root].flags.is_in_document() {
            return;
        }
        for node in self.frame_nodes(root) {
            if !self.nodes[node].flags.is_in_document()
                || self.initial_frame_bases.contains_key(&node)
            {
                continue;
            }
            let base = self.document_base_snapshot(node);
            self.initial_frame_bases.insert(node, base);
        }
    }

    pub(crate) fn discard_frame_bases(&mut self, root: NodeId) {
        if self.initial_frame_bases.is_empty() || !self.nodes[root].flags.is_in_document() {
            return;
        }
        for node in self.frame_nodes(root) {
            self.initial_frame_bases.remove(&node);
        }
    }

    fn frame_nodes(&self, root: NodeId) -> Vec<NodeId> {
        let mut frames = Vec::new();
        let mut pending = vec![root];
        while let Some(node) = pending.pop() {
            let node = &self.nodes[node];
            pending.extend(node.children.iter().rev().copied());
            if let Some(element) = node.element_data() {
                pending.extend(element.shadow_root);
                if element.name.ns == crate::ns!(html) && element.name.local.as_ref() == "iframe" {
                    frames.push(node.id);
                }
            }
        }
        frames
    }
}
