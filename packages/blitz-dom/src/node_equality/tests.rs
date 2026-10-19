use crate::{Attribute, BaseDocument, DocumentConfig, DomString, NodeId, QualName, ns};

fn element(doc: &mut BaseDocument, local: &str) -> NodeId {
    doc.mutate()
        .try_create_element(QualName::new(None, ns!(html), local.into()), vec![])
        .unwrap()
}

fn attribute(prefix: Option<&str>, namespace: &str, local: &str, value: &str) -> Attribute {
    Attribute {
        name: QualName::new(prefix.map(Into::into), namespace.into(), local.into()),
        value: value.into(),
    }
}

#[test]
fn attributes_compare_without_order_or_prefix_but_with_namespace_and_value() {
    let mut a = BaseDocument::new(Default::default());
    let mut b = BaseDocument::new(Default::default());
    let name = QualName::new(None, ns!(html), "div".into());
    let x = a
        .mutate()
        .try_create_element(
            name.clone(),
            vec![
                attribute(None, "", "id", "a"),
                attribute(Some("p"), "urn:a", "x", "v"),
            ],
        )
        .unwrap();
    let y = b
        .mutate()
        .try_create_element(
            name.clone(),
            vec![
                attribute(Some("q"), "urn:a", "x", "v"),
                attribute(None, "", "id", "a"),
            ],
        )
        .unwrap();
    assert!(a.is_equal_node(x, &b, y));
    for attrs in [
        vec![
            attribute(None, "", "id", "b"),
            attribute(Some("p"), "urn:a", "x", "v"),
        ],
        vec![
            attribute(None, "", "id", "a"),
            attribute(Some("p"), "urn:b", "x", "v"),
        ],
        vec![attribute(None, "", "id", "a")],
    ] {
        let different = b.mutate().try_create_element(name.clone(), attrs).unwrap();
        assert!(!a.is_equal_node(x, &b, different));
    }
}

#[test]
fn element_namespace_prefix_and_local_name_participate() {
    let mut doc = BaseDocument::new(Default::default());
    let names = [
        QualName::new(Some("a".into()), "urn:a".into(), "x".into()),
        QualName::new(Some("b".into()), "urn:a".into(), "x".into()),
        QualName::new(Some("a".into()), "urn:b".into(), "x".into()),
        QualName::new(Some("a".into()), "urn:a".into(), "y".into()),
    ];
    let nodes: Vec<_> = names
        .into_iter()
        .map(|name| doc.mutate().try_create_element(name, vec![]).unwrap())
        .collect();
    let copy = doc.mutate().try_clone_node(nodes[0], false).unwrap();
    assert!(doc.is_equal_node(nodes[0], &doc, copy));
    for &other in &nodes[1..] {
        assert!(!doc.is_equal_node(nodes[0], &doc, other));
    }
}

#[test]
fn kinds_pi_targets_and_all_doctype_fields_participate() {
    let mut doc = BaseDocument::new(Default::default());
    let text = doc.mutate().try_create_text_node("same").unwrap();
    let comment = doc.mutate().try_create_comment_node("same").unwrap();
    assert!(!doc.is_equal_node(text, &doc, comment));
    let pi = doc
        .mutate()
        .try_create_processing_instruction("p", "same")
        .unwrap();
    let other_target = doc
        .mutate()
        .try_create_processing_instruction("q", "same")
        .unwrap();
    let other_data = doc
        .mutate()
        .try_create_processing_instruction("p", "different")
        .unwrap();
    assert!(!doc.is_equal_node(pi, &doc, other_target));
    assert!(!doc.is_equal_node(pi, &doc, other_data));
    let doctype = doc
        .mutate()
        .try_create_doctype("html", "public", "system")
        .unwrap();
    for (name, public, system, equal) in [
        ("html", "public", "system", true),
        ("xml", "public", "system", false),
        ("html", "other", "system", false),
        ("html", "public", "other", false),
    ] {
        let other = doc
            .mutate()
            .try_create_doctype(name, public, system)
            .unwrap();
        assert_eq!(doc.is_equal_node(doctype, &doc, other), equal);
    }
    let copy = doc.mutate().try_clone_node(pi, false).unwrap();
    assert!(doc.is_equal_node(pi, &doc, copy));
}

#[test]
fn equality_preserves_utf16_code_units_across_representations() {
    let mut doc = BaseDocument::new(Default::default());
    let utf8 = doc.mutate().try_create_text_node("a😀").unwrap();
    let utf16 = doc
        .mutate()
        .try_create_text_node(DomString::from_utf16("a😀".encode_utf16().collect()))
        .unwrap();
    assert!(doc.is_equal_node(utf8, &doc, utf16));
    let a = doc
        .mutate()
        .try_create_text_node(DomString::from_utf16(vec![0xd800]))
        .unwrap();
    let b = doc
        .mutate()
        .try_create_text_node(DomString::from_utf16(vec![0xd801]))
        .unwrap();
    let c = doc.mutate().try_create_text_node("�").unwrap();
    assert!(!doc.is_equal_node(a, &doc, b));
    assert!(!doc.is_equal_node(a, &doc, c));
}

#[test]
fn child_order_structure_and_text_node_boundaries_participate() {
    let mut doc = BaseDocument::new(Default::default());
    let a = element(&mut doc, "div");
    let one = doc.mutate().try_create_text_node("a").unwrap();
    let two = doc.mutate().try_create_text_node("b").unwrap();
    doc.mutate().append_children(a, &[one, two]);
    let copy = doc.mutate().try_clone_node(a, true).unwrap();
    assert!(doc.is_equal_node(a, &doc, copy));
    doc.mutate().append_children(a, &[one]);
    assert!(!doc.is_equal_node(a, &doc, copy));
    doc.mutate().try_set_text_content(a, "ab").unwrap();
    assert!(!doc.is_equal_node(a, &doc, copy));
    let fragment = doc.mutate().try_create_document_fragment().unwrap();
    assert!(!doc.is_equal_node(a, &doc, fragment));
}

#[test]
fn shadow_and_template_contents_are_not_dom_children() {
    let mut doc = BaseDocument::new(Default::default());
    let a = element(&mut doc, "div");
    let b = element(&mut doc, "div");
    let shadow = doc
        .mutate()
        .attach_shadow(a, crate::shadow::ShadowRootInit::default())
        .unwrap();
    let text = doc.mutate().try_create_text_node("shadow").unwrap();
    doc.mutate().append_children(shadow, &[text]);
    assert!(doc.is_equal_node(a, &doc, b));
    let a = element(&mut doc, "template");
    let b = element(&mut doc, "template");
    let contents = doc.mutate().try_ensure_template_contents(a).unwrap();
    let text = doc.mutate().try_create_text_node("inert").unwrap();
    doc.mutate().append_children(contents, &[text]);
    assert!(doc.is_equal_node(a, &doc, b));
}

#[test]
fn documents_fragments_and_stale_nodes_are_checked() {
    let mut a = BaseDocument::new(Default::default());
    let mut b = BaseDocument::new(Default::default());
    assert!(a.is_equal_node(a.root_node().id, &b, b.root_node().id));
    let x = a.mutate().try_create_document_fragment().unwrap();
    let y = b.mutate().try_create_document_fragment().unwrap();
    assert!(a.is_equal_node(x, &b, y));
    a.mutate().remove_and_drop_node(x);
    assert!(!a.is_equal_node(x, &a, x));
    assert!(!a.is_equal_node(x, &b, y));
}

#[test]
fn deep_and_wide_trees_use_bounded_iterative_work() {
    let mut doc = BaseDocument::new(DocumentConfig {
        node_limit: Some(50_000),
        ..Default::default()
    });
    let root = element(&mut doc, "div");
    let mut leaf = root;
    for _ in 0..10_000 {
        let child = element(&mut doc, "div");
        doc.mutate().append_children(leaf, &[child]);
        leaf = child;
    }
    let copy = doc.mutate().try_clone_node(root, true).unwrap();
    assert!(doc.is_equal_node(root, &doc, copy));
    let text = doc.mutate().try_create_text_node("changed").unwrap();
    doc.mutate().append_children(leaf, &[text]);
    assert!(!doc.is_equal_node(root, &doc, copy));
    let wide = element(&mut doc, "div");
    for _ in 0..4_096 {
        let child = doc.mutate().try_create_text_node("same").unwrap();
        doc.mutate().append_children(wide, &[child]);
    }
    let copy = doc.mutate().try_clone_node(wide, true).unwrap();
    assert!(doc.is_equal_node(wide, &doc, copy));
}
