//! Shared helpers for blitz-html integration tests.
#![allow(dead_code)]

use std::sync::Arc;

use blitz_dom::{BaseDocument, DocumentConfig, NodeId};
use blitz_html::{DocumentHtmlParser, HtmlProvider};
use blitz_traits::shell::{ColorScheme, Viewport};

/// Parse `html` into a document with a base URL, an 800x600 viewport, and
/// the HTML parser provider (needed for `innerHTML`-style parsing).
pub fn parse(html: &str) -> BaseDocument {
    let config = DocumentConfig {
        base_url: Some("https://example.test/dir/page.html".into()),
        viewport: Some(Viewport::new(800, 600, 1.0, ColorScheme::Light)),
        html_parser_provider: Some(Arc::new(HtmlProvider)),
        ..Default::default()
    };
    let mut doc = BaseDocument::new(config);
    DocumentHtmlParser::parse_into_mutator(&mut doc.mutate(), html);
    doc
}

pub fn q(doc: &BaseDocument, selector: &str) -> NodeId {
    doc.query_selector(selector).unwrap().unwrap_or_else(|| panic!("no match for {selector}"))
}
