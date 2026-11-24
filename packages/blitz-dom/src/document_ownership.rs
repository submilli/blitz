//! Node-document identity survives detachment and changes only through adoption.
use crate::{BaseDocument, DocumentMutator, NodeId};

impl BaseDocument {
    /// The node document, including the document itself for Document nodes.
    pub fn node_document(&self, node: NodeId) -> NodeId {
        self.nodes[node].owner_document
    }

    /// HTML's appropriate template contents owner document. Inert template
    /// documents use themselves, so nested templates share the same owner.
    pub fn template_document(&self, node: NodeId) -> NodeId {
        let document = self.node_document(node);
        self.nodes[document]
            .template_document
            .expect("documents allocate an inert template owner")
    }
}

impl DocumentMutator<'_> {
    /// Unlink the arena-local ends when a template's contents move separately
    /// from their host. The embedder retains their association across arenas.
    /// A refused or staged transfer must leave both links intact.
    pub fn unlink_template_contents(&mut self, contents: NodeId) -> Option<NodeId> {
        let host = self.doc.get_node(contents)?.template_host?;
        let element = self.doc.get_node_mut(host)?.element_data_mut()?;
        if element.template_contents != Some(contents) {
            return None;
        }
        element.template_contents = None;
        self.doc.nodes[contents].template_host = None;
        Some(host)
    }

    /// Restore both local links when a separately adopted contents fragment
    /// and its host meet again. The fragment keeps its adopted node document.
    pub fn relink_template_contents(
        &mut self,
        host: NodeId,
        contents: NodeId,
    ) -> Result<(), crate::dom_api::DomError> {
        use crate::dom_api::DomError::HierarchyRequest;
        let element = self
            .doc
            .get_node(host)
            .and_then(|node| node.element_data())
            .ok_or(HierarchyRequest)?;
        if element.name.ns != crate::ns!(html)
            || element.name.local != crate::local_name!("template")
            || element.template_contents.is_some()
        {
            return Err(HierarchyRequest);
        }
        let fragment = self.doc.get_node(contents).ok_or(HierarchyRequest)?;
        if !matches!(fragment.data, crate::NodeData::DocumentFragment)
            || fragment.shadow_root_data.is_some()
            || fragment.parent.is_some()
            || fragment.template_host.is_some()
        {
            return Err(HierarchyRequest);
        }
        if self
            .doc
            .is_host_including_inclusive_ancestor(contents, host)
        {
            return Err(HierarchyRequest);
        }
        self.doc.nodes[host]
            .element_data_mut()
            .expect("validated template")
            .template_contents = Some(contents);
        self.doc.nodes[contents].template_host = Some(host);
        Ok(())
    }

    /// Update the inclusive subtree's document without changing its topology.
    /// Includes shadow roots and template contents, which are not DOM children.
    /// Callers pass a live Document from this arena. No script runs here.
    pub fn set_node_document(&mut self, node: NodeId, document: NodeId) {
        if self.doc.node_document(node) == document {
            return;
        }
        let mut pending = vec![(node, document)];
        while let Some((id, document)) = pending.pop() {
            let template_document = self.doc.template_document(document);
            let node = &mut self.doc.nodes[id];
            let old_document = node.owner_document;
            node.owner_document = document;
            pending.extend(node.children.iter().rev().map(|&id| (id, document)));
            if let Some(element) = node.element_data() {
                pending.extend(element.shadow_root.map(|id| (id, document)));
                pending.extend(element.template_contents.map(|id| (id, template_document)));
            }
            // Adoption changes the declaration parser's document base. This
            // also covers parser-created nodes staged in an inert document.
            if old_document != document
                && self.doc.nodes[id]
                    .attr(crate::local_name!("style"))
                    .is_some()
            {
                let base = self.doc.style_base_url(id);
                let node = &mut self.doc.nodes[id];
                if let Some(element) = node.element_data_mut() {
                    element.flush_style_attribute(&self.doc.guard, &base);
                    node.set_restyle_hint(style::invalidation::element::restyle_hints::RestyleHint::RESTYLE_STYLE_ATTRIBUTE);
                }
            }
            if old_document == document {
                continue;
            }
            // Adoption is a relevant mutation of an `<img>`, and moves an
            // `feImage`'s request to its new document.
            self.queue_image_update_if_sourced(id);
            #[cfg(feature = "svg")]
            self.fe_image_inserted(id);
            if matches!(
                self.doc.custom_element_state(id),
                crate::custom_elements::CustomElementState::Custom
            ) {
                self.doc.record_custom_element_reaction(
                    crate::custom_elements::CustomElementReaction::Adopted {
                        element: id,
                        old_document,
                        new_document: document,
                    },
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{BaseDocument, DocumentConfig, QualName, ns};

    #[test]
    fn detached_ownership_and_connectivity_are_independent() {
        let mut doc = BaseDocument::new(DocumentConfig::default());
        let mut m = doc.mutate();
        let document = m.try_create_document_node().unwrap();
        let node = m
            .try_create_element(QualName::new(None, ns!(html), "div".into()), vec![])
            .unwrap();
        m.set_node_document(node, document);
        assert_eq!(m.doc.node_document(node), document);
        assert!(!m.doc.is_connected(node));
        m.pre_insert(node, document, None).unwrap();
        assert!(m.doc.is_connected(node));
        assert!(m.doc.is_connected(document));
        assert!(!m.doc.nodes[node].flags.is_in_document());
        m.remove_node(node);
        assert_eq!(m.doc.node_document(node), document);
        assert!(!m.doc.is_connected(node));
        let clone = m.try_clone_node(node, true).unwrap();
        assert_eq!(m.doc.node_document(clone), document);
    }

    #[test]
    fn secondary_document_activation_preserves_inert_documents() {
        let mut doc = BaseDocument::new(DocumentConfig::default());
        let mut m = doc.mutate();
        let active = m.create_document_node();
        let inert = m.create_document_node();
        let child = m.create_element(QualName::new(None, ns!(html), "iframe".into()), vec![]);
        m.pre_insert(child, active, None).unwrap();
        assert!(!m.doc.nodes[child].flags.is_in_document());

        m.activate_document_node(active);
        assert!(m.doc.nodes[active].flags.is_in_document());
        assert!(m.doc.nodes[child].flags.is_in_document());
        assert!(!m.doc.nodes[inert].flags.is_in_document());

        m.deactivate_document_node(active);
        assert!(!m.doc.nodes[child].flags.is_in_document());
        assert!(m.doc.is_connected(child));
        assert!(!m.doc.nodes[inert].flags.is_in_document());
    }

    #[test]
    fn adoption_updates_shadow_and_nested_template_documents() {
        let mut doc = BaseDocument::new(DocumentConfig::default());
        let mut m = doc.mutate();
        let other = m.try_create_document_node().unwrap();
        let host = m
            .try_create_element(QualName::new(None, ns!(html), "div".into()), vec![])
            .unwrap();
        let shadow = m
            .attach_shadow(host, crate::shadow::ShadowRootInit::default())
            .unwrap();
        let template = m
            .try_create_element(QualName::new(None, ns!(html), "template".into()), vec![])
            .unwrap();
        let contents = m.try_ensure_template_contents(template).unwrap();
        let text = m.try_create_text_node("text").unwrap();
        m.append_children(contents, &[text]);
        m.append_children(shadow, &[template]);
        m.set_node_document(host, other);
        let inert = m.doc.template_document(other);
        for id in [host, shadow, template] {
            assert_eq!(m.doc.node_document(id), other);
        }
        for id in [contents, text] {
            assert_eq!(m.doc.node_document(id), inert);
        }
        assert_eq!(m.doc.template_document(inert), inert);
        assert_eq!(m.doc.tree_root(text), contents);
        assert_eq!(m.doc.host_including_parent(contents), Some(template));
        assert!(m.doc.is_host_including_inclusive_ancestor(host, text));
        assert!(!m.doc.is_inclusive_ancestor(host, text));
        assert_eq!(m.doc.shadow_host_of(contents), None);

        let copy = m.deep_clone_node(template);
        let copied_contents = m.try_template_contents(copy).unwrap();
        assert_ne!(copied_contents, contents);
        assert_eq!(m.doc.host_including_parent(copied_contents), Some(copy));
        assert_eq!(m.doc.nodes[copied_contents].children.len(), 1);
        assert_ne!(m.doc.nodes[copied_contents].children[0], text);
    }

    #[test]
    fn a_live_detached_node_retains_its_document_but_not_sibling_detached_nodes() {
        let mut doc = BaseDocument::new(DocumentConfig::default());
        let (other, live, dead) = {
            let mut m = doc.mutate();
            let other = m.try_create_document_node().unwrap();
            let live = m.try_create_text_node("live").unwrap();
            let dead = m.try_create_text_node("dead").unwrap();
            m.set_node_document(live, other);
            m.set_node_document(dead, other);
            (other, live, dead)
        };
        assert_eq!(doc.reclaim_detached_nodes(&[live]), vec![dead]);
        assert!(doc.get_node(other).is_some());
        assert_eq!(doc.node_document(live), other);
        let removed = doc.reclaim_detached_nodes(&[]);
        assert!(removed.contains(&live) && removed.contains(&other));
        assert_eq!(doc.node_count(), 2);
    }

    #[test]
    fn document_pair_allocation_is_admitted_atomically() {
        let mut doc = BaseDocument::new(DocumentConfig {
            node_limit: Some(3),
            ..Default::default()
        });
        assert!(doc.mutate().try_create_document_node().is_err());
        assert_eq!(doc.node_count(), 2);
        assert!(doc.mutate().try_create_text_node("remaining slot").is_ok());
    }
}
