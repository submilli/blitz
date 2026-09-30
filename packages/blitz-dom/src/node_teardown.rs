//! Iterative teardown of native tree ownership, including inert template trees.

use crate::{BaseDocument, Node, NodeId};

impl BaseDocument {
    pub(crate) fn drop_node_ignoring_parent(&mut self, id: NodeId) -> Option<Node> {
        self.drop_node_ignoring_parent_with(id, &mut |_| {})
    }

    /// The caller detaches the root first. Anonymous boxes borrow DOM children;
    /// template, shadow and pseudo trees instead belong to their owner.
    pub(crate) fn drop_node_ignoring_parent_with(
        &mut self,
        root: NodeId,
        on_drop: &mut dyn FnMut(NodeId),
    ) -> Option<Node> {
        let mut pending = vec![(root, true)];
        let mut removed_root = None;
        while let Some((id, owns_children)) = pending.pop() {
            let Some(node) = self.remove_node_from_tree(id) else {
                continue;
            };
            on_drop(id);
            self.shadow_hosts.remove(&id);
            pending.extend(node.anonymous_blocks.iter().map(|&id| (id, false)));
            if owns_children {
                pending.extend(
                    node.template_document
                        .filter(|&document| document != id)
                        .map(|id| (id, true)),
                );
                pending.extend(node.children.iter().map(|&id| (id, true)));
                pending.extend(node.before().into_iter().map(|id| (id, true)));
                pending.extend(node.after().into_iter().map(|id| (id, true)));
                if let Some(element) = node.element_data() {
                    pending.extend(element.shadow_root.into_iter().map(|id| (id, true)));
                    pending.extend(element.template_contents.into_iter().map(|id| (id, true)));
                }
            }
            if id == root {
                removed_root = Some(node);
            }
        }
        removed_root
    }

    /// Release layout-owned wrappers while preserving the DOM children they
    /// merely refer to. Nested anonymous blocks are ownership edges.
    pub(crate) fn deallocate_anonymous_block(&mut self, root: NodeId) {
        let mut pending = vec![root];
        while let Some(id) = pending.pop() {
            if let Some(node) = self.remove_node_from_tree(id) {
                pending.extend(node.anonymous_blocks.iter().copied());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DocumentConfig, QualName, ns, shadow::ShadowRootInit};

    #[test]
    fn deep_template_and_shadow_ownership_is_released_without_recursion() {
        let mut doc = BaseDocument::new(DocumentConfig::default());
        let root = doc.mutate().try_create_document_fragment().unwrap();
        let mut parent = root;
        let mut ids = vec![root];
        for _ in 0..5_000 {
            let mut m = doc.mutate();
            let template = m
                .try_create_element(QualName::new(None, ns!(html), "template".into()), vec![])
                .unwrap();
            m.append_children(parent, &[template]);
            parent = m.try_ensure_template_contents(template).unwrap();
            ids.extend([template, parent]);
        }
        let mut m = doc.mutate();
        let host = m
            .try_create_element(QualName::new(None, ns!(html), "div".into()), vec![])
            .unwrap();
        m.append_children(parent, &[host]);
        let shadow = m.attach_shadow(host, ShadowRootInit::default()).unwrap();
        ids.extend([host, shadow]);
        drop(m);
        doc.mutate().remove_and_drop_node(root);
        assert_eq!(doc.node_count(), 2);
        assert_eq!(doc.changed_nodes.len(), 1);
        assert!(ids.into_iter().all(|id| doc.get_node(id).is_none()));
        assert!(doc.shadow_hosts.is_empty());
    }
}
