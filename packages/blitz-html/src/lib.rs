#![allow(clippy::collapsible_if)]

mod bounded_parser;
mod custom_element_pause;
mod encoding;
mod handler_elements;
pub use encoding::{detect_xml_encoding, prescan_html_encoding};
mod html_document;
mod html_sink;
mod parser_handle;
mod streaming;

pub use custom_element_pause::DefinitionLookup;
pub use html_document::HtmlDocument;
pub use html_sink::DocumentHtmlParser;
pub use html_sink::HtmlProvider;
pub use html_sink::{BorrowedMutator, DocAccess, HtmlSink, SharedDocument};
pub use streaming::{ParseStep, StreamingParser};
