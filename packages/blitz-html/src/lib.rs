#![allow(clippy::collapsible_if)]

mod html_document;
mod html_sink;
mod streaming;

pub use html_document::HtmlDocument;
pub use html_sink::DocumentHtmlParser;
pub use html_sink::HtmlProvider;
pub use html_sink::{BorrowedMutator, DocAccess, HtmlSink, SharedDocument};
pub use streaming::{ParseStep, StreamingParser};
