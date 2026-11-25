use super::*;
use crate::{DocumentConfig, QualName};
fn element(doc: &mut BaseDocument, tag: &str) -> NodeId {
    doc.mutate()
        .create_element(QualName::new(None, crate::ns!(html), tag.into()), vec![])
}
fn shadow(doc: &mut BaseDocument, manual: bool) -> (NodeId, NodeId, NodeId) {
    let host = element(doc, "div");
    let root = doc
        .mutate()
        .attach_shadow(
            host,
            ShadowRootInit {
                open: true,
                manual_slots: manual,
                clonable: true,
                serializable: true,
                ..Default::default()
            },
        )
        .unwrap();
    let slot = element(doc, "slot");
    doc.mutate().append_children(root, &[slot]);
    (host, root, slot)
}
#[test]
fn manual_assignment_is_ordered_unique_and_reassigned() {
    let mut doc = BaseDocument::new(Default::default());
    let (host, root, slot) = shadow(&mut doc, true);
    let other = element(&mut doc, "slot");
    doc.mutate().append_children(root, &[other]);
    let a = element(&mut doc, "span");
    let b = doc.mutate().create_text_node("text");
    doc.mutate().append_children(host, &[a, b]);
    assert!(doc.assigned_nodes(slot).is_empty());
    doc.mutate().assign_slot(slot, &[b, a, b]);
    assert_eq!(doc.assigned_nodes(slot), vec![b, a]);
    assert_eq!(doc.assigned_slot(a), Some(slot));
    doc.mutate().assign_slot(other, &[a]);
    assert_eq!(doc.assigned_nodes(slot), vec![b]);
    assert_eq!(doc.assigned_slot(a), Some(other));
    doc.mutate().remove_node(a);
    assert!(doc.assigned_nodes(other).is_empty());
    doc.mutate().append_children(host, &[a]);
    assert_eq!(doc.assigned_nodes(other), vec![a]);
}
#[test]
fn slot_changes_coalesce_without_mutation_observers() {
    let mut doc = BaseDocument::new(Default::default());
    let (host, root, slot) = shadow(&mut doc, false);
    let a = element(&mut doc, "span");
    doc.mutate().append_children(host, &[a]);
    doc.mutate().remove_node(a);
    doc.mutate().append_children(host, &[a]);
    assert_eq!(doc.take_slot_changes(), vec![slot]);
    doc.mutate().append_children(host, &[a]);
    assert_eq!(doc.take_slot_changes(), vec![slot]);
    doc.mutate().remove_node(slot);
    assert_eq!(doc.take_slot_changes(), vec![slot]);
    doc.mutate().append_children(root, &[slot]);
    assert_eq!(doc.take_slot_changes(), vec![slot]);
}
#[test]
fn manual_fallback_and_signal_nodes_are_retained_until_delivery() {
    let mut doc = BaseDocument::new(Default::default());
    let (host, _, slot) = shadow(&mut doc, true);
    let text = doc.mutate().create_text_node("fallback");
    doc.mutate().append_children(slot, &[text]);
    assert_eq!(doc.flattened_assigned_nodes(slot, 10).unwrap(), vec![text]);
    assert_eq!(doc.take_slot_changes(), vec![slot]);
    let foreign = element(&mut doc, "span");
    doc.mutate().assign_slot(slot, &[foreign]);
    assert!(doc.assigned_nodes(slot).is_empty());
    doc.mutate().append_children(host, &[foreign]);
    assert_eq!(doc.take_slot_changes(), vec![slot]);
    doc.mutate().assign_slot(slot, &[]);
    doc.reclaim_detached_nodes(&[]);
    assert!(doc.get_node(slot).is_some());
    doc.take_slot_changes();
    doc.reclaim_detached_nodes(&[]);
    assert!(doc.get_node(slot).is_none());
}
#[test]
fn shallow_clone_copies_shadow_subtree_and_metadata_but_not_assignments() {
    let mut doc = BaseDocument::new(Default::default());
    let (host, _, slot) = shadow(&mut doc, true);
    let text = doc.mutate().create_text_node("light");
    doc.mutate().append_children(host, &[text]);
    doc.mutate().assign_slot(slot, &[text]);
    let clone = doc.mutate().try_clone_node(host, false).unwrap();
    let root = doc.shadow_root_of(clone).unwrap();
    assert!(doc.nodes[clone].children.is_empty());
    assert_eq!(doc.nodes[root].children.len(), 1);
    assert!(
        doc.nodes[root]
            .shadow_root_data
            .as_ref()
            .unwrap()
            .manual_slots
    );
    assert!(doc.assigned_nodes(doc.nodes[root].children[0]).is_empty());
    assert_eq!(
        doc.get_html(clone, true, &[]).as_str_lossy(),
        "<template shadowrootmode=\"open\" shadowrootserializable=\"\" shadowrootslotassignment=\"manual\" shadowrootclonable=\"\"><slot></slot></template>"
    );
    assert!(doc.get_html(clone, false, &[]).is_empty());
}
#[test]
fn clone_budget_counts_shadow_descendants_even_for_shallow_clone() {
    let mut doc = BaseDocument::new(DocumentConfig {
        node_limit: Some(7),
        ..Default::default()
    });
    let (host, _, _) = shadow(&mut doc, false);
    let before = doc.node_count();
    assert_eq!(
        doc.mutate().try_clone_node(host, false),
        Err(crate::NodeBudgetExceeded)
    );
    assert_eq!(doc.node_count(), before);
}

#[test]
fn flatten_preserves_a_light_tree_slot_and_manual_references_are_weak() {
    let mut doc = BaseDocument::new(Default::default());
    let (host, _, slot) = shadow(&mut doc, false);
    let light = element(&mut doc, "slot");
    let fallback = doc.mutate().create_text_node("light fallback");
    doc.mutate().append_children(light, &[fallback]);
    doc.mutate().append_children(host, &[light]);
    assert_eq!(doc.flattened_assigned_nodes(slot, 10).unwrap(), vec![light]);
    assert!(doc.flattened_assigned_nodes(light, 10).unwrap().is_empty());
    doc.take_slot_changes();
    let foreign = element(&mut doc, "span");
    doc.mutate().assign_slot(slot, &[foreign]);
    doc.reclaim_detached_nodes(&[host]);
    assert!(doc.get_node(foreign).is_none());
    assert_eq!(doc.assigned_nodes(slot), vec![light]);
}
#[test]
fn delegated_focus_obeys_shadow_ancestors_and_retargets_active_element() {
    let mut doc = BaseDocument::new(Default::default());
    let host = element(&mut doc, "div");
    let document = doc.root_node().id;
    doc.mutate().append_children(document, &[host]);
    let root = doc
        .mutate()
        .attach_shadow(
            host,
            ShadowRootInit {
                open: true,
                delegates_focus: true,
                ..Default::default()
            },
        )
        .unwrap();
    let button = element(&mut doc, "button");
    doc.mutate().append_children(root, &[button]);
    assert_eq!(doc.delegated_focus_target(host), Some(button));
    doc.set_focus_to(button);
    assert_eq!(doc.active_element_in(document), Some(host));
    assert_eq!(doc.active_element_in(root), Some(button));
    doc.mutate()
        .set_attribute(host, QualName::new(None, crate::ns!(), "inert".into()), "");
    assert_eq!(doc.delegated_focus_target(host), None);
}

#[test]
fn provisional_split_signals_only_committed_or_independent_slot_changes() {
    let mut doc = BaseDocument::new(Default::default());
    let (host, _, slot) = shadow(&mut doc, false);
    let prefix = doc.mutate().create_text_node("ab");
    doc.mutate().append_children(host, &[prefix]);
    assert_eq!(doc.take_slot_changes(), [slot]);
    for independent in [false, true] {
        let split = doc.split_text_for_insertion(prefix, 1).unwrap();
        assert!(doc.assigned_nodes(slot).contains(&split.suffix()));
        if independent {
            let sibling = element(&mut doc, "i");
            doc.mutate().append_children(host, &[sibling]);
        }
        assert!(doc.finish_insertion_split(split, true));
        assert_eq!(
            doc.take_slot_changes(),
            if independent { vec![slot] } else { vec![] }
        );
    }
    let split = doc.split_text_for_insertion(prefix, 1).unwrap();
    assert!(!doc.finish_insertion_split(split, false));
    assert_eq!(doc.take_slot_changes(), [slot]);
}

#[test]
fn committing_split_skips_a_destroyed_slot() {
    let mut doc = BaseDocument::new(Default::default());
    let (host, _, slot) = shadow(&mut doc, false);
    let prefix = doc.mutate().create_text_node("ab");
    doc.mutate().append_children(host, &[prefix]);
    doc.take_slot_changes();
    let split = doc.split_text_for_insertion(prefix, 1).unwrap();
    doc.mutate().remove_and_drop_node(slot);
    assert!(!doc.finish_insertion_split(split, false));
    assert!(!doc.take_slot_changes().contains(&slot));
}

#[test]
fn provisional_slot_signals_keep_their_position_through_teardown_and_blur() {
    for refuse in [true, false] {
        let mut doc = BaseDocument::new(Default::default());
        let (a, _, slot_a) = shadow(&mut doc, false);
        let (b, _, slot_b) = shadow(&mut doc, false);
        let (c, _, slot_c) = shadow(&mut doc, false);
        let prefix = doc.mutate().create_text_node("ab");
        doc.mutate().append_children(b, &[prefix]);
        doc.take_slot_changes();
        let first = element(&mut doc, "i");
        doc.mutate().append_children(a, &[first]);
        let split = doc.split_text_for_insertion(prefix, 1).unwrap();
        doc.mutate().remove_and_drop_node(slot_a);
        assert!(doc.has_slot_changes());
        let later = element(&mut doc, "i");
        doc.mutate().append_children(c, &[later]);
        let independent = element(&mut doc, "i");
        doc.mutate().append_children(b, &[independent]);
        assert_eq!(doc.finish_insertion_split(split, refuse), refuse);
        assert_eq!(
            doc.take_slot_changes(),
            if refuse {
                [slot_c, slot_b]
            } else {
                [slot_b, slot_c]
            }
        );
    }
}

#[test]
fn transferred_split_signals_follow_the_original_split_outcome() {
    for refused in [false, true] {
        let mut source = BaseDocument::new(Default::default());
        let (host, _, slot) = shadow(&mut source, false);
        let prefix = source.mutate().create_text_node("ab");
        source.mutate().append_children(host, &[prefix]);
        source.take_slot_changes();

        let mut target = BaseDocument::new(Default::default());
        let (first_host, _, first) = shadow(&mut target, false);
        let (_, _, moved) = shadow(&mut target, false);
        let (last_host, _, last) = shadow(&mut target, false);
        let child = element(&mut target, "b");
        target.mutate().append_children(first_host, &[child]);
        let split = source.split_text_for_insertion(prefix, 1).unwrap();
        let child = element(&mut target, "b");
        target.mutate().append_children(last_host, &[child]);
        source.transfer_slot_changes(&mut target, &[(slot, moved)].into());
        let fallback = target.mutate().create_text_node("fallback");
        target.mutate().append_children(moved, &[fallback]);

        assert_eq!(source.finish_insertion_split(split, refused), refused);
        assert!(source.take_slot_changes().is_empty());
        assert_eq!(
            target.take_slot_changes(),
            if refused {
                [first, last, moved]
            } else {
                [first, moved, last]
            }
        );
    }
}

#[test]
fn retired_slot_drops_local_edits_but_preserves_an_incoming_signal_on_return() {
    let mut source = BaseDocument::new(Default::default());
    let (_, _, slot) = shadow(&mut source, false);
    let mut retired = BaseDocument::new(Default::default());
    let (_, _, parked) = shadow(&mut retired, false);
    let fallback = retired.mutate().create_text_node("retired");
    retired.mutate().append_children(parked, &[fallback]);
    retired.retire_slot_changes();
    assert!(retired.take_slot_changes().is_empty());
    retired.mutate().remove_node(fallback);
    assert!(retired.take_slot_changes().is_empty());

    let live = source.mutate().create_text_node("live");
    source.mutate().append_children(slot, &[live]);
    source.transfer_slot_changes(&mut retired, &[(slot, parked)].into());
    retired.retire_slot_changes();
    retired.transfer_slot_changes(&mut source, &[(parked, slot)].into());
    assert_eq!(source.take_slot_changes(), [slot]);
    assert!(retired.take_slot_changes().is_empty());
}

#[test]
fn detached_manual_members_signal_in_tree_order_and_do_not_survive_collection() {
    let mut doc = BaseDocument::new(Default::default());
    let (host, root, first) = shadow(&mut doc, true);
    let second = element(&mut doc, "slot");
    doc.mutate().append_children(root, &[second]);
    let a = element(&mut doc, "i");
    let b = element(&mut doc, "b");
    doc.mutate().assign_slot(first, &[a, b]);
    assert_eq!(doc.take_slot_changes(), [first]);
    doc.mutate().assign_slot(first, &[a, a, b]);
    assert!(doc.take_slot_changes().is_empty());
    doc.mutate().assign_slot(first, &[b, a]);
    assert_eq!(doc.take_slot_changes(), [first]);
    doc.mutate().assign_slot(second, &[a]);
    assert_eq!(doc.take_slot_changes(), [first, second]);
    doc.mutate().assign_slot(first, &[b]);
    assert!(doc.take_slot_changes().is_empty());
    doc.reclaim_detached_nodes(&[host]);
    assert!(doc.get_node(a).is_none() && doc.get_node(b).is_none());
    doc.mutate().assign_slot(first, &[]);
    doc.mutate().assign_slot(second, &[]);
    assert!(doc.take_slot_changes().is_empty());
}

#[test]
fn off_host_manual_members_signal_named_roots_and_manual_removal() {
    for manual in [true, false] {
        let mut doc = BaseDocument::new(Default::default());
        let (_, root, slot) = shadow(&mut doc, manual);
        let member = element(&mut doc, "span");
        doc.mutate().assign_slot(slot, &[member]);
        assert_eq!(doc.take_slot_changes(), [slot]);
        assert!(doc.assigned_nodes(slot).is_empty());
        doc.mutate().remove_node(slot);
        assert_eq!(
            doc.take_slot_changes(),
            if manual { vec![slot] } else { vec![] }
        );
        doc.mutate().append_children(root, &[slot]);
        assert!(doc.take_slot_changes().is_empty());
        doc.mutate().assign_slot(slot, &[]);
        assert_eq!(doc.take_slot_changes(), [slot]);
    }
}

#[test]
fn nested_hosts_refresh_the_enclosing_manual_slots() {
    let mut doc = BaseDocument::new(Default::default());
    let (host, root, slot) = shadow(&mut doc, true);
    let (nested, _, _) = shadow(&mut doc, false);
    doc.mutate().append_children(root, &[nested]);
    let child = element(&mut doc, "b");
    doc.mutate().append_children(host, &[child]);
    doc.mutate().remove_node(slot);
    doc.mutate().assign_slot(slot, &[child]);
    doc.mutate().append_children(nested, &[slot]);
    assert_eq!(doc.assigned_slot(child), Some(slot));
    assert_eq!(doc.assigned_nodes(slot), [child]);
    doc.mutate().remove_node(slot);
    assert!(doc.assigned_nodes(slot).is_empty());
}
