//! Document parsing identity and environment, independent of tree attachment.
use crate::{BaseDocument, DocumentMutator, NodeId, node::NodeData};

/// MIME values supported by the document factories.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DocumentType {
    #[default]
    Html,
    TextXml,
    Xml,
    Xhtml,
    Svg,
}

impl DocumentType {
    pub fn content_type(self) -> &'static str {
        match self {
            Self::Html => "text/html",
            Self::TextXml => "text/xml",
            Self::Xml => "application/xml",
            Self::Xhtml => "application/xhtml+xml",
            Self::Svg => "image/svg+xml",
        }
    }

    pub fn is_html(self) -> bool {
        self == Self::Html
    }
}

/// Parser state persists after the parser and document children are discarded.
#[derive(Clone, Debug, Default)]
pub struct DocumentMetadata {
    pub document_type: DocumentType,
    pub quirks: bool,
    /// Detached document URL; the active document uses the embedder's live URL.
    pub url: Option<url::Url>,
    /// Detached factories snapshot their creator's origin domain.
    pub domain: String,
}

impl BaseDocument {
    /// Metadata of a live node's document, including detached nodes.
    pub fn document_metadata(&self, node: NodeId) -> &DocumentMetadata {
        let NodeData::Document(data) = &self.nodes[self.node_document(node)].data else {
            unreachable!("every node has a document owner")
        };
        &data.metadata
    }
}

impl DocumentMutator<'_> {
    /// Set metadata on a live Document, before exposing its wrapper.
    pub fn document_metadata_mut(&mut self, document: NodeId) -> &mut DocumentMetadata {
        let NodeData::Document(data) = &mut self.doc.nodes[document].data else {
            unreachable!("metadata mutations require a document node")
        };
        &mut data.metadata
    }
}
