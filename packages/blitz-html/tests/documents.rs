//! Detached documents (for `DOMParser` and `createHTMLDocument`).

mod common;

use blitz_dom::dom_api::node_type;
use common::parse;

#[test]
fn html_parses_into_a_detached_document_node() {
    let mut doc = parse("<p>main</p>");
    let mut m = doc.mutate();
    let other = m.create_document_node();
    m.doc.html_parser_provider.clone().parse_into_document_node(
        &mut m,
        other,
        "<!DOCTYPE html><title>t</title><p id=x>parsed</p><script>not run</script>",
        false,
    );
    drop(m);
    assert_eq!(doc.node_type(other), node_type::DOCUMENT);
    assert!(doc.get_node(other).unwrap().parent.is_none());
    assert!(!doc.is_connected(other));
    let html = doc.get_node(other).unwrap().inner_html();
    assert!(html.starts_with("<!DOCTYPE html><html><head><title>t</title></head><body><p id=\"x\">parsed</p>"));
    // The main document is untouched (including its doctype-less start).
    assert!(doc.root_node().inner_html().starts_with("<html>"));
    assert!(doc.root_node().inner_html().contains("<p>main</p>"));
    // Scripts in parsed documents never run, so none are queued.
    assert!(doc.take_connected_scripts().is_empty());
}

#[test]
fn xml_parses_into_a_detached_document_node() {
    let mut doc = parse("");
    let mut m = doc.mutate();
    let other = m.create_document_node();
    m.doc.html_parser_provider.clone().parse_into_document_node(&mut m, other, "<root><item a='1'/></root>", true);
    drop(m);
    let root = doc.get_node(other).unwrap().children[0];
    assert_eq!(doc.node_name(root), "root");
}
