//! Wide DOM regressions for indexed child storage. Run optimized for timing.
use blitz_dom::mutations::MutationRecord;
use blitz_dom::{BaseDocument, NodeId};
use std::time::{Duration, Instant};

fn children(count: usize) -> (BaseDocument, NodeId, Vec<NodeId>) {
    let mut doc = BaseDocument::new(Default::default());
    let mut m = doc.mutate();
    let parent = m.create_document_fragment();
    let ids: Vec<_> = (0..count).map(|_| m.create_text_node("x")).collect();
    m.append_children(parent, &ids);
    drop(m);
    (doc, parent, ids)
}

fn measured(label: &str, action: impl FnOnce()) {
    let start = Instant::now();
    action();
    let elapsed = start.elapsed();
    eprintln!("{label}: {elapsed:?}");
    // Optimized historic scans took seconds; leave debug/instrumented runs
    // unconstrained while keeping all semantic assertions enabled.
    if !cfg!(debug_assertions) {
        assert!(elapsed < Duration::from_millis(500), "{label}: {elapsed:?}");
    }
}

#[test]
fn hundred_thousand_sibling_walks_and_single_removals() {
    let (mut doc, parent, ids) = children(100_000);
    measured("100k forward/backward/index", || {
        for (index, &id) in ids.iter().enumerate() {
            let node = doc.get_node(id).unwrap();
            assert_eq!(node.child_index(), Some(index));
            assert_eq!(node.forward(1).map(|n| n.id), ids.get(index + 1).copied());
            assert_eq!(
                node.backward(1).map(|n| n.id),
                index.checked_sub(1).map(|i| ids[i])
            );
            assert!(node.forward(usize::MAX).is_none());
        }
    });
    measured("100k front removals", || {
        let mut m = doc.mutate();
        for &id in &ids {
            m.remove_child(parent, id).unwrap();
        }
    });
    assert!(doc.get_node(parent).unwrap().children.is_empty());
    assert!(
        ids.iter()
            .all(|&id| doc.get_node(id).unwrap().child_index().is_none())
    );
}

#[test]
fn fifty_thousand_adjacent_text_removals() {
    let (mut doc, parent, ids) = children(50_000);
    measured("50k normalize mutator sequence", || {
        let data: String = ids
            .iter()
            .map(|&id| doc.character_data(id).unwrap())
            .collect();
        let mut m = doc.mutate();
        m.set_character_data(ids[0], &data);
        for &id in &ids[1..] {
            m.remove_child(parent, id).unwrap();
        }
    });
    assert_eq!(doc.get_node(parent).unwrap().children.as_slice(), &ids[..1]);
    assert_eq!(doc.character_data(ids[0]).unwrap().len(), 50_000);
}

#[test]
fn hundred_thousand_recorded_fragment_children() {
    let (mut doc, fragment, ids) = children(100_000);
    let destination = doc.mutate().create_document_fragment();
    doc.set_mutation_recording(true);
    measured("100k recorded fragment append", || {
        doc.mutate()
            .pre_insert(fragment, destination, None)
            .unwrap();
    });
    assert!(doc.get_node(fragment).unwrap().children.is_empty());
    assert_eq!(doc.get_node(destination).unwrap().children.as_slice(), ids);
    // The existing bounded log fails closed; indexing must stay linear even
    // when recording is enabled after that cap has been reached.
    assert!(doc.mutation_recording_overflowed());
}

#[test]
fn recorded_moves_preserve_each_removal_neighbors() {
    let (mut doc, fragment, ids) = children(7);
    let destination = doc.mutate().create_document_fragment();
    doc.set_mutation_recording(true);
    doc.mutate()
        .append_children(destination, &[ids[3], ids[1], ids[5]]);
    let records = doc.take_mutation_records();
    let neighbors = [(3, 2, 4), (1, 0, 2), (5, 4, 6)];
    for (record, (removed, prev, next)) in records.iter().zip(neighbors) {
        assert_eq!(
            *record,
            MutationRecord::ChildList {
                target: fragment,
                added: vec![],
                removed: vec![ids[removed]],
                previous_sibling: Some(ids[prev]),
                next_sibling: Some(ids[next]),
            }
        );
    }
    assert_eq!(records.len(), 4);
    assert_eq!(
        doc.get_node(fragment).unwrap().children.as_slice(),
        [ids[0], ids[2], ids[4], ids[6]]
    );
    assert_eq!(
        doc.get_node(destination).unwrap().children.as_slice(),
        [ids[3], ids[1], ids[5]]
    );
}
