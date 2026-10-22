use super::*;
use crate::{BaseDocument, mutations::MutationRecord, ns};

fn name(local: &str) -> QualName {
    QualName::new(None, ns!(), local.into())
}
fn element(document: &mut BaseDocument) -> NodeId {
    document
        .mutate()
        .try_create_element(QualName::new(None, ns!(html), "div".into()), Vec::new())
        .unwrap()
}
fn ensured(document: &mut BaseDocument, owner: NodeId, name: &QualName) -> AttributeId {
    document
        .try_ensure_attribute_node(owner, name, 32)
        .unwrap()
        .unwrap()
}
fn value(document: &BaseDocument, id: AttributeId) -> &DomString {
    document.attribute_node_value(id).unwrap()
}

#[test]
fn identities_read_live_values_then_keep_removed_values_independently() {
    let mut document = BaseDocument::new(Default::default());
    let root = document.root_node().id;
    let owner = element(&mut document);
    document
        .mutate()
        .set_attribute(owner, name("class"), "before");
    let id = ensured(&mut document, owner, &name("class"));
    assert_eq!(ensured(&mut document, owner, &name("class")), id);
    document
        .mutate()
        .set_attribute(owner, name("class"), "changed");
    assert_eq!(value(&document, id), "changed");
    document.mutate().clear_attribute(owner, name("class"));
    assert_eq!(document.attribute_node(id).unwrap().owner_element(), None);
    assert_eq!(document.attribute_node_document(id), Some(root));
    assert_eq!(value(&document, id), "changed");
    document
        .mutate()
        .set_attribute(owner, name("class"), "new attribute");
    assert_ne!(ensured(&mut document, owner, &name("class")), id);
    assert_eq!(value(&document, id), "changed");
}

#[test]
fn detached_clones_preserve_names_values_and_independent_mutation() {
    let mut document = BaseDocument::new(Default::default());
    let root = document.root_node().id;
    let qualified = QualName::new(Some("p".into()), "urn:test".into(), "x".into());
    let id = document
        .try_create_attribute_node(root, qualified.clone(), 32)
        .unwrap();
    document
        .try_set_attribute_node_value(id, "before".into())
        .unwrap();
    let copy = document.try_clone_attribute_node(id, root, 32).unwrap();
    assert_ne!(copy, id);
    assert_eq!(document.attribute_node(copy).unwrap().name(), &qualified);
    assert_eq!(document.attribute_node(copy).unwrap().owner_element(), None);
    assert_eq!(value(&document, copy), "before");
    document
        .try_set_attribute_node_value(copy, "after".into())
        .unwrap();
    assert_eq!(value(&document, id), "before");
    assert_eq!(value(&document, copy), "after");
}

#[test]
fn replacement_returns_old_identity_and_preserves_attribute_order() {
    let mut document = BaseDocument::new(Default::default());
    let root = document.root_node().id;
    let owner = element(&mut document);
    document.mutate().set_attribute(owner, name("id"), "i");
    document.mutate().set_attribute(owner, name("class"), "old");
    document.mutate().set_attribute(owner, name("title"), "t");
    let old = ensured(&mut document, owner, &name("class"));
    let new = document
        .try_create_attribute_node(root, name("class"), 32)
        .unwrap();
    document
        .try_set_attribute_node_value(new, "new".into())
        .unwrap();
    assert_eq!(
        document.try_attach_attribute_node(owner, new, 32).unwrap(),
        Some(old)
    );
    assert_eq!(document.attribute_names(owner), ["id", "class", "title"]);
    assert_eq!(document.attribute_node(old).unwrap().owner_element(), None);
    assert_eq!(value(&document, old), "old");
    assert_eq!(value(&document, new), "new");
    assert_eq!(
        document.attribute_node(new).unwrap().owner_element(),
        Some(owner)
    );
    assert_eq!(
        document.try_attach_attribute_node(owner, new, 32).unwrap(),
        Some(new)
    );
}

#[test]
fn prefix_replacement_preserves_position_and_records_one_value_change() {
    let mut document = BaseDocument::new(Default::default());
    let root = document.root_node().id;
    let owner = element(&mut document);
    let old_name = QualName::new(Some("old".into()), "urn:test".into(), "x".into());
    let new_name = QualName::new(Some("new".into()), "urn:test".into(), "x".into());
    document.mutate().set_attribute(owner, name("id"), "i");
    document
        .mutate()
        .set_attribute(owner, old_name.clone(), "before");
    document.mutate().set_attribute(owner, name("title"), "t");
    let old = ensured(&mut document, owner, &old_name);
    let new = document
        .try_create_attribute_node(root, new_name.clone(), 32)
        .unwrap();
    document
        .try_set_attribute_node_value(new, "after".into())
        .unwrap();
    document.set_mutation_recording(true);
    assert_eq!(
        document.try_attach_attribute_node(owner, new, 32).unwrap(),
        Some(old)
    );
    assert_eq!(document.attribute_names(owner), ["id", "new:x", "title"]);
    assert_eq!(document.attribute_node(old).unwrap().name(), &old_name);
    assert_eq!(document.attribute_node(new).unwrap().name(), &new_name);
    assert_eq!(value(&document, old), "before");
    let changes = document.take_mutation_records();
    assert_eq!(changes.len(), 1);
    assert!(
        matches!(&changes[0], MutationRecord::Attributes { target, name, namespace: Some(namespace), old_value: Some(old_value) }
        if *target == owner && name == "x" && namespace == "urn:test" && old_value == "before")
    );
}

#[test]
fn attachment_and_removal_errors_leave_both_elements_unchanged() {
    let mut document = BaseDocument::new(Default::default());
    let first = element(&mut document);
    let second = element(&mut document);
    document
        .mutate()
        .set_attribute(first, name("class"), "first");
    let id = ensured(&mut document, first, &name("class"));
    assert_eq!(
        document.try_attach_attribute_node(second, id, 32),
        Err(AttributeNodeError::InUse)
    );
    assert_eq!(
        document.try_remove_attribute_node(second, id),
        Err(AttributeNodeError::NotFound)
    );
    assert_eq!(
        document.attribute_node(id).unwrap().owner_element(),
        Some(first)
    );
    assert_eq!(value(&document, id), "first");
    assert!(
        document
            .get_node(second)
            .unwrap()
            .element_data()
            .unwrap()
            .attrs()
            .is_empty()
    );
}

#[test]
fn exact_record_limit_reads_existing_identity_and_reclaims_without_aliasing() {
    let mut document = BaseDocument::new(Default::default());
    let root = document.root_node().id;
    let owner = element(&mut document);
    document.mutate().set_attribute(owner, name("id"), "a");
    let id = document
        .try_ensure_attribute_node(owner, &name("id"), 1)
        .unwrap()
        .unwrap();
    assert_eq!(
        document
            .try_ensure_attribute_node(owner, &name("id"), 0)
            .unwrap(),
        Some(id)
    );
    assert_eq!(
        document.try_create_attribute_node(root, name("new"), 1),
        Err(AttributeNodeError::Limit)
    );
    assert_eq!(document.reclaim_attribute_nodes(&[]), [id]);
    assert!(document.attribute_node(id).is_none());
    let fresh = document
        .try_ensure_attribute_node(owner, &name("id"), 1)
        .unwrap()
        .unwrap();
    assert_ne!(fresh, id);
    assert_eq!(value(&document, fresh), "a");
    assert!(document.attribute_node_value(id).is_none());
}

#[test]
fn node_reclamation_prunes_unowned_records_before_tree_id_reuse() {
    let mut document = BaseDocument::new(Default::default());
    let owner = element(&mut document);
    document.mutate().set_attribute(owner, name("id"), "a");
    let id = ensured(&mut document, owner, &name("id"));
    assert!(document.reclaim_detached_nodes(&[]).contains(&owner));
    assert!(document.attribute_node(id).is_none());
    let fresh_owner = element(&mut document);
    document
        .mutate()
        .set_attribute(fresh_owner, name("id"), "b");
    let fresh = ensured(&mut document, fresh_owner, &name("id"));
    assert_ne!(fresh, id);
    assert_eq!(value(&document, fresh), "b");
}

#[test]
fn attributes_follow_attachment_then_keep_their_adopted_inert_document() {
    let mut document = BaseDocument::new(Default::default());
    let root = document.root_node().id;
    let template = document
        .mutate()
        .try_create_element(
            QualName::new(None, ns!(html), "template".into()),
            Vec::new(),
        )
        .unwrap();
    let contents = document
        .mutate()
        .try_ensure_template_contents(template)
        .unwrap();
    let inert = document.node_document(contents);
    let owner = element(&mut document);
    let id = document
        .try_create_attribute_node(inert, name("x"), 32)
        .unwrap();
    document
        .try_set_attribute_node_value(id, "value".into())
        .unwrap();
    assert_eq!(document.attribute_node_document(id), Some(inert));
    assert_eq!(
        document.try_attach_attribute_node(owner, id, 32).unwrap(),
        None
    );
    assert_eq!(document.attribute_node_document(id), Some(root));
    document.try_adopt_attribute_node(id, inert).unwrap();
    assert_eq!(document.attribute_node(id).unwrap().owner_element(), None);
    assert_eq!(document.attribute_node_document(id), Some(inert));
    assert!(document.attribute_by_name(owner, "x").is_none());
    assert_eq!(value(&document, id), "value");
}

#[test]
fn detached_and_attached_values_preserve_every_utf16_unit() {
    let mut document = BaseDocument::new(Default::default());
    let root = document.root_node().id;
    let owner = element(&mut document);
    let id = document
        .try_create_attribute_node(root, name("data-x"), 32)
        .unwrap();
    let units = vec![0xd800, 0x61, 0xdc00];
    document
        .try_set_attribute_node_value(id, DomString::from_utf16(units.clone()))
        .unwrap();
    assert!(value(&document, id).utf16_units().eq(units.iter().copied()));
    document.try_attach_attribute_node(owner, id, 32).unwrap();
    assert!(
        document
            .attribute_by_name(owner, "data-x")
            .unwrap()
            .value
            .utf16_units()
            .eq(units.iter().copied())
    );
    document.try_remove_attribute_node(owner, id).unwrap();
    assert!(value(&document, id).utf16_units().eq(units));
}

#[test]
fn invalid_identities_and_receivers_return_errors_without_tree_mutation() {
    let mut document = BaseDocument::new(Default::default());
    let root = document.root_node().id;
    let owner = element(&mut document);
    let missing = AttributeId::default();
    assert_eq!(
        document.try_create_attribute_node(owner, name("x"), 32),
        Err(AttributeNodeError::InvalidNode)
    );
    assert_eq!(
        document.try_set_attribute_node_value(missing, "x".into()),
        Err(AttributeNodeError::InvalidAttribute)
    );
    assert_eq!(
        document.try_attach_attribute_node(root, missing, 32),
        Err(AttributeNodeError::InvalidNode)
    );
    assert_eq!(document.attribute_node_count(), 0);
    assert_eq!(document.attribute_names(owner), Vec::<String>::new());
}

#[test]
fn replacement_limit_failure_does_not_change_the_element_or_new_attribute() {
    let mut document = BaseDocument::new(Default::default());
    let root = document.root_node().id;
    let owner = element(&mut document);
    document.mutate().set_attribute(owner, name("id"), "old");
    let new = document
        .try_create_attribute_node(root, name("id"), 1)
        .unwrap();
    document
        .try_set_attribute_node_value(new, "new".into())
        .unwrap();
    document.set_mutation_recording(true);
    assert_eq!(
        document.try_attach_attribute_node(owner, new, 1),
        Err(AttributeNodeError::Limit)
    );
    assert_eq!(document.attribute_node_count(), 1);
    assert_eq!(
        document.attribute_by_name(owner, "id").unwrap().value,
        "old"
    );
    assert_eq!(document.attribute_node(new).unwrap().owner_element(), None);
    assert_eq!(value(&document, new), "new");
    assert!(document.take_mutation_records().is_empty());
    assert!(
        document
            .try_attach_attribute_node(owner, new, 2)
            .unwrap()
            .is_some()
    );
    assert_eq!(
        document.attribute_by_name(owner, "id").unwrap().value,
        "new"
    );
}

#[test]
fn native_duplicate_names_fail_before_allocating_or_recording_mutations() {
    let mut document = BaseDocument::new(Default::default());
    let root = document.root_node().id;
    let owner = element(&mut document);
    let first = QualName::new(Some("a".into()), "urn:test".into(), "x".into());
    let second = QualName::new(Some("b".into()), "urn:test".into(), "x".into());
    document.mutate().set_attribute(owner, first.clone(), "one");
    document.mutate().set_attribute(owner, second, "two");
    let new = document.try_create_attribute_node(root, first, 32).unwrap();
    document.set_mutation_recording(true);
    assert_eq!(
        document.try_attach_attribute_node(owner, new, 32),
        Err(AttributeNodeError::InvalidAttribute)
    );
    assert_eq!(document.attribute_node_count(), 1);
    assert_eq!(document.attribute_names(owner), ["a:x", "b:x"]);
    assert_eq!(document.attribute_node(new).unwrap().owner_element(), None);
    assert!(document.take_mutation_records().is_empty());
}
