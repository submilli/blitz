//! HTML document bases are independent of document URL and origin authority.
//! https://html.spec.whatwg.org/multipage/semantics.html#the-base-element

use crate::{BaseDocument, NodeId, node::NodeData};
use std::rc::Rc;
use url::Url;

/// Only the first href-bearing HTML base participates. Its parsed URL stays
/// frozen until that element's href or its position as the first base changes.
#[derive(Default, Debug)]
pub(crate) struct DocumentBase {
    first: Option<NodeId>,
    frozen: Option<Rc<Url>>,
    fallback_snapshot: Option<Rc<Url>>,
}

impl BaseDocument {
    /// The exposed document URL is separate from inherited host authority.
    pub fn document_url(&self, node: NodeId) -> Url {
        let document = self.node_document(node);
        self.document_metadata(document)
            .url
            .clone()
            .unwrap_or_else(|| {
                if document == self.root_node_id {
                    self.url().clone()
                } else {
                    Url::parse("about:blank").expect("static about-blank URL")
                }
            })
    }

    pub(crate) fn style_base_url(&self, node: NodeId) -> style::stylesheets::UrlExtraData {
        style::stylesheets::UrlExtraData(style::servo_arc::Arc::new(self.document_base_url(node)))
    }

    /// The HTML document base of a live node's owner document. Ordinary DOM
    /// children participate; shadow and template contents belong to other trees.
    pub fn document_base_url(&self, node: NodeId) -> Url {
        let document = self.node_document(node);
        self.document_base(document).frozen.as_ref().map_or_else(
            || self.document_fallback_base_url(document),
            |url| (**url).clone(),
        )
    }

    /// About-blank embedders capture an inherited fallback separately from the
    /// document's URL. Changing a parent later never changes this snapshot.
    pub fn document_fallback_base_url(&self, node: NodeId) -> Url {
        let document = self.node_document(node);
        let metadata = self.document_metadata(document);
        if let Some(base) = &metadata.inherited_base_url {
            return base.clone();
        }
        if document == self.root_node_id {
            return self.url().clone();
        }
        metadata
            .url
            .clone()
            .unwrap_or_else(|| Url::parse("about:blank").expect("static about-blank URL"))
    }

    pub(crate) fn document_base_snapshot(&mut self, node: NodeId) -> Rc<Url> {
        let document = self.node_document(node);
        let base = self.document_base(document);
        if let Some(url) = base.frozen.as_ref().or(base.fallback_snapshot.as_ref()) {
            return Rc::clone(url);
        }
        let url = Rc::new(self.document_fallback_base_url(document));
        self.document_base_mut(document).fallback_snapshot = Some(Rc::clone(&url));
        url
    }

    pub(crate) fn invalidate_base_snapshot(&mut self, document: NodeId) {
        self.document_base_mut(document).fallback_snapshot = None;
    }

    pub(crate) fn base_subtree_inserted(&mut self, node: NodeId) {
        if !self.nodes[node].flags.is_in_document()
            && self.tree_root(node) != self.node_document(node)
        {
            return;
        }
        if self.first_base_in(node).is_some() {
            let document = self.node_document(node);
            self.refresh_document_base(document);
        }
    }

    /// Pre-removal runs before topology changes. Clear a removed first base now
    /// and promote its successor after removal, before effects are flushed.
    pub(crate) fn base_subtree_removing(&mut self, node: NodeId) {
        if self.nodes[node].parent.is_none() {
            return;
        }
        let document = self.node_document(node);
        if self
            .document_base(document)
            .first
            .is_some_and(|first| self.is_inclusive_ancestor(node, first))
        {
            *self.document_base_mut(document) = DocumentBase::default();
            self.dirty_base_documents.insert(document);
        }
    }

    pub(crate) fn base_href_changed(&mut self, node: NodeId, name: &crate::QualName) {
        if name.ns != crate::ns!() || name.local.as_ref() != "href" || !self.is_html_base(node) {
            return;
        }
        let document = self.node_document(node);
        if self.tree_root(node) != document {
            return;
        }
        if self.document_base(document).first == Some(node) {
            *self.document_base_mut(document) = DocumentBase::default();
        }
        self.refresh_document_base(document);
    }

    pub(crate) fn refresh_dirty_document_bases(&mut self) {
        for document in std::mem::take(&mut self.dirty_base_documents) {
            if self.nodes.contains_key(document) {
                self.refresh_document_base(document);
            }
        }
    }

    fn refresh_document_base(&mut self, document: NodeId) {
        let first = self.first_base_in(document);
        if self.document_base(document).first == first {
            return;
        }
        let frozen = first
            .map(|first| {
                let fallback = self.document_fallback_base_url(document);
                let href = self
                    .null_attribute(first, "href")
                    .expect("the first base has an href attribute")
                    .value
                    .as_str_lossy();
                fallback
                    .join(href)
                    .ok()
                    .filter(|url| !matches!(url.scheme(), "data" | "javascript"))
                    .unwrap_or(fallback)
            })
            .map(Rc::new);
        *self.document_base_mut(document) = DocumentBase {
            first,
            frozen,
            fallback_snapshot: None,
        };
        self.dirty_base_documents.remove(&document);
    }

    fn first_base_in(&self, root: NodeId) -> Option<NodeId> {
        let mut pending = vec![root];
        while let Some(node) = pending.pop() {
            if self.is_html_base(node) && self.null_attribute(node, "href").is_some() {
                return Some(node);
            }
            pending.extend(self.nodes[node].children.iter().rev().copied());
        }
        None
    }

    fn is_html_base(&self, node: NodeId) -> bool {
        self.nodes[node].element_data().is_some_and(|element| {
            element.name.ns == crate::ns!(html) && element.name.local.as_ref() == "base"
        })
    }

    fn document_base(&self, document: NodeId) -> &DocumentBase {
        let NodeData::Document(data) = &self.nodes[document].data else {
            unreachable!("every node's owner is a document")
        };
        &data.base
    }

    fn document_base_mut(&mut self, document: NodeId) -> &mut DocumentBase {
        let NodeData::Document(data) = &mut self.nodes[document].data else {
            unreachable!("every node's owner is a document")
        };
        &mut data.base
    }
}

#[cfg(test)]
#[path = "document_base/tests.rs"]
mod tests;
