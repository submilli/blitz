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
