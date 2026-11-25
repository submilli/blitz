use super::ImportMode;
use crate::custom_elements::CustomElementState;
use crate::dom_api::DomError;
use crate::dom_events::{EventTargetId, ListenerOptions};
use crate::shadow::ShadowRootInit;
use crate::{BaseDocument, DocumentConfig, NodeId, QualName, ns};

fn element(doc: &mut BaseDocument, name: &str) -> NodeId {
    doc.mutate()
        .create_element(QualName::new(None, ns!(html), name.into()), vec![])
}

fn append(doc: &mut BaseDocument, parent: NodeId, child: NodeId) {
    doc.mutate().append_children(parent, &[child]);
}

fn html(doc: &BaseDocument, id: NodeId) -> String {
    doc.get_node(id).unwrap().outer_html()
}

/// A source tree with a closed manual-slot shadow root, template contents,
/// a dirty checkbox and a custom element.
fn source() -> (BaseDocument, NodeId) {
    let mut doc = BaseDocument::new(DocumentConfig::default());
    let root = element(&mut doc, "x-host");
    let shadow = doc
        .mutate()
        .attach_shadow(
            root,
            ShadowRootInit {
                open: false,
                delegates_focus: true,
                manual_slots: true,
                clonable: false,
                serializable: false,
            },
        )
        .unwrap();
    let slot = element(&mut doc, "slot");
    append(&mut doc, shadow, slot);
    let light = element(&mut doc, "span");
    append(&mut doc, root, light);
    doc.mutate().assign_slot(slot, &[light]);
    let template = element(&mut doc, "template");
    append(&mut doc, root, template);
    let contents = doc.mutate().template_contents(template);
    let inert = element(&mut doc, "b");
    append(&mut doc, contents, inert);
    let input = element(&mut doc, "input");
    doc.mutate()
        .set_attribute(input, QualName::new(None, ns!(), "type".into()), "checkbox");
    append(&mut doc, root, input);
    doc.mutate().set_checkedness(input, true);
    doc.set_custom_element_state(root, CustomElementState::Custom);
    (doc, root)
}

#[test]
fn transfer_keeps_upgrade_phase_and_registered_connection_callbacks() {
    use crate::custom_elements::{CustomElementReaction as R, ReactionCallbacks};
    let (mut source, root) = source();
    let el = source
        .get_node_mut(root)
        .unwrap()
        .element_data_mut()
        .unwrap();
    el.custom_element_upgrading = true;
    el.custom_element_callbacks = Some(std::sync::Arc::new(ReactionCallbacks {
        connected: true,
        disconnected: false,
        ..Default::default()
    }));
    for mode in [ImportMode::Transfer, ImportMode::Copy { deep: false }] {
        let mut target = BaseDocument::new(Default::default());
        let doc = target.root_node_id;
        let new = target
            .mutate()
            .try_import_foreign(&source, root, doc, mode)
            .unwrap()
            .root();
        let moved = mode == ImportMode::Transfer;
        assert_eq!(target.custom_element_upgrading(new), moved);
        target.set_custom_element_reactions(true);
        target.observe_custom_element_enqueues();
        let refused = target.enqueue_custom_element_reactions(vec![
            R::AdoptedFromArena {
                element: new,
                old_document: 1,
                new_document: 2,
            },
            R::Connected(new),
        ]);
        if moved {
            assert!(matches!(
                refused.as_slice(),
                [R::AdoptedFromArena {
                    old_document: 1,
                    new_document: 2,
                    ..
                }]
            ));
            assert!(
                matches!(target.take_custom_element_reactions().as_slice(), [R::Connected(id)] if *id == new)
            );
        } else {
            assert!(refused.is_empty());
        }
        assert!(!target.take_custom_element_reaction_overflow());
        assert_eq!(
            target.take_enqueued_custom_element_targets().len(),
            if moved { 1 } else { 2 }
        );
        target.set_custom_element_reactions(false);
        assert!(matches!(
            target
                .enqueue_custom_element_reactions(vec![R::FormAssociatedFromArena {
                    element: new,
                    form: 7
                }])
                .as_slice(),
            [R::FormAssociatedFromArena { form: 7, .. }]
        ));
        assert!(!target.take_custom_element_reaction_overflow());
    }
}

#[test]
fn transfer_rebuilds_shadow_template_slot_and_control_state() {
    let (source, root) = source();
    let before = source.node_count();
    let mut target = BaseDocument::new(DocumentConfig::default());
    let document = target.root_node_id;
    let mut m = target.mutate();
    m.doc.set_mutation_recording(true);
    let imported = m
        .try_import_foreign(&source, root, document, ImportMode::Transfer)
        .unwrap();
    let host = imported.root();
    assert!(m.doc.take_logged_mutations().is_empty());
    drop(m);
    assert_eq!(source.node_count(), before, "the source is read-only");
    assert_eq!(target.node_document(host), document);
    assert!(target.get_node(host).unwrap().parent.is_none());
    assert_eq!(
        target.custom_element_state(host),
        CustomElementState::Custom
    );
    assert!(
        target.take_custom_element_reactions().is_empty(),
        "planting is not an adoption"
    );
    let shadow = target.shadow_root_of(host).unwrap();
    let data = target.nodes[shadow].shadow_root_data.as_deref().unwrap();
    assert!(!data.open && data.delegates_focus && data.manual_slots);
    assert_eq!(data.host, host);
    let slot = target.nodes[shadow].children[0];
    let light = target.nodes[host].children[0];
    assert_eq!(target.assigned_nodes(slot), vec![light]);
    let template = target.nodes[host].children[1];
    let contents = target.nodes[template]
        .element_data()
        .unwrap()
        .template_contents
        .unwrap();
    assert_eq!(html(&target, target.nodes[contents].children[0]), "<b></b>");
    assert_eq!(
        target.node_document(contents),
        target.template_document(host)
    );
    let input = target.nodes[host].children[2];
    assert!(target.checkedness(input));
    let form = &target.nodes[input].element_data().unwrap().form_state;
    assert!(form.checked_dirty);
    // Shadow-including preorder: root, shadow root, slot, light child,
    // template, template contents, its child, input.
    let order: Vec<_> = imported.pairs().iter().map(|&(_, id)| id).collect();
    assert_eq!(
        order,
        [
            host,
            shadow,
            slot,
            light,
            template,
            contents,
            target.nodes[contents].children[0],
            input
        ]
    );
}

#[test]
fn transferring_a_settled_slot_does_not_signal_reconstruction() {
    let (mut source, host) = source();
    let shadow = source.shadow_root_of(host).unwrap();
    let slot = source.nodes[shadow].children[0];
    let fallback = source.mutate().create_text_node("fallback");
    append(&mut source, slot, fallback);
    assert_eq!(source.take_slot_changes(), [slot]);

    let (mut target, other_host) = self::source();
    let other_root = target.shadow_root_of(other_host).unwrap();
    let pending = target.nodes[other_root].children[0];
    let owner = target.root_node_id;
    let moved = target
        .mutate()
        .try_import_foreign(&source, host, owner, ImportMode::Transfer)
        .unwrap();
    assert_eq!(target.take_slot_changes(), [pending]);
    let shadow = target.shadow_root_of(moved.root()).unwrap();
    let slot = target.nodes[shadow].children[0];
    let light = target.nodes[moved.root()].children[0];
    assert_eq!(target.assigned_nodes(slot), [light]);
    target.mutate().remove_node(light);
    assert_eq!(target.take_slot_changes(), [slot]);
}

#[test]
fn copy_follows_clone_steps() {
    let (source, root) = source();
    let mut target = BaseDocument::new(DocumentConfig::default());
    let document = target.root_node_id;
    let shallow = target
        .mutate()
        .try_import_foreign(&source, root, document, ImportMode::Copy { deep: false })
        .unwrap();
    assert_eq!(shallow.pairs().len(), 1, "a closed root is not clonable");
    let host = shallow.root();
    assert!(target.nodes[host].children.is_empty());
    assert_eq!(
        target.custom_element_state(host),
        CustomElementState::Undefined
    );
    let deep = target
        .mutate()
        .try_import_foreign(&source, root, document, ImportMode::Copy { deep: true })
        .unwrap();
    let host = deep.root();
    assert!(target.shadow_root_of(host).is_none());
    assert_eq!(target.nodes[host].children.len(), 3);
    let input = target.nodes[host].children[2];
    assert!(target.checkedness(input), "input checkedness is cloned");
}

#[test]
fn rejected_admission_and_documents_change_nothing() {
    let (source, root) = source();
    let mut target = BaseDocument::new(DocumentConfig {
        node_limit: Some(6),
        ..Default::default()
    });
    let document = target.root_node_id;
    let before = target.node_count();
    let result = target
        .mutate()
        .try_import_foreign(&source, root, document, ImportMode::Transfer);
    assert_eq!(result.unwrap_err(), DomError::QuotaExceeded);
    assert_eq!(target.node_count(), before);
    let source_document = source.root_node_id;
    let result = target.mutate().try_import_foreign(
        &source,
        source_document,
        document,
        ImportMode::Transfer,
    );
    assert_eq!(result.unwrap_err(), DomError::NotSupported);
    assert_eq!(target.node_count(), before);
}

#[test]
fn discarded_imports_release_every_node() {
    let (source, root) = source();
    let mut target = BaseDocument::new(DocumentConfig::default());
    let document = target.root_node_id;
    let before = target.node_count();
    let mut m = target.mutate();
    let imported = m
        .try_import_foreign(&source, root, document, ImportMode::Transfer)
        .unwrap();
    let ids: Vec<_> = imported.pairs().iter().map(|&(_, id)| id).collect();
    m.discard_import(imported);
    drop(m);
    assert_eq!(target.node_count(), before);
    assert!(ids.iter().all(|&id| target.get_node(id).is_none()));
}

#[test]
fn taken_listeners_move_without_being_marked_removed() {
    let mut doc = BaseDocument::new(DocumentConfig::default());
    let (from, to) = (
        EventTargetId::Node(element(&mut doc, "a")),
        EventTargetId::Node(element(&mut doc, "b")),
    );
    let listeners = &mut doc.event_listeners;
    listeners.add(to, "click", 1, ListenerOptions::default());
    listeners.add(from, "click", 2, ListenerOptions::default());
    listeners.add(from, "input", 3, ListenerOptions::default());
    let snapshot = listeners.listeners(from, "click");
    let taken = listeners.take_target(from);
    listeners.restore_target(to, taken);
    assert!(listeners.callbacks_of(from).is_empty());
    assert_eq!(listeners.callbacks_of(to), vec![1, 2, 3]);
    assert!(!snapshot[0].removed.get());
}

#[test]
fn attribute_lookup_with_an_id_from_another_arena_is_none() {
    let mut other = BaseDocument::new(DocumentConfig::default());
    for _ in 0..8 {
        element(&mut other, "div");
    }
    let foreign = element(&mut other, "div");
    let doc = BaseDocument::new(DocumentConfig::default());
    assert!(doc.get_node(foreign).is_none());
    assert!(doc.attribute_by_name(foreign, "id").is_none());
}

#[test]
fn copied_template_contents_preserve_pending_observations() {
    let mut target = BaseDocument::new(Default::default());
    let mut foreign = BaseDocument::new(Default::default());
    let a = target.mutate().create_document_fragment();
    let b = foreign.mutate().create_document_fragment();
    let local = element(&mut target, "i");
    let remote = element(&mut foreign, "b");
    append(&mut target, a, local);
    append(&mut foreign, b, remote);
    let observer_target = element(&mut target, "div");
    let changed = element(&mut target, "p");
    target.set_mutation_recording(true);
    append(&mut target, observer_target, changed);
    for (source, original, name) in [(None, a, "<i></i>"), (Some(&foreign), b, "<b></b>")] {
        let template = element(&mut target, "template");
        let copied = target
            .mutate()
            .try_copy_template_contents(template, source, original)
            .unwrap();
        let child = target.get_node(copied).unwrap().children[0];
        assert_eq!(target.host_including_parent(copied), Some(template));
        assert_eq!(
            target.node_document(child),
            target.template_document(template)
        );
        assert_eq!(html(&target, child), name);
        let count = target.node_count();
        assert_eq!(
            target
                .mutate()
                .try_copy_template_contents(template, source, original),
            Err(DomError::HierarchyRequest)
        );
        assert_eq!(target.node_count(), count);
    }
    let records = target.take_mutation_records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].target(), observer_target);
}
