//! Transactional inert document parsing with embedder payload admission.
use crate::{BaseDocument, DocumentMetadata, DocumentMutator, NodeBudgetExceeded, NodeId};

impl DocumentMutator<'_> {
    /// Parse a new inert document using this arena's provider. Scripts remain
    /// disabled. The callback may inspect native state, but must not run script
    /// or reenter the engine. Rejection releases the new document and its owned
    /// trees, preserving unrelated nodes and their mutation records.
    ///
    /// Metadata strings must be admitted before copying them into this call.
    /// See HTML: <https://html.spec.whatwg.org/#dom-domparser-parsefromstring>.
    pub fn try_parse_document_admitted<T, E: From<NodeBudgetExceeded>>(
        &mut self,
        markup: &str,
        metadata: DocumentMetadata,
        admit: impl FnOnce(&BaseDocument, NodeId) -> Result<T, E>,
    ) -> Result<(NodeId, T), E> {
        let document = self.try_create_document_node()?;
        let xml = !metadata.document_type.is_html();
        *self.document_metadata_mut(document) = metadata;
        // Scratch IDs must never escape through mutation or selection records.
        let log = self.doc.mutation_log.take();
        let selection_scratch = self.selection_scratch.replace(document);
        let provider = self.doc.html_parser_provider.clone();
        let result = provider
            .try_parse_into_document_node(self, document, markup, xml)
            .map_err(E::from)
            .and_then(|()| admit(self.doc, document));
        if result.is_err() {
            self.remove_and_drop_node(document);
        }
        self.doc.mutation_log = log;
        self.selection_scratch = selection_scratch;
        result.map(|value| (document, value))
    }
}

#[cfg(test)]
mod tests;
