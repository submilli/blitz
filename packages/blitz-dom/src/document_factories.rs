//! Fixed inert document trees, independent of the parser or a JS engine.
//! https://dom.spec.whatwg.org/#dom-domimplementation-createhtmldocument
use crate::{DocumentMutator, DomString, NodeBudgetExceeded, NodeId, QualName, ns};

impl DocumentMutator<'_> {
    /// Construct an inert XML document, optionally adopting an existing doctype.
    /// Name/type validation and complete node admission precede all mutation.
    /// The namespace selects MIME identity even when no root is requested.
    /// https://dom.spec.whatwg.org/#dom-domimplementation-createdocument
    pub fn try_create_xml_document(
        &mut self,
        namespace: &str,
        qualified: &str,
        doctype: Option<NodeId>,
    ) -> Result<NodeId, crate::dom_api::DomError> {
        use crate::{
            DocumentType,
            dom_api::{DomError, node_type},
        };
        let name = if qualified.is_empty() {
            None
        } else {
            Some(crate::validate_qualified_name(namespace, qualified)?)
        };
        if doctype.is_some_and(|id| {
            self.doc
                .get_node(id)
                .is_none_or(|node| !matches!(node.data, crate::node::NodeData::Doctype { .. }))
        }) {
            return Err(DomError::HierarchyRequest);
        }
        self.doc
            .check_node_allocation(2 + usize::from(name.is_some()))
            .map_err(|_| DomError::QuotaExceeded)?;
        let document = self.create_document_node();
        self.document_metadata_mut(document).document_type = match namespace {
            value if value == ns!(html).as_ref() => DocumentType::Xhtml,
            value if value == ns!(svg).as_ref() => DocumentType::Svg,
            _ => DocumentType::Xml,
        };
        // Both insertions are valid by construction: a checked doctype and a
        // new Element enter a new empty Document in their required order.
        if let Some(doctype) = doctype {
            debug_assert_eq!(self.doc.node_type(doctype), node_type::DOCUMENT_TYPE);
            self.append_children(document, &[doctype]);
        }
        if let Some(name) = name {
            let root = self.create_element(name, Vec::new());
            self.append_children(document, &[root]);
        }
        Ok(document)
    }

    /// Construct a detached HTML document with its standard doctype and skeleton.
    /// Node admission includes its private template document and every child.
    /// The embedder admits retained string payload before calling, and supplies
    /// origin metadata before exposing the result. No parser or script runs.
    pub fn try_create_html_document(
        &mut self,
        title: Option<DomString>,
    ) -> Result<NodeId, NodeBudgetExceeded> {
        self.doc
            .check_node_allocation(6 + 2 * usize::from(title.is_some()))?;
        let document = self.create_document_node();
        let doctype = self.create_doctype("html", "", "");
        let html = self.create_element(html_name("html"), Vec::new());
        let head = self.create_element(html_name("head"), Vec::new());
        let body = self.create_element(html_name("body"), Vec::new());
        self.append_children(html, &[head, body]);
        if let Some(title) = title {
            let element = self.create_element(html_name("title"), Vec::new());
            let text = self.create_text_node(title);
            self.append_children(element, &[text]);
            self.append_children(head, &[element]);
        }
        self.append_children(document, &[doctype, html]);
        Ok(document)
    }
}

fn html_name(local: &str) -> QualName {
    QualName::new(None, ns!(html), local.into())
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod xml_tests;
