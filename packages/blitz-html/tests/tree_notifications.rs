//! Every mutator path presents the old topology to the pre-removal observer.
mod common;

use blitz_dom::{BaseDocument, NodeId, tree_notifications::TreeObserver};
use common::{parse, q};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

struct Observer {
    root: NodeId,
    reference: Cell<(NodeId, bool)>,
    removed: RefCell<Vec<NodeId>>,
    inserted: RefCell<Vec<NodeId>>,
}

impl TreeObserver for Observer {
    fn removing(&self, doc: &BaseDocument, node: NodeId) {
        let parent = doc.get_node(node).unwrap().parent.unwrap();
        assert!(doc.get_node(parent).unwrap().children.contains(&node));
        for &sibling in doc.get_node(parent).unwrap().children.iter() {
            assert_eq!(doc.get_node(sibling).unwrap().parent, Some(parent));
        }
        let (reference, before) = self.reference.get();
        if let Some(position) = doc.iterator_pre_removal(self.root, reference, before, node) {
            self.reference.set(position);
        }
        self.removed.borrow_mut().push(node);
    }
    fn inserted(&self, doc: &BaseDocument, node: NodeId) {
        assert!(doc.get_node(node).unwrap().parent.is_some());
        self.inserted.borrow_mut().push(node);
    }
}

const PAGE: &str =
    "<div id=r><p id=a><b id=b></b></p><p id=c></p><p id=d></p></div><aside id=other></aside>";

fn observe(doc: &mut BaseDocument, before: bool) -> Rc<Observer> {
    let observer = Rc::new(Observer {
        root: q(doc, "#r"),
        reference: Cell::new((q(doc, "#b"), before)),
        removed: RefCell::default(),
        inserted: RefCell::default(),
    });
    doc.set_tree_observer(observer.clone());
    observer
}

#[test]
fn removal_paths_keep_topology_and_adjust_each_bulk_child() {
    for operation in 0..6 {
        let mut doc = parse(PAGE);
        let observer = observe(&mut doc, true);
        let root = q(&doc, "#r");
        let a = q(&doc, "#a");
        let other = q(&doc, "#other");
        let c = q(&doc, "#c");
        let d = q(&doc, "#d");
        let mut m = doc.mutate();
        match operation {
            0 => m.remove_node(a),
            1 => m.set_text_content(root, "replacement"),
            2 => m.reparent_children(root, other),
            3 => m.append_children(other, &[a, c, d]),
            4 => m.remove_and_drop_all_children(root),
            5 => {
                m.remove_and_drop_node(a);
            }
            _ => unreachable!(),
        }
        drop(m);
        assert_eq!(observer.removed.borrow()[0], a);
        if matches!(operation, 0 | 5) {
            assert_eq!(observer.reference.get(), (c, true));
        } else {
            assert_eq!(&*observer.removed.borrow(), &[a, c, d]);
            assert_eq!(observer.reference.get(), (root, false));
        }
    }
}

#[test]
fn removing_a_root_or_its_ancestor_preserves_the_iterator_position() {
    let mut doc = parse(PAGE);
    let observer = observe(&mut doc, false);
    let reference = observer.reference.get();
    let root = q(&doc, "#r");
    doc.mutate().remove_node(root);
    assert_eq!(observer.reference.get(), reference);
}

#[test]
fn iterator_removal_notifications_do_not_change_observer_records() {
    let mut doc = parse(PAGE);
    let observer = observe(&mut doc, false);
    doc.set_mutation_recording(true);
    let root = q(&doc, "#r");
    doc.mutate().set_text_content(root, "new");
    assert_eq!(observer.removed.borrow().len(), 3);
    let records = doc.take_mutation_records();
    assert_eq!(records.len(), 1);
    let blitz_dom::mutations::MutationRecord::ChildList { removed, added, .. } = &records[0] else {
        panic!("child list")
    };
    assert_eq!(removed.len(), 3);
    assert_eq!(added.len(), 1);
}
