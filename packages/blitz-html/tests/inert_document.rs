//! Inert parsing admission against the actual HTML and XML providers.
use blitz_dom::{
    BaseDocument, DocumentConfig, DocumentMetadata, DocumentType, NodeBudgetExceeded, NodeId,
};
use blitz_html::HtmlProvider;
use std::sync::Arc;
fn document(limit: usize) -> BaseDocument {
    BaseDocument::new(DocumentConfig {
        node_limit: Some(limit),
        html_parser_provider: Some(Arc::new(HtmlProvider)),
        ..Default::default()
    })
}
fn parse(doc: &mut BaseDocument, markup: &str, kind: DocumentType) -> NodeId {
    doc.mutate()
        .try_parse_document_admitted(
            markup,
            DocumentMetadata {
                document_type: kind,
                url: Some("https://example.test/page".parse().unwrap()),
                ..Default::default()
            },
            |_, _| Ok::<_, NodeBudgetExceeded>(()),
        )
        .unwrap()
        .0
}
#[test]
fn html_parser_populates_inert_skeleton_title_quirks_and_template_owners() {
    let mut doc = document(128);
    let id = parse(
        &mut doc,
        "<!doctype html><title>parsed</title><template><b>inside</b></template><p>text</p>",
        DocumentType::Html,
    );
    assert_eq!(
        doc.document_title_with_limit(id, 100)
            .unwrap()
            .as_str_lossy(),
        "parsed"
    );
    assert!(!doc.document_metadata(id).quirks);
    assert!(doc.document_body(id).is_some());
    let template = doc.query_selector_in(id, "template").unwrap().unwrap();
    let contents = doc
        .get_node(template)
        .unwrap()
        .element_data()
        .unwrap()
        .template_contents
        .unwrap();
    assert_eq!(doc.node_document(contents), doc.template_document(id));
    assert!(!doc.get_node(template).unwrap().flags.is_in_document());
    let quirks = parse(&mut doc, "<p>quirks</p>", DocumentType::Html);
    assert!(doc.document_metadata(quirks).quirks);
    assert!(doc.root_node().children.is_empty());
}
#[test]
fn xml_mime_namespace_case_and_default_encoding_survive_the_string_parser() {
    for kind in [
        DocumentType::TextXml,
        DocumentType::Xml,
        DocumentType::Xhtml,
        DocumentType::Svg,
    ] {
        let mut doc = document(64);
        let id = parse(
            &mut doc,
            "<?xml version='1.0' encoding='windows-1252'?><Root xmlns='urn:test' a='&amp;'><Child/></Root>",
            kind,
        );
        let root = doc
            .get_node(id)
            .unwrap()
            .children
            .iter()
            .copied()
            .find(|&n| doc.node_type(n) == 1)
            .unwrap();
        let name = &doc.get_node(root).unwrap().element_data().unwrap().name;
        assert_eq!(name.local.as_ref(), "Root");
        assert_eq!(name.ns.as_ref(), "urn:test");
        assert_eq!(doc.document_metadata(id).document_type, kind);
        assert_eq!(doc.document_metadata(id).encoding, None);
        assert!(!doc.document_metadata(id).xml_parse_error);
        assert_eq!(doc.document_url(root).as_str(), "https://example.test/page");
    }
}
#[test]
fn malformed_xml_returns_an_owned_parsererror_document_without_partial_children() {
    let mut doc = document(128);
    for markup in ["<a><b></a>", "<x:p/>", "<a>"] {
        let id = parse(&mut doc, markup, DocumentType::Xml);
        assert!(doc.document_metadata(id).xml_parse_error);
        let children = &doc.get_node(id).unwrap().children;
        assert_eq!(children.len(), 1);
        let error = doc.get_node(children[0]).unwrap();
        assert_eq!(
            error.element_data().unwrap().name.local.as_ref(),
            "parsererror"
        );
        assert_eq!(doc.node_document(error.id), id);
    }
}
#[test]
fn real_parser_exhaustion_releases_all_html_xml_and_template_nodes() {
    for kind in [DocumentType::Html, DocumentType::Xml] {
        for limit in 2..18 {
            let mut doc = document(limit);
            let before = doc.node_count();
            let source = if kind.is_html() {
                "<template><template><b>x</b></template></template><p>end"
            } else {
                "<a><b><c><d><e><f><g><h>text</h></g></f></e></d></c></b></a>"
            };
            let result = doc.mutate().try_parse_document_admitted(
                source,
                DocumentMetadata {
                    document_type: kind,
                    ..Default::default()
                },
                |_, _| Ok::<_, NodeBudgetExceeded>(()),
            );
            if result.is_err() {
                assert_eq!(doc.node_count(), before, "{kind:?} at {limit}");
            }
        }
    }
}
#[test]
fn payload_rejection_releases_nested_templates_and_malformed_xml_error_trees() {
    for (kind, source) in [
        (
            DocumentType::Html,
            "<template><template><b>x</b></template></template><table>foster<tr><td>cell",
        ),
        (DocumentType::Xml, "<a><b></a>"),
    ] {
        let mut doc = document(128);
        let before = doc.node_count();
        for _ in 0..8 {
            let result = doc.mutate().try_parse_document_admitted(
                source,
                DocumentMetadata {
                    document_type: kind,
                    ..Default::default()
                },
                |_, _| Err::<(), _>(NodeBudgetExceeded),
            );
            assert_eq!(result, Err(NodeBudgetExceeded));
            assert_eq!(doc.node_count(), before);
        }
    }
}

#[test]
fn frameset_recovery_discards_the_replaced_implied_body_before_admission() {
    let mut doc = document(128);
    let before = doc.node_count();
    let id = parse(
        &mut doc,
        "<input type=hidden><frameset>",
        DocumentType::Html,
    );
    assert_eq!(doc.node_count(), before + 5); // Document pair, html, head, frameset.
    assert_eq!(
        doc.document_body(id).map(|id| doc.node_name(id)),
        Some("FRAMESET".into())
    );
    doc.mutate().remove_and_drop_node(id);
    assert_eq!(doc.node_count(), before);
    for _ in 0..8 {
        let result = doc.mutate().try_parse_document_admitted(
            "<input type=hidden><frameset>",
            Default::default(),
            |dom, _| {
                assert_eq!(dom.node_count(), before + 5);
                Err::<(), _>(NodeBudgetExceeded)
            },
        );
        assert_eq!(result, Err(NodeBudgetExceeded));
        assert_eq!(doc.node_count(), before);
    }
}
#[derive(Default)]
struct NetworkSpy(std::sync::atomic::AtomicUsize);
impl blitz_traits::net::NetProvider for NetworkSpy {
    fn fetch(
        &self,
        _: usize,
        _: blitz_traits::net::Request,
        _: Box<dyn blitz_traits::net::NetHandler>,
    ) {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}
#[test]
fn inert_styles_neither_fetch_imports_and_fonts_nor_enter_the_active_stylist() {
    let net = Arc::new(NetworkSpy::default());
    let mut doc = BaseDocument::new(DocumentConfig {
        node_limit: Some(128),
        html_parser_provider: Some(Arc::new(HtmlProvider)),
        net_provider: Some(net.clone()),
        ..Default::default()
    });
    let id = parse(
        &mut doc,
        "<style>@import url('https://assets.test/import.css');@font-face{font-family:fixture;src:url('https://assets.test/font.woff2')}p{color:red}</style>",
        DocumentType::Html,
    );
    let style = doc.query_selector_in(id, "style").unwrap().unwrap();
    assert_eq!(net.0.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert_eq!(doc.author_stylesheets().count(), 0);
    doc.mutate()
        .try_set_text_content(
            style,
            "@import url('https://assets.test/changed.css');p{color:blue}",
        )
        .unwrap();
    assert_eq!(net.0.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert_eq!(doc.author_stylesheets().count(), 0);
    let active = doc.root_node().id;
    doc.mutate().append_children(active, &[style]);
    assert_eq!(net.0.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(doc.author_stylesheets().count(), 1);
    assert_eq!(doc.node_document(style), active);
}
#[test]
fn a_style_detached_before_flush_does_not_fetch_or_install() {
    let net = Arc::new(NetworkSpy::default());
    let mut doc = BaseDocument::new(DocumentConfig {
        node_limit: Some(128),
        html_parser_provider: Some(Arc::new(HtmlProvider)),
        net_provider: Some(net.clone()),
        ..Default::default()
    });
    let id = parse(
        &mut doc,
        "<style>@import url('https://assets.test/import.css');</style>",
        DocumentType::Html,
    );
    let style = doc.query_selector_in(id, "style").unwrap().unwrap();
    let active = doc.root_node().id;
    {
        let mut m = doc.mutate();
        m.append_children(active, &[style]);
        m.remove_node(style);
    }
    assert_eq!(net.0.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert_eq!(doc.author_stylesheets().count(), 0);
}
