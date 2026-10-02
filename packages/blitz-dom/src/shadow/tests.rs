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
    assert!(doc.nodes[slot].manual_slottables.is_empty());
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
