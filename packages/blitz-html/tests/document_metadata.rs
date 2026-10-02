//! Parsing metadata belongs to each document and survives tree mutations.
mod common;
use blitz_dom::DocumentType;
use blitz_html::DocumentHtmlParser;
use common::parse;

#[test]
fn html_mode_survives_parser_and_child_removal() {
    for (markup, quirks) in [("<p>x", true), ("<!doctype html><p>x", false)] {
        let mut doc = parse(markup);
        let root = doc.root_node().id;
        assert_eq!(doc.document_metadata(root).quirks, quirks);
        assert!(!doc.document_metadata(root).xml_parse_error);
        let children = doc.root_node().children.to_vec();
        for child in children {
            doc.mutate().remove_and_drop_node(child);
        }
        assert_eq!(doc.document_metadata(root).quirks, quirks);
    }
}

#[test]
fn detached_modes_are_independent_and_xml_errors_keep_identity() {
    let mut doc = parse("<!doctype html><p>main");
    let root = doc.root_node().id;
    let mut m = doc.mutate();
    let html = m.try_create_document_node().unwrap();
    DocumentHtmlParser::try_parse_into_document_node(&mut m, html, "<p>detached", false).unwrap();
    let xml = m.try_create_document_node().unwrap();
    m.document_metadata_mut(xml).document_type = DocumentType::Svg;
    DocumentHtmlParser::try_parse_into_document_node(&mut m, xml, "<root><bad></root>", true)
        .unwrap();
    assert!(!m.doc.document_metadata(root).quirks);
    assert!(m.doc.document_metadata(html).quirks);
    assert!(!m.doc.document_metadata(xml).quirks);
    assert_eq!(
        m.doc.document_metadata(xml).document_type,
        DocumentType::Svg
    );
    let error = m.doc.get_node(xml).unwrap().children[0];
    assert_eq!(
        m.doc
            .get_node(error)
            .unwrap()
            .element_data()
            .unwrap()
            .name
            .local
            .as_ref(),
        "parsererror"
    );
}

#[test]
fn xml_parser_sets_type_without_an_embedder_override() {
    let mut doc = parse("<p>main");
    let root = doc.root_node().id;
    let mut m = doc.mutate();
    let xml = m.try_create_document_node().unwrap();
    DocumentHtmlParser::try_parse_into_document_node(&mut m, xml, "<root/>", true).unwrap();
    assert_eq!(
        m.doc.document_metadata(xml).document_type,
        DocumentType::Xml
    );
    assert!(!m.doc.document_metadata(xml).quirks);
    assert!(m.doc.document_metadata(root).quirks);
}

#[test]
fn unclosed_xml_reports_an_error_document() {
    let mut doc = parse("<!doctype html>");
    let mut m = doc.mutate();
    let xml = m.try_create_document_node().unwrap();
    DocumentHtmlParser::try_parse_into_document_node(&mut m, xml, "<root>", true).unwrap();
    let error = m.doc.get_node(xml).unwrap().children[0];
    assert_eq!(
        m.doc
            .get_node(error)
            .unwrap()
            .element_data()
            .unwrap()
            .name
            .local
            .as_ref(),
        "parsererror"
    );
}

#[test]
fn xml_parse_status_is_not_inferred_from_element_names() {
    let mut document = parse("");
    let mut m = document.mutate();
    for (source, failed) in [
        ("<root>", true),
        ("<parsererror/>", false),
        ("<root><parsererror/></root>", false),
        (
            "<parsererror xmlns='http://www.mozilla.org/newlayout/xml/parsererror.xml'>XML parsing error</parsererror>",
            false,
        ),
    ] {
        let xml = m.try_create_document_node().unwrap();
        DocumentHtmlParser::try_parse_into_document_node(&mut m, xml, source, true).unwrap();
        assert_eq!(m.doc.document_metadata(xml).xml_parse_error, failed);
    }
}
