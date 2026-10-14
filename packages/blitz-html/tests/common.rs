//! Shared helpers for blitz-html integration tests.
#![allow(dead_code)]

use std::sync::Arc;

use blitz_dom::{BaseDocument, DocumentConfig, NodeId};
use blitz_html::{DocumentHtmlParser, HtmlProvider};
use blitz_traits::shell::{ColorScheme, Viewport};

/// A 10px square left float.
pub const FLOAT: &str = "<img style='float:left;width:10px;height:10px'>";

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

/// Parse `html` and lay it out with the bundled Liberation Sans as the only
/// font, so text metrics do not depend on the machine.
pub fn laid_out(html: &str) -> BaseDocument {
    let mut doc = parse(html);
    *doc.font_ctx().lock().unwrap() = blitz_dom::build_single_font_ctx(include_bytes!(
        "../../blitz-dom/assets/test-fonts/LiberationSans-Regular.ttf"
    ));
    doc.resolve(0.0);
    doc
}

/// Parse `html` and lay it out with the bundled Liberation Sans for every
/// family and the bullet font, as the browser lays pages out.
pub fn laid_out_with_browser_fonts(html: &str) -> BaseDocument {
    laid_out_at_scale(html, 1.0)
}

/// `laid_out_with_browser_fonts` at `scale` device pixels per CSS pixel.
pub fn laid_out_at_scale(html: &str, scale: f32) -> BaseDocument {
    let mut doc = parse(html);
    let mut viewport = doc.viewport().clone();
    viewport.set_hidpi_scale(scale);
    doc.set_viewport(viewport);
    *doc.font_ctx().lock().unwrap() = blitz_dom::build_font_ctx(&[(
        blitz_dom::FontRole::SansSerif,
        include_bytes!("../../blitz-dom/assets/test-fonts/LiberationSans-Regular.ttf"),
    )]);
    doc.resolve(0.0);
    doc
}

pub fn q(doc: &BaseDocument, selector: &str) -> NodeId {
    doc.query_selector(selector)
        .unwrap()
        .unwrap_or_else(|| panic!("no match for {selector}"))
}
