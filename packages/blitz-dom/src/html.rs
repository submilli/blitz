use crate::{BaseDocument, Document, DocumentConfig, DocumentMutator, PlainDocument};
use blitz_traits::node_id::NodeId;

pub trait HtmlParserProvider {
    /// Fallible fragment parsing. Providers with bounded allocation override
    /// this to report exhaustion before the caller commits a replacement.
    fn try_parse_inner_html(
        &self,
        mutr: &mut DocumentMutator<'_>,
        element_id: NodeId,
        html: &str,
    ) -> Result<(), crate::NodeBudgetExceeded> {
        self.parse_inner_html(mutr, element_id, html);
        Ok(())
    }

    /// Fallible inert-document parsing; normal page loads may instead stop
    /// at the budget and retain their successfully parsed prefix.
    fn try_parse_into_document_node(
        &self,
        mutr: &mut DocumentMutator<'_>,
        document: NodeId,
        markup: &str,
        xml: bool,
    ) -> Result<(), crate::NodeBudgetExceeded> {
        self.parse_into_document_node(mutr, document, markup, xml);
        Ok(())
    }

    fn parse_inner_html<'m, 'doc>(
        &self,
        mutr: &'m mut DocumentMutator<'doc>,
        element_id: NodeId,
        html: &str,
    );

    /// Parse `markup` as a whole document into the detached document node
    /// `document` (see `DocumentMutator::create_document_node`). Scripts
    /// in it never run. `xml` selects the XML parser.
    fn parse_into_document_node<'m, 'doc>(
        &self,
        mutr: &'m mut DocumentMutator<'doc>,
        document: NodeId,
        markup: &str,
        xml: bool,
    ) {
        let _ = (mutr, document, markup, xml);
    }

    /// Parse a full HTML document (e.g. the contents of an `<iframe>`).
    ///
    /// The default implementation ignores the HTML and returns an empty document.
    fn parse_document(&self, html: &str, config: DocumentConfig) -> Box<dyn Document> {
        let _ = html;
        Box::new(PlainDocument(BaseDocument::new(config)))
    }
}

pub struct DummyHtmlParserProvider;
impl HtmlParserProvider for DummyHtmlParserProvider {
    fn parse_inner_html<'m, 'doc>(
        &self,
        mutr: &'m mut DocumentMutator<'doc>,
        element_id: NodeId,
        html: &str,
    ) {
        let _ = mutr;
        let _ = element_id;
        let _ = html;
        // Do nothing for now
        //
        // TODO: do something:
        // - Print warning?
        // - Parse HTML as plain text?
    }
}
