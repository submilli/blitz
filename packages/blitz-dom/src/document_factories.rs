//! Fixed inert document trees, independent of the parser or a JS engine.
//! https://dom.spec.whatwg.org/#dom-domimplementation-createhtmldocument
use crate::{DocumentMutator, DomString, NodeBudgetExceeded, NodeId, QualName, ns};

impl DocumentMutator<'_> {
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
