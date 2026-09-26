//! Text in SVG images uses the document's SVG fonts
//! (`DocumentConfig::svg_fonts`), not whatever the system has installed.

mod common;

use std::sync::Arc;

use blitz_dom::{BaseDocument, DocumentConfig, FontRole, build_svg_font_db};
use blitz_html::{DocumentHtmlParser, HtmlProvider};
use blitz_traits::shell::{ColorScheme, Viewport};
use common::q;

const DEJAVU_SANS: &[u8] = include_bytes!("../../../examples/wasm_hello/assets/DejaVuSans.woff2");

/// Width of the text in an inline SVG, laid out with `fonts` only.
fn svg_text_width(fonts: &[(FontRole, &[u8])], family: &str) -> f32 {
    let mut doc = BaseDocument::new(DocumentConfig {
        viewport: Some(Viewport::new(800, 600, 1.0, ColorScheme::Light)),
        html_parser_provider: Some(Arc::new(HtmlProvider)),
        svg_fonts: Some(build_svg_font_db(fonts)),
        ..Default::default()
    });
    let html = format!(
        "<svg id=s width=200 height=40><text x=0 y=20 font-family='{family}'>Hello</text></svg>"
    );
    DocumentHtmlParser::parse_into_mutator(&mut doc.mutate(), &html);
    doc.resolve(0.0);
    let node = doc.get_node(q(&doc, "#s")).unwrap();
    let tree = node.element_data().unwrap().svg_data().expect("inline svg is parsed");
    tree.root().abs_bounding_box().width()
}

#[test]
fn svg_text_renders_with_supplied_fonts() {
    assert!(svg_text_width(&[(FontRole::SansSerif, DEJAVU_SANS)], "sans-serif") > 10.0);
}

#[test]
fn svg_text_uses_no_system_fonts() {
    // A family every system has, but the document was given no fonts.
    assert_eq!(svg_text_width(&[], "Arial, Helvetica, sans-serif"), 0.0);
}

#[test]
fn unknown_and_missing_generic_families_fall_back_to_a_supplied_font() {
    // Only a sans-serif font: `serif`, `monospace` and unknown names still render.
    let fonts = [(FontRole::SansSerif, DEJAVU_SANS)];
    for family in ["serif", "monospace", "No Such Font"] {
        assert!(svg_text_width(&fonts, family) > 10.0, "{family}");
    }
}
