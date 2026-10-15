use super::*;
use crate::{DocumentConfig, QualName, ns};

#[test]
fn deep_object_ancestry_is_iterative_memoized_and_budgeted() {
    let mut doc = BaseDocument::new(DocumentConfig::default());
    let mut parent = doc.root_node().id;
    // No styles/layout are needed: the parent-render predicate must still
    // inspect the entire chain before deciding these nodes have no object.
    for _ in 0..40_000 {
        let id = doc
            .mutate()
            .create_element(QualName::new(None, ns!(html), local_name!("div")), vec![]);
        doc.nodes[id].parent = Some(parent);
        doc.nodes[parent].children.push(id);
        parent = id;
    }
    let cx = Context::new(&doc);
    assert!(cx.object(parent).is_none());
    assert_eq!(cx.objects.borrow().len(), 40_001);
    let remaining = cx.budget.get();
    assert!(cx.object(parent).is_none());
    assert_eq!(
        cx.budget.get(),
        remaining,
        "memoized queries do not walk again"
    );

    let limited = Context::with_tree(Tree::new(&doc), Rc::new(Cell::new(3)), None);
    assert!(limited.object(parent).is_none());
    assert!(limited.exhausted());
    assert!(
        limited.objects.borrow().is_empty(),
        "exhaustion cannot memoize incomplete results"
    );
}
