use super::{Boundary, RangeMutation};
use crate::{BaseDocument, DocumentConfig, NodeId, tree_notifications::TreeObserver};
use std::{cell::RefCell, rc::Rc};

#[derive(Default)]
struct Points(RefCell<Vec<Boundary>>);
impl TreeObserver for Points {
    fn removing(&self, doc: &BaseDocument, node: NodeId) {
        for point in self.0.borrow_mut().iter_mut() {
            *point = point.removing(doc, node);
        }
    }
    fn inserted(&self, _: &BaseDocument, _: NodeId) {}
    fn range_mutation(&self, doc: &BaseDocument, mutation: RangeMutation) {
        for point in self.0.borrow_mut().iter_mut() {
            *point = point.changed(doc, mutation);
        }
    }
}
fn setup() -> (BaseDocument, Rc<Points>, NodeId, NodeId) {
    let mut doc = BaseDocument::new(DocumentConfig::default());
    let points = Rc::new(Points::default());
    doc.set_tree_observer(points.clone());
    let mut m = doc.mutate();
    let parent = m.create_document_fragment();
    let text = m.create_text_node("abcdef");
    m.append_children(parent, &[text]);
    drop(m);
    (doc, points, parent, text)
}
fn point(node: NodeId, offset: usize) -> Boundary {
    Boundary { node, offset }
}

#[test]
fn replacement_clamps_only_points_in_the_replaced_interval() {
    let (mut doc, points, _, text) = setup();
    *points.0.borrow_mut() = (0..=6).map(|i| point(text, i)).collect();
    doc.mutate()
        .replace_character_data(text, 2, 2, &"XYZ".into())
        .unwrap();
    assert_eq!(
        *points.0.borrow(),
        [0, 1, 2, 2, 2, 6, 7].map(|i| point(text, i))
    );
    assert_eq!(doc.character_data(text), Some("abXYZef"));
    doc.mutate().set_node_text(text, "abXYZef");
    assert_eq!(*points.0.borrow(), vec![point(text, 0); 7]);
}

#[test]
fn split_updates_text_and_the_boundary_after_it_then_normalize_restores_them() {
    let (mut doc, points, parent, text) = setup();
    *points.0.borrow_mut() = vec![
        point(text, 2),
        point(text, 3),
        point(text, 6),
        point(parent, 1),
    ];
    let tail = doc.mutate().split_text(text, 2).unwrap();
    assert_eq!(
        *points.0.borrow(),
        vec![
            point(text, 2),
            point(tail, 1),
            point(tail, 4),
            point(parent, 2)
        ]
    );
    points.0.borrow_mut().push(point(parent, 1));
    doc.mutate().normalize_subtree(parent);
    assert_eq!(doc.character_data(text), Some("abcdef"));
    assert_eq!(
        *points.0.borrow(),
        vec![
            point(text, 2),
            point(text, 3),
            point(text, 6),
            point(parent, 1),
            point(text, 2)
        ]
    );
}

#[test]
fn detached_split_clamps_instead_of_moving_to_the_new_text() {
    let (mut doc, points, _, text) = setup();
    doc.mutate().remove_node(text);
    *points.0.borrow_mut() = vec![point(text, 1), point(text, 4)];
    doc.mutate().split_text(text, 2).unwrap();
    assert_eq!(*points.0.borrow(), vec![point(text, 1), point(text, 2)]);
}

#[test]
fn insertion_and_subtree_removal_preserve_boundary_sides() {
    let (mut doc, points, parent, text) = setup();
    *points.0.borrow_mut() = vec![point(parent, 0), point(parent, 1), point(text, 3)];
    let mut m = doc.mutate();
    let a = m.create_text_node("a");
    let b = m.create_text_node("b");
    m.insert_nodes_before(text, &[a, b]);
    drop(m);
    assert_eq!(
        *points.0.borrow(),
        vec![point(parent, 0), point(parent, 3), point(text, 3)]
    );
    doc.mutate().remove_node(text);
    assert_eq!(
        *points.0.borrow(),
        vec![point(parent, 0), point(parent, 2), point(parent, 2)]
    );
}

#[test]
fn invalid_offsets_and_node_admission_leave_text_and_boundaries_unchanged() {
    let (mut doc, points, _, text) = setup();
    *points.0.borrow_mut() = vec![point(text, 4)];
    assert!(
        doc.mutate()
            .replace_character_data(text, 7, 0, &"x".into())
            .is_err()
    );
    assert!(doc.mutate().split_text(text, 7).is_err());
    assert_eq!(doc.character_data(text), Some("abcdef"));
    assert_eq!(*points.0.borrow(), vec![point(text, 4)]);
    doc.mutate()
        .replace_character_data(text, 2, usize::MAX, &"x".into())
        .unwrap();
    assert_eq!(doc.character_data(text), Some("abx"));
    assert_eq!(*points.0.borrow(), vec![point(text, 2)]);
}

#[test]
fn split_offsets_are_utf16_even_inside_a_surrogate_pair() {
    let (mut doc, points, parent, text) = setup();
    doc.mutate().set_character_data(
        text,
        crate::DomString::from_utf16(vec![65, 0xd83d, 0xde00, 0xd800]),
    );
    *points.0.borrow_mut() = vec![point(text, 2), point(text, 4)];
    let tail = doc.mutate().split_text(text, 2).unwrap();
    assert_eq!(
        doc.character_data_dom(text).unwrap().to_utf16().as_ref(),
        &[65, 0xd83d]
    );
    assert_eq!(
        doc.character_data_dom(tail).unwrap().to_utf16().as_ref(),
        &[0xde00, 0xd800]
    );
    doc.mutate().normalize_subtree(parent);
    assert_eq!(*points.0.borrow(), vec![point(text, 2), point(text, 4)]);
    assert_eq!(
        doc.character_data_dom(text).unwrap().to_utf16().as_ref(),
        &[65, 0xd83d, 0xde00, 0xd800]
    );
}

#[test]
fn split_admission_failure_is_atomic() {
    let mut doc = BaseDocument::new(DocumentConfig {
        node_limit: Some(3),
        ..DocumentConfig::default()
    });
    let text = doc.mutate().try_create_text_node("abcd").unwrap();
    let points = Rc::new(Points::default());
    *points.0.borrow_mut() = vec![point(text, 3)];
    doc.set_tree_observer(points.clone());
    assert_eq!(
        doc.mutate().split_text(text, 2),
        Err(crate::dom_api::DomError::QuotaExceeded)
    );
    assert_eq!(doc.character_data(text), Some("abcd"));
    assert_eq!(*points.0.borrow(), vec![point(text, 3)]);
    assert_eq!(doc.node_count(), 3);
}

#[test]
fn normalize_appends_once_before_removing_adjacent_text_nodes() {
    use crate::mutations::MutationRecord;
    let (mut doc, points, parent, text) = setup();
    let mut m = doc.mutate();
    let empty = m.create_text_node("");
    let last = m.create_text_node("tail");
    m.append_children(parent, &[empty, last]);
    drop(m);
    *points.0.borrow_mut() = vec![
        point(empty, 0),
        point(last, 2),
        point(parent, 1),
        point(parent, 2),
    ];
    doc.set_mutation_recording(true);
    doc.mutate().normalize_subtree(parent);
    assert_eq!(
        *points.0.borrow(),
        vec![
            point(text, 6),
            point(text, 8),
            point(text, 6),
            point(text, 6)
        ]
    );
    let records = doc.take_mutation_records();
    assert_eq!(records.len(), 3);
    assert!(
        matches!(&records[0],MutationRecord::CharacterData {target, old_value} if *target==text && old_value.as_str_lossy()=="abcdef")
    );
    assert!(matches!(&records[1],MutationRecord::ChildList {removed,..} if removed==&vec![empty]));
    assert!(matches!(&records[2],MutationRecord::ChildList {removed,..} if removed==&vec![last]));
}

#[test]
fn rendered_selection_preserves_boundary_spaces_and_text_overlap() {
    let mut doc = BaseDocument::new(DocumentConfig::default());
    let root = doc.root_node().id;
    let mut m = doc.mutate();
    let html = m.create_element(
        crate::QualName::new(None, crate::ns!(html), "html".into()),
        vec![],
    );
    let body = m.create_element(
        crate::QualName::new(None, crate::ns!(html), "body".into()),
        vec![],
    );
    let p = m.create_element(
        crate::QualName::new(None, crate::ns!(html), "p".into()),
        vec![],
    );
    let text = m.create_text_node("a b");
    m.append_children(root, &[html]);
    m.append_children(html, &[body]);
    m.append_children(body, &[p]);
    m.append_children(p, &[text]);
    drop(m);
    doc.resolve(0.0);
    for (start, end, expected) in [(1, 2, " "), (0, 2, "a "), (1, 3, " b")] {
        assert_eq!(
            doc.dom_selection_text(point(text, start), point(text, end))
                .as_str_lossy(),
            expected
        );
        assert!(doc.dom_selection_contains(point(text, start), point(text, end), text, false));
    }
    assert!(!doc.dom_selection_contains(point(p, 0), point(p, 1), p, false));
    assert!(doc.dom_selection_contains(point(p, 0), point(p, 1), p, true));
}
