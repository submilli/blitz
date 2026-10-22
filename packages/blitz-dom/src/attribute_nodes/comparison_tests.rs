use super::{AttributeId, NodeIdentity};
use crate::{BaseDocument, NodeId, QualName, ns};

fn element(document: &mut BaseDocument) -> NodeId {
    document
        .mutate()
        .try_create_element(QualName::new(None, ns!(html), "div".into()), Vec::new())
        .unwrap()
}
fn attribute(document: &mut BaseDocument, owner: NodeId, name: &str) -> AttributeId {
    let name = QualName::new(None, ns!(), name.into());
    document
        .mutate()
        .set_attribute(owner, name.clone(), "value");
    document
        .try_ensure_attribute_node(owner, &name, 32)
        .unwrap()
        .unwrap()
}

#[test]
fn attribute_position_uses_owner_order_without_becoming_a_tree_child() {
    use NodeIdentity::{Attribute as A, Tree as T};
    let mut document = BaseDocument::new(Default::default());
    let root = document.root_node().id;
    let owner = element(&mut document);
    let child = element(&mut document);
    document.mutate().pre_insert(owner, root, None).unwrap();
    document.mutate().pre_insert(child, owner, None).unwrap();
    let a = attribute(&mut document, owner, "id");
    let b = attribute(&mut document, owner, "class");
    for (left, right, expected) in [
        (A(a), A(a), 0),
        (A(a), A(b), 36),
        (A(b), A(a), 34),
        (T(owner), A(a), 20),
        (A(a), T(owner), 10),
        (T(root), A(a), 20),
        (A(a), T(root), 10),
        (A(a), T(child), 4),
        (T(child), A(a), 2),
    ] {
        assert_eq!(document.compare_node_positions(left, right), Some(expected));
    }
}

#[test]
fn disconnected_attributes_have_stable_antisymmetric_versioned_order() {
    use NodeIdentity::{Attribute as A, Tree as T};
    let mut document = BaseDocument::new(Default::default());
    let root = document.root_node().id;
    let name = QualName::new(None, ns!(), "x".into());
    let a = document
        .try_create_attribute_node(root, name.clone(), 32)
        .unwrap();
    let b = document.try_clone_attribute_node(a, root, 32).unwrap();
    let ab = document.compare_node_positions(A(a), A(b)).unwrap();
    let ba = document.compare_node_positions(A(b), A(a)).unwrap();
    assert_eq!(ab & 33, 33);
    assert_eq!(ba & 33, 33);
    assert_ne!(ab & 6, ba & 6);
    assert_eq!(document.compare_node_positions(A(a), A(b)), Some(ab));
    assert_eq!(
        document.compare_node_positions(A(a), T(root)).unwrap() & 33,
        33
    );
    assert_eq!(
        document.compare_node_positions(T(root), A(a)).unwrap() & 6,
        4
    );
    document.reclaim_attribute_nodes(&[]);
    let fresh = document.try_create_attribute_node(root, name, 32).unwrap();
    assert_ne!(fresh, a);
    assert_eq!(document.compare_node_positions(A(a), A(a)), None);
    assert_eq!(document.compare_node_positions(A(fresh), A(fresh)), Some(0));
}

#[test]
fn ordinary_tree_positions_keep_ancestor_sibling_and_disconnected_flags() {
    use NodeIdentity::Tree as T;
    let mut document = BaseDocument::new(Default::default());
    let root = document.root_node().id;
    let owner = element(&mut document);
    let a = element(&mut document);
    let b = element(&mut document);
    let detached = element(&mut document);
    document.mutate().pre_insert(owner, root, None).unwrap();
    document.mutate().pre_insert(a, owner, None).unwrap();
    document.mutate().pre_insert(b, owner, None).unwrap();
    for (left, right, expected) in [
        (root, a, 20),
        (a, root, 10),
        (a, b, 4),
        (b, a, 2),
        (a, a, 0),
    ] {
        assert_eq!(
            document.compare_node_positions(T(left), T(right)),
            Some(expected)
        );
    }
    assert_eq!(
        document.compare_node_positions(T(a), T(detached)).unwrap() & 33,
        33
    );
    assert_eq!(
        document.compare_node_positions(T(NodeId::from_u64(u64::MAX)), T(a)),
        None
    );
}

#[test]
fn detached_owner_and_removed_attributes_retain_independent_position_roots() {
    use NodeIdentity::{Attribute as A, Tree as T};
    let mut document = BaseDocument::new(Default::default());
    let owner = element(&mut document);
    let a = attribute(&mut document, owner, "id");
    assert_eq!(document.compare_node_positions(T(owner), A(a)), Some(20));
    document.try_remove_attribute_node(owner, a).unwrap();
    assert_eq!(
        document.compare_node_positions(T(owner), A(a)).unwrap() & 33,
        33
    );
    assert_eq!(
        document.compare_node_positions(A(a), T(owner)).unwrap() & 6,
        2
    );
    document.reclaim_detached_nodes(&[]);
    assert_eq!(document.compare_node_positions(T(owner), A(a)), None);
    assert_eq!(document.compare_node_positions(A(a), A(a)), Some(0));
}

#[test]
fn destructive_native_teardown_invalidates_attached_attribute_positions() {
    use NodeIdentity::{Attribute as A, Tree as T};
    let mut document = BaseDocument::new(Default::default());
    let root = document.root_node().id;
    let owner = element(&mut document);
    let id = attribute(&mut document, owner, "x");
    assert_eq!(document.compare_node_positions(A(id), T(owner)), Some(10));
    document.mutate().remove_and_drop_node(owner);
    assert_eq!(document.compare_node_positions(A(id), A(id)), None);
    assert_eq!(document.compare_node_positions(A(id), T(root)), None);
    let fresh_owner = element(&mut document);
    assert_ne!(fresh_owner, owner);
    assert_eq!(document.compare_node_positions(A(id), T(fresh_owner)), None);
}
