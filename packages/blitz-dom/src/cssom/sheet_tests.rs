use crate::{BaseDocument, DocumentConfig, QualName, local_name, ns};
fn sheet(doc: &mut BaseDocument, text: &str) -> crate::NodeId {
    let root = doc.root_node().id;
    let mut m = doc.mutate();
    let owner = m.create_element(QualName::new(None, ns!(html), local_name!("style")), vec![]);
    m.append_children(root, &[owner]);
    m.set_text_content(owner, text);
    owner
}
#[test]
fn retained_sheet_survives_detachment_and_source_replacement() {
    let mut doc = BaseDocument::new(DocumentConfig::default());
    let node = sheet(&mut doc, "a { color: red }");
    let original = doc.retain_stylesheet(node).unwrap();
    doc.mutate().set_text_content(node, "b {color:blue}");
    assert!(!doc.stylesheet_is_current(&original));
    assert_eq!(doc.retained_stylesheet_rule_count(&original, &[]), Some(1));
    assert_eq!(
        doc.retained_stylesheet_rule_info(&original, &[0])
            .unwrap()
            .attributes[0]
            .1,
        "a"
    );
    let current = doc.retain_stylesheet(node).unwrap();
    doc.mutate().remove_node(node);
    assert!(!doc.stylesheet_is_current(&current));
    assert_eq!(
        doc.retained_stylesheet_insert_rule(&current, &[], "c {color:green}", 1),
        Ok(1)
    );
    assert_eq!(doc.retained_stylesheet_rule_count(&current, &[]), Some(2));
    assert_eq!(
        doc.retained_stylesheet_delete_rule(&current, &[], 0),
        Ok(())
    );
    assert_eq!(doc.retained_stylesheet_rule_count(&current, &[]), Some(1));
    assert_eq!(
        doc.retained_stylesheet_rule_style_css_text(&current, &[0])
            .unwrap(),
        "color: green;"
    );
}
#[test]
fn detached_sheet_accepts_import_rules_without_loading() {
    use blitz_traits::net::{NetHandler, NetProvider, Request};
    struct RejectRequests;
    impl NetProvider for RejectRequests {
        fn fetch(&self, _: usize, _: Request, _: Box<dyn NetHandler>) {
            panic!("detached CSSOM must not request host I/O");
        }
    }
    let mut doc = BaseDocument::new(DocumentConfig {
        net_provider: Some(std::sync::Arc::new(RejectRequests)),
        base_url: Some("https://example.test/".into()),
        ..DocumentConfig::default()
    });
    let owner = sheet(&mut doc, "a{}");
    let retained = doc.retain_stylesheet(owner).unwrap();
    doc.mutate().remove_node(owner);
    assert_eq!(
        doc.retained_stylesheet_insert_rule(&retained, &[], "@import url('/x.css') screen;", 0),
        Ok(0)
    );
    assert_eq!(
        doc.retained_stylesheet_rule_info(&retained, &[0])
            .unwrap()
            .interface,
        "CSSImportRule"
    );
    assert!(
        doc.retained_stylesheet_rule_info(&retained, &[0])
            .unwrap()
            .css_text
            .contains("screen")
    );
}

#[test]
fn retained_sheet_rejects_foreign_document_and_preserves_unrelated_identity() {
    let mut doc = BaseDocument::new(DocumentConfig::default());
    let node = sheet(&mut doc, "a{}");
    let retained = doc.retain_stylesheet(node).unwrap();
    let other = sheet(&mut doc, "b{}");
    doc.mutate().set_text_content(other, "c{}");
    assert!(doc.stylesheet_is_current(&retained));
    let mut foreign = BaseDocument::new(DocumentConfig::default());
    assert_eq!(foreign.retained_stylesheet_rule_count(&retained, &[]), None);
    assert_eq!(
        foreign.retained_stylesheet_insert_rule(&retained, &[], "a{}", 0),
        Err(super::CssomError::NotFound)
    );
    assert_eq!(
        foreign.retained_stylesheet_delete_rule(&retained, &[], 0),
        Err(super::CssomError::NotFound)
    );
}
