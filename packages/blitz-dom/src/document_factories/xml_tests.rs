use super::*;
use crate::{BaseDocument, DocumentConfig, dom_api::DomError};
fn document(limit: usize) -> BaseDocument {
    BaseDocument::new(DocumentConfig {
        node_limit: Some(limit),
        ..Default::default()
    })
}
#[test]
fn namespace_selects_xml_identity_even_without_a_root() {
    for (namespace, mime) in [
        ("", "application/xml"),
        ("urn:custom", "application/xml"),
        (ns!(html).as_ref(), "application/xhtml+xml"),
        (ns!(svg).as_ref(), "image/svg+xml"),
    ] {
        for qualified in ["", "Root"] {
            let mut doc = document(5);
            let active = doc.root_node().id;
            let before = doc.node_count();
            let id = doc
                .mutate()
                .try_create_xml_document(namespace, qualified, None)
                .unwrap();
            assert_eq!(
                doc.node_count(),
                before + 2 + usize::from(!qualified.is_empty())
            );
            assert_eq!(doc.document_metadata(id).document_type.content_type(), mime);
            assert!(!doc.document_metadata(id).document_type.is_html());
            assert!(!doc.document_metadata(id).quirks);
            assert_eq!(doc.document_url(id).as_str(), "about:blank");
            assert_eq!(doc.document_base_url(id).as_str(), "about:blank");
            assert!(doc.document_metadata(id).referrer.is_empty());
            assert!(doc.document_head(id).is_none());
            assert!(doc.document_body(id).is_none());
            let children = &doc.get_node(id).unwrap().children;
            assert_eq!(children.len(), usize::from(!qualified.is_empty()));
            if let Some(&root) = children.first() {
                let el = doc.get_node(root).unwrap().element_data().unwrap();
                assert_eq!(el.name.local.as_ref(), qualified);
                assert_eq!(el.name.ns.as_ref(), namespace);
                assert_eq!(doc.node_document(root), id);
                assert!(doc.is_connected(root));
                assert!(!doc.get_node(root).unwrap().flags.is_in_document());
            }
            assert!(doc.get_node(active).unwrap().children.is_empty());
            assert_ne!(doc.template_document(id), id);
        }
    }
}
#[test]
fn xml_roots_preserve_prefix_and_case() {
    let mut doc = document(5);
    let id = doc
        .mutate()
        .try_create_xml_document("urn:Case", "p:Root", None)
        .unwrap();
    let root = doc.get_node(id).unwrap().children[0];
    let name = &doc.get_node(root).unwrap().element_data().unwrap().name;
    assert_eq!(name.ns.as_ref(), "urn:Case");
    assert_eq!(name.local.as_ref(), "Root");
    assert_eq!(name.prefix.as_deref(), Some("p"));
}
#[test]
fn xml_factory_validation_leaves_existing_nodes_and_mutations_unchanged() {
    for (namespace, name, error) in [
        ("", "p:Root", DomError::Namespace),
        ("urn:test", "a b", DomError::InvalidCharacter),
        ("urn:test", "xml:Root", DomError::Namespace),
    ] {
        let mut doc = document(8);
        let active = doc.root_node().id;
        let doctype = doc.mutate().create_doctype("old", "", "");
        doc.mutate().append_children(active, &[doctype]);
        doc.set_mutation_recording(true);
        doc.take_logged_mutations();
        let before = doc.node_count();
        assert_eq!(
            doc.mutate()
                .try_create_xml_document(namespace, name, Some(doctype)),
            Err(error)
        );
        assert_eq!(doc.node_count(), before);
        assert_eq!(doc.get_node(doctype).unwrap().parent, Some(active));
        assert!(doc.take_logged_mutations().is_empty());
        assert_eq!(
            doc.mutate()
                .try_create_xml_document("", "Root", Some(active)),
            Err(DomError::HierarchyRequest)
        );
        assert_eq!(doc.node_count(), before);
        assert!(doc.take_logged_mutations().is_empty());
    }
}
#[test]
fn xml_quota_failure_precedes_doctype_adoption_and_allocation() {
    for (name, extra) in [("", 2), ("Root", 3)] {
        let mut doc = document(3 + extra - 1);
        let active = doc.root_node().id;
        let doctype = doc.mutate().create_doctype("old", "public", "system");
        doc.mutate().append_children(active, &[doctype]);
        doc.set_mutation_recording(true);
        doc.take_logged_mutations();
        let before = doc.node_count();
        assert_eq!(
            doc.mutate()
                .try_create_xml_document("", name, Some(doctype)),
            Err(DomError::QuotaExceeded)
        );
        assert_eq!(doc.node_count(), before);
        assert_eq!(doc.get_node(doctype).unwrap().parent, Some(active));
        assert_eq!(doc.node_document(doctype), active);
        assert!(doc.take_logged_mutations().is_empty());
    }
}
#[test]
fn xml_factory_adopts_the_same_doctype_and_admits_its_exact_node_count() {
    for (name, extra) in [("", 2), ("Root", 3)] {
        let mut doc = document(3 + extra);
        let active = doc.root_node().id;
        let doctype = doc.mutate().create_doctype("old", "public", "system");
        doc.mutate().append_children(active, &[doctype]);
        let id = doc
            .mutate()
            .try_create_xml_document("", name, Some(doctype))
            .unwrap();
        assert_eq!(doc.node_count(), 3 + extra);
        assert_eq!(doc.get_node(id).unwrap().children[0], doctype);
        assert_eq!(doc.node_document(doctype), id);
        assert_eq!(doc.get_node(doctype).unwrap().parent, Some(id));
        assert!(doc.get_node(active).unwrap().children.is_empty());
        assert!(doc.mutate().try_create_text_node("overflow").is_err());
        doc.mutate().remove_and_drop_node(id);
        assert_eq!(doc.node_count(), 2);
    }
}
