use super::*;
use crate::{DocumentConfig, QualName, ns};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
#[derive(Default)]
struct NetworkSpy(AtomicUsize);
impl blitz_traits::net::NetProvider for NetworkSpy {
    fn fetch(
        &self,
        _: usize,
        _: blitz_traits::net::Request,
        _: Box<dyn blitz_traits::net::NetHandler>,
    ) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
fn setup() -> (BaseDocument, Arc<NetworkSpy>, NodeId, NodeId) {
    let net = Arc::new(NetworkSpy::default());
    let mut doc = BaseDocument::new(DocumentConfig {
        net_provider: Some(net.clone()),
        base_url: Some("https://example.test/".into()),
        ..Default::default()
    });
    let other = doc.mutate().try_create_html_document(None).unwrap();
    let head = doc.document_head(other).unwrap();
    let node = doc
        .mutate()
        .try_create_element(QualName::new(None, ns!(html), "style".into()), vec![])
        .unwrap();
    doc.mutate()
        .try_set_text_content(node, "p {color:red}")
        .unwrap();
    assert!(doc.retain_stylesheet(node).is_none());
    doc.mutate().append_children(head, &[node]);
    (doc, net, other, node)
}
#[test]
fn inert_sheet_rules_remain_associated_until_removal_and_survive_replacement() {
    let (mut doc, net, other, node) = setup();
    let original = doc.retain_stylesheet(node).unwrap();
    assert!(doc.stylesheet_is_current(&original));
    assert_eq!(doc.node_document(node), other);
    assert_eq!(
        doc.retained_stylesheet_rule_style_css_text(&original, &[0])
            .as_deref(),
        Some("color: red;")
    );
    assert_eq!(doc.author_stylesheets().count(), 0);
    doc.mutate()
        .try_set_text_content(node, "p {color:blue}")
        .unwrap();
    let current = doc.retain_stylesheet(node).unwrap();
    assert!(!doc.stylesheet_is_current(&original));
    assert_eq!(
        doc.retained_stylesheet_rule_style_css_text(&original, &[0])
            .as_deref(),
        Some("color: red;")
    );
    assert_eq!(
        doc.retained_stylesheet_rule_style_css_text(&current, &[0])
            .as_deref(),
        Some("color: blue;")
    );
    doc.mutate().remove_node(node);
    assert!(!doc.stylesheet_is_current(&current));
    assert!(doc.retain_stylesheet(node).is_none());
    assert_eq!(doc.retained_stylesheet_rule_count(&current, &[]), Some(1));
    assert_eq!(net.0.load(Ordering::SeqCst), 0);
}
#[test]
fn inert_cssom_imports_and_font_faces_never_load_and_activation_uses_the_live_path() {
    let (mut doc, net, _, node) = setup();
    let sheet = doc.retain_stylesheet(node).unwrap();
    doc.retained_stylesheet_insert_rule(
        &sheet,
        &[],
        "@import url('https://assets.test/inert.css');",
        0,
    )
    .unwrap();
    doc.retained_stylesheet_insert_rule(
        &sheet,
        &[],
        "@font-face{font-family:fixture;src:url('https://assets.test/font.woff2')}",
        1,
    )
    .unwrap();
    assert_eq!(doc.retained_stylesheet_rule_count(&sheet, &[]), Some(3));
    assert_eq!(net.0.load(Ordering::SeqCst), 0);
    assert_eq!(doc.author_stylesheets().count(), 0);
    let active = doc.root_node().id;
    doc.mutate().append_children(active, &[node]);
    assert!(!doc.stylesheet_is_current(&sheet));
    let current = doc.retain_stylesheet(node).unwrap();
    assert_eq!(doc.retained_stylesheet_rule_count(&current, &[]), Some(1));
    doc.retained_stylesheet_insert_rule(
        &current,
        &[],
        "@import url('https://assets.test/active.css');",
        0,
    )
    .unwrap();
    assert_eq!(net.0.load(Ordering::SeqCst), 1);
    assert_eq!(doc.author_stylesheets().count(), 1);
}

#[test]
fn inert_shadow_sheets_preserve_rules_without_loading() {
    let (mut doc, net, other, node) = setup();
    let body = doc.document_body(other).unwrap();
    let host = doc
        .mutate()
        .try_create_element(QualName::new(None, ns!(html), "div".into()), vec![])
        .unwrap();
    doc.mutate().append_children(body, &[host]);
    let shadow = doc
        .mutate()
        .attach_shadow(host, crate::shadow::ShadowRootInit::default())
        .unwrap();
    doc.mutate().append_children(shadow, &[node]);
    doc.mutate()
        .try_set_text_content(
            node,
            "@import url('https://assets.test/shadow.css'); p {color:red}",
        )
        .unwrap();
    let sheet = doc.retain_stylesheet(node).unwrap();
    assert!(doc.stylesheet_is_current(&sheet));
    assert_eq!(doc.retained_stylesheet_rule_count(&sheet, &[]), Some(2));
    assert_eq!(
        doc.retained_stylesheet_rule_style_css_text(&sheet, &[1])
            .as_deref(),
        Some("color: red;")
    );
    doc.retained_stylesheet_insert_rule(
        &sheet,
        &[],
        "@font-face{font-family:fixture;src:url('https://assets.test/shadow.woff2')}",
        1,
    )
    .unwrap();
    assert_eq!(doc.retained_stylesheet_rule_count(&sheet, &[]), Some(3));
    assert_eq!(doc.author_stylesheets().count(), 0);
    assert_eq!(net.0.load(Ordering::SeqCst), 0);
    doc.mutate().remove_node(node);
    assert!(!doc.stylesheet_is_current(&sheet));
    assert!(doc.retain_stylesheet(node).is_none());
    assert_eq!(doc.retained_stylesheet_rule_count(&sheet, &[]), Some(3));
}
