//! Fallible admission for page-controlled node allocation.
//!
//! Layout admits its anonymous boxes in bounded batches. DOM
//! entry points and parser sinks use these checked methods; admission includes
//! every currently live arena slot, including detached nodes and layout boxes.

use crate::node::Attribute;
use crate::{BaseDocument, DocumentMutator, DomString, NodeId, QualName};

/// Default admission ceiling, configurable through `DocumentConfig`.
/// This bounds node count; it does not bound total page memory.
pub const DEFAULT_NODE_LIMIT: usize = 65_536;

/// Node admission failed before changing the tree or allocating a node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NodeBudgetExceeded;

impl std::fmt::Display for NodeBudgetExceeded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("document node limit exceeded")
    }
}

impl std::error::Error for NodeBudgetExceeded {}

impl BaseDocument {
    /// Live arena slots, including detached and internal layout nodes.
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// The document's configured node admission ceiling.
    pub fn node_limit(&self) -> usize {
        self.node_limit
    }

    /// Check a bounded allocation batch before it mutates the document.
    /// The caller must allocate immediately, without yielding to other writers.
    pub fn check_node_allocation(&self, additional: usize) -> Result<(), NodeBudgetExceeded> {
        if additional > self.node_limit.saturating_sub(self.nodes.len()) {
            return Err(NodeBudgetExceeded);
        }
        Ok(())
    }
}

impl DocumentMutator<'_> {
    /// Replace text atomically with respect to node admission. Clearing an
    /// element and editing existing character data need no additional node.
    pub fn try_set_text_content(
        &mut self,
        id: NodeId,
        text: impl Into<DomString>,
    ) -> Result<(), NodeBudgetExceeded> {
        let text = text.into();
        let kind = self.doc.node_type(id);
        if !text.is_empty()
            && matches!(
                kind,
                crate::dom_api::node_type::ELEMENT | crate::dom_api::node_type::DOCUMENT_FRAGMENT
            )
        {
            self.doc.check_node_allocation(1)?;
        }
        self.set_text_content(id, text);
        Ok(())
    }

    /// Admit the complete clone, including inert template contents, before
    /// allocating any of it. Rejected clones leave no partial detached tree.
    pub fn try_clone_node(&mut self, id: NodeId, deep: bool) -> Result<NodeId, NodeBudgetExceeded> {
        if self.doc.node_type(id) == crate::dom_api::node_type::DOCUMENT {
            return Ok(id);
        }
        let mut pending = vec![id];
        let mut count = 0;
        while let Some(id) = pending.pop() {
            count += 1;
            self.doc.check_node_allocation(count)?;
            if deep {
                let node = &self.doc.nodes[id];
                pending.extend(node.children.iter().copied());
                if let Some(contents) = node
                    .element_data()
                    .and_then(|element| element.template_contents)
                {
                    pending.push(contents);
                }
            }
        }
        Ok(self.clone_node(id, deep))
    }

    /// Allocate an element, or reject admission before parsing its attributes.
    pub fn try_create_element(
        &mut self,
        name: QualName,
        attrs: Vec<Attribute>,
    ) -> Result<NodeId, NodeBudgetExceeded> {
        self.doc.check_node_allocation(1)?;
        Ok(self.create_element(name, attrs))
    }

    /// Allocate a text node without exceeding the configured ceiling.
    pub fn try_create_text_node(
        &mut self,
        text: impl Into<DomString>,
    ) -> Result<NodeId, NodeBudgetExceeded> {
        self.doc.check_node_allocation(1)?;
        Ok(self.create_text_node(text))
    }

    /// Allocate a comment without exceeding the configured ceiling.
    pub fn try_create_comment_node(
        &mut self,
        text: impl Into<DomString>,
    ) -> Result<NodeId, NodeBudgetExceeded> {
        self.doc.check_node_allocation(1)?;
        Ok(self.create_comment_node(text))
    }

    /// Allocate an inert document node in the same bounded arena.
    pub fn try_create_document_node(&mut self) -> Result<NodeId, NodeBudgetExceeded> {
        self.doc.check_node_allocation(2)?;
        Ok(self.create_document_node())
    }

    /// Allocate a fragment in the same bounded arena.
    pub fn try_create_document_fragment(&mut self) -> Result<NodeId, NodeBudgetExceeded> {
        self.doc.check_node_allocation(1)?;
        Ok(self.create_document_fragment())
    }

    /// Allocate a doctype without exceeding the configured ceiling.
    pub fn try_create_doctype(
        &mut self,
        name: &str,
        public_id: &str,
        system_id: &str,
    ) -> Result<NodeId, NodeBudgetExceeded> {
        self.doc.check_node_allocation(1)?;
        Ok(self.create_doctype(name, public_id, system_id))
    }

    /// Allocate a processing instruction without exceeding the ceiling.
    pub fn try_create_processing_instruction(
        &mut self,
        target: &str,
        text: impl Into<DomString>,
    ) -> Result<NodeId, NodeBudgetExceeded> {
        self.doc.check_node_allocation(1)?;
        Ok(self.create_processing_instruction(target, text))
    }

    /// Return or allocate a template's inert contents within the node budget.
    pub fn try_ensure_template_contents(
        &mut self,
        template: NodeId,
    ) -> Result<NodeId, NodeBudgetExceeded> {
        if let Some(contents) = self.try_template_contents(template) {
            return Ok(contents);
        }
        self.doc.check_node_allocation(1)?;
        Ok(self.template_contents(template))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DocumentConfig;

    #[test]
    fn admission_is_atomic_and_reuses_released_capacity() {
        let mut document = BaseDocument::new(DocumentConfig {
            node_limit: Some(3),
            ..DocumentConfig::default()
        });
        let text = document.mutate().try_create_text_node("kept").unwrap();
        assert_eq!(document.node_count(), 3);
        assert_eq!(
            document.mutate().try_create_comment_node("rejected"),
            Err(NodeBudgetExceeded)
        );
        assert_eq!(document.character_data(text), Some("kept"));
        assert_eq!(document.node_count(), 3);
        document.mutate().remove_and_drop_node(text);
        let replacement = document.mutate().try_create_comment_node("new").unwrap();
        assert_ne!(text, replacement);
        assert!(document.get_node(text).is_none());
        assert_eq!(document.node_count(), 3);
    }

    #[test]
    fn batch_admission_handles_overflow_and_zero_capacity() {
        let document = BaseDocument::new(DocumentConfig {
            node_limit: Some(0),
            ..DocumentConfig::default()
        });
        assert_eq!(document.node_limit(), 1);
        assert_eq!(document.check_node_allocation(0), Ok(()));
        assert_eq!(document.check_node_allocation(1), Err(NodeBudgetExceeded));
        assert_eq!(
            document.check_node_allocation(usize::MAX),
            Err(NodeBudgetExceeded)
        );
    }

    #[test]
    fn releasing_slots_also_releases_changed_node_bookkeeping() {
        let mut document = BaseDocument::new(DocumentConfig {
            node_limit: Some(3),
            ..Default::default()
        });
        for _ in 0..10_000 {
            let text = document.mutate().try_create_text_node("temporary").unwrap();
            document.mutate().remove_and_drop_node(text);
            assert_eq!(document.node_count(), 2);
            assert_eq!(document.changed_nodes.len(), 1);
        }
    }

    #[test]
    fn rejected_text_replacement_preserves_children_and_clear_still_works() {
        let mut document = BaseDocument::new(DocumentConfig {
            node_limit: Some(4),
            ..Default::default()
        });
        let element = document
            .mutate()
            .try_create_element(QualName::new(None, crate::ns!(html), "div".into()), vec![])
            .unwrap();
        document
            .mutate()
            .try_set_text_content(element, "original")
            .unwrap();
        assert_eq!(
            document.mutate().try_set_text_content(element, "rejected"),
            Err(NodeBudgetExceeded)
        );
        assert_eq!(
            document.text_content_of(element).as_deref(),
            Some("original")
        );
        document.mutate().try_set_text_content(element, "").unwrap();
        assert!(document.get_node(element).unwrap().children.is_empty());
    }

    #[test]
    fn clone_admission_counts_template_contents_before_allocating() {
        let mut document = BaseDocument::new(DocumentConfig {
            node_limit: Some(7),
            ..Default::default()
        });
        let template = document
            .mutate()
            .try_create_element(
                QualName::new(None, crate::ns!(html), "template".into()),
                vec![],
            )
            .unwrap();
        let contents = document
            .mutate()
            .try_ensure_template_contents(template)
            .unwrap();
        let text = document.mutate().try_create_text_node("inside").unwrap();
        document.mutate().append_children(contents, &[text]);
        assert_eq!(
            document.mutate().try_clone_node(template, true),
            Err(NodeBudgetExceeded)
        );
        assert_eq!(document.node_count(), 5);
        assert_eq!(
            document.text_content_of(contents).as_deref(),
            Some("inside")
        );
        let copy = document.mutate().try_clone_node(template, false).unwrap();
        assert_ne!(copy, template);
        assert_eq!(document.node_count(), 6);
    }
}
