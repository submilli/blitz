//! HTML fragment serialization (`innerHTML` / `outerHTML`) and node kinds.

use blitz_dom::DocumentConfig;
use blitz_html::HtmlDocument;

fn doc(html: &str) -> HtmlDocument {
    let config = DocumentConfig {
        base_url: Some("https://example.test/".into()),
        ..Default::default()
    };
    HtmlDocument::from_html(html, config)
}

fn inner(doc: &HtmlDocument, selector: &str) -> String {
    let id = doc.query_selector(selector).unwrap().unwrap();
    doc.get_node(id).unwrap().inner_html()
}

fn outer(doc: &HtmlDocument, selector: &str) -> String {
    let id = doc.query_selector(selector).unwrap().unwrap();
    doc.get_node(id).unwrap().outer_html()
}

#[test]
fn empty_elements_get_end_tags() {
    let d = doc("<div id=x><span></span><p></p></div>");
    assert_eq!(inner(&d, "#x"), "<span></span><p></p>");
}

#[test]
fn void_elements_have_no_end_tag() {
    let d = doc("<div id=x><br><img src=a.png><input type=text></div>");
    assert_eq!(inner(&d, "#x"), r#"<br><img src="a.png"><input type="text">"#);
}

#[test]
fn text_and_attributes_are_escaped() {
    let d = doc(r#"<div id=x title='a "b" &amp; <c>'>1 &lt; 2 &amp;&amp; 3 &gt; 2&nbsp;!</div>"#);
    assert_eq!(
        outer(&d, "#x"),
        r#"<div id="x" title="a &quot;b&quot; &amp; <c>">1 &lt; 2 &amp;&amp; 3 &gt; 2&nbsp;!</div>"#
    );
}

#[test]
fn raw_text_elements_are_not_escaped() {
    let d = doc("<div id=x><script>if (a < b && c > d) {}</script><style>a > b {}</style></div>");
    assert_eq!(
        inner(&d, "#x"),
        "<script>if (a < b && c > d) {}</script><style>a > b {}</style>"
    );
}

#[test]
fn comments_are_serialized() {
    let d = doc("<div id=x><!-- hi --></div>");
    assert_eq!(inner(&d, "#x"), "<!-- hi -->");
}

#[test]
fn template_serializes_its_contents() {
    let d = doc("<template id=t><b>bold</b></template>");
    assert_eq!(inner(&d, "#t"), "<b>bold</b>");
    assert_eq!(outer(&d, "#t"), r#"<template id="t"><b>bold</b></template>"#);
}

#[test]
fn doctype_is_serialized() {
    let d = doc("<!DOCTYPE html><html><body></body></html>");
    assert!(d.root_node().inner_html().starts_with("<!DOCTYPE html><html>"));
}

#[test]
fn svg_and_foreign_names_keep_case() {
    let d = doc(r#"<div id=x><svg viewBox="0 0 1 1"><foreignObject></foreignObject></svg></div>"#);
    assert_eq!(
        inner(&d, "#x"),
        r#"<svg viewBox="0 0 1 1"><foreignObject></foreignObject></svg>"#
    );
}

