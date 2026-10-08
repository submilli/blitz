use super::*;
use crate::{DocumentConfig, QualName};

fn element(doc: &mut BaseDocument, parent: NodeId, tag: &str, name: &str) -> NodeId {
    let id = doc
        .mutate()
        .create_element(QualName::new(None, ns!(html), tag.into()), vec![]);
    doc.mutate()
        .set_attribute_by_name(id, "name", name)
        .unwrap();
    doc.mutate().append_children(parent, &[id]);
    id
}

#[test]
fn snapshots_match_new_arena_controls_and_keep_defaults_distinct() {
    let mut source = BaseDocument::new(DocumentConfig::default());
    let root = source.root_node().id;
    let form = element(&mut source, root, "form", "form");
    let text = element(&mut source, form, "input", "text");
    let check = element(&mut source, form, "input", "check");
    source
        .mutate()
        .set_attribute_by_name(check, "type", "checkbox")
        .unwrap();
    source
        .mutate()
        .set_form_value_dom(text, &DomString::from_utf16(vec![0x61, 0xd800]));
    source.mutate().set_checkedness(check, true);
    let snapshots = source.capture_form_state().unwrap();

    let mut target = BaseDocument::new(DocumentConfig::default());
    let root = target.root_node().id;
    // Allocate an unrelated detached node to prevent matching native IDs.
    target
        .mutate()
        .create_element(QualName::new(None, ns!(html), "div".into()), vec![]);
    let form = element(&mut target, root, "form", "form");
    let text = element(&mut target, form, "input", "text");
    let check = element(&mut target, form, "input", "check");
    target
        .mutate()
        .set_attribute_by_name(text, "value", "default")
        .unwrap();
    target
        .mutate()
        .set_attribute_by_name(check, "type", "checkbox")
        .unwrap();
    for (id, key) in target.persisted_control_keys().unwrap() {
        if let Some(saved) = snapshots.iter().find(|saved| saved.key == key) {
            target.restore_control_state(id, &saved.state).unwrap();
        }
    }
    assert_eq!(
        target.form_value_dom(text).unwrap().to_utf16().as_ref(),
        &[0x61, 0xd800]
    );
    assert!(target.checkedness(check));
    assert_eq!(
        target
            .null_attribute(text, "value")
            .unwrap()
            .value
            .as_str_lossy(),
        "default"
    );
    target.mutate().reset_form_controls(form).unwrap();
    assert_eq!(target.form_value(text).as_deref(), Some("default"));
    assert!(!target.checkedness(check));
}

#[test]
fn duplicate_occurrences_and_private_controls_are_explicit() {
    let mut doc = BaseDocument::new(DocumentConfig::default());
    let root = doc.root_node().id;
    element(&mut doc, root, "input", "same");
    element(&mut doc, root, "input", "same");
    let private = element(&mut doc, root, "input", "private");
    doc.mutate()
        .set_attribute_by_name(private, "autocomplete", "OFF")
        .unwrap();
    let password = element(&mut doc, root, "input", "password");
    doc.mutate()
        .set_attribute_by_name(password, "type", "password")
        .unwrap();
    let keys = doc.persisted_control_keys().unwrap();
    assert_eq!(keys.len(), 2);
    assert_eq!(keys[0].1.occurrence, 0);
    assert_eq!(keys[1].1.occurrence, 1);
}

#[test]
fn custom_state_preserves_file_and_form_data_without_changing_submission() {
    use blitz_traits::net::Entry;
    let mut doc = BaseDocument::new(DocumentConfig::default());
    let root = doc.root_node().id;
    let custom = element(&mut doc, root, "x-control", "custom");
    doc.register_form_associated(custom);
    let file = FormFile {
        last_modified: 123.0,
        name: "saved.txt".into(),
        content_type: "text/plain".into(),
        bytes: b"abc".as_slice().into(),
    };
    let states = [
        CustomFormValue::Single(EntryValue::String("saved".into())),
        CustomFormValue::Single(EntryValue::FileContents(file.clone())),
        CustomFormValue::Entries(vec![Entry {
            name: "file".into(),
            value: EntryValue::FileContents(file),
        }]),
    ];
    for state in states {
        doc.set_internals_form_value(
            custom,
            CustomFormValue::Single(EntryValue::String("submitted".into())),
            state,
        );
        let snapshot = doc.capture_form_state().unwrap();
        assert_eq!(snapshot.len(), 1);
        assert!(matches!(snapshot[0].state, ControlState::Custom(_)));
        doc.restore_control_state(custom, &snapshot[0].state)
            .unwrap();
        let internals = doc
            .get_node(custom)
            .unwrap()
            .element_data()
            .unwrap()
            .custom_internals
            .as_ref()
            .unwrap();
        assert!(
            matches!(&internals.value, CustomFormValue::Single(EntryValue::String(value)) if value == "submitted")
        );
    }
    doc.set_internals_form_value(custom, CustomFormValue::Null, CustomFormValue::Null);
    assert!(doc.capture_form_state().unwrap().is_empty());
}

#[test]
fn snapshot_limits_and_wrong_control_types_leave_live_state_unchanged() {
    let mut doc = BaseDocument::new(DocumentConfig::default());
    let root = doc.root_node().id;
    let text = element(&mut doc, root, "input", "text");
    doc.mutate().set_form_value(text, "unchanged");
    assert_eq!(
        doc.restore_control_state(text, &ControlState::Checked(true)),
        Err(RestoreError::Mismatch)
    );
    assert_eq!(doc.form_value(text).as_deref(), Some("unchanged"));
    for _ in 0..MAX_CONTROLS {
        element(&mut doc, root, "input", "duplicate");
    }
    assert!(matches!(doc.capture_form_state(), Err(FormStateLimit)));
    assert_eq!(doc.form_value(text).as_deref(), Some("unchanged"));
}

#[test]
fn explicit_empty_select_state_survives_without_default_reconciliation() {
    let mut doc = BaseDocument::new(DocumentConfig::default());
    let root = doc.root_node().id;
    let select = element(&mut doc, root, "select", "select");
    element(&mut doc, select, "option", "a");
    element(&mut doc, select, "option", "b");
    doc.mutate().set_select_index(select, -1);
    let snapshot = doc.capture_form_state().unwrap();
    doc.mutate().set_select_index(select, 0);
    doc.restore_control_state(select, &snapshot[0].state)
        .unwrap();
    assert!(doc.selected_options(select).is_empty());
}

#[test]
fn explicit_autocomplete_on_overrides_form_policy() {
    let mut doc = BaseDocument::new(DocumentConfig::default());
    let root = doc.root_node().id;
    let form = element(&mut doc, root, "form", "form");
    doc.mutate()
        .set_attribute_by_name(form, "autocomplete", "off")
        .unwrap();
    let inherited = element(&mut doc, form, "input", "inherited");
    let explicit = element(&mut doc, form, "input", "explicit");
    doc.mutate()
        .set_attribute_by_name(explicit, "autocomplete", "on")
        .unwrap();
    let keys = doc.persisted_control_keys().unwrap();
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].0, explicit);
    doc.mutate()
        .set_attribute_by_name(form, "autocomplete", "on")
        .unwrap();
    doc.mutate()
        .set_attribute_by_name(explicit, "autocomplete", "off")
        .unwrap();
    let keys = doc.persisted_control_keys().unwrap();
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].0, inherited);
}

#[test]
fn decorative_custom_elements_do_not_exhaust_control_admission() {
    let mut doc = BaseDocument::new(DocumentConfig::default());
    let root = doc.root_node().id;
    let input = element(&mut doc, root, "input", "real");
    doc.mutate().set_form_value(input, "saved");
    for _ in 0..MAX_CONTROLS + 1 {
        element(&mut doc, root, "x-card", "decoration");
    }
    let snapshot = doc.capture_form_state().unwrap();
    assert_eq!(snapshot.len(), 1);
    assert!(
        matches!(&snapshot[0].state, ControlState::Text(value) if value.as_str_lossy() == "saved")
    );
}

#[test]
fn stable_form_identity_survives_reordering() {
    let mut doc = BaseDocument::new(DocumentConfig::default());
    let root = doc.root_node().id;
    let a = element(&mut doc, root, "form", "a");
    let b = element(&mut doc, root, "form", "b");
    let input_a = element(&mut doc, a, "input", "same");
    let input_b = element(&mut doc, b, "input", "same");
    let keys = doc.persisted_control_keys().unwrap();
    let key_a = keys
        .iter()
        .find(|(id, _)| *id == input_a)
        .unwrap()
        .1
        .clone();
    let key_b = keys
        .iter()
        .find(|(id, _)| *id == input_b)
        .unwrap()
        .1
        .clone();
    doc.mutate().append_children(root, &[a]);
    let keys = doc.persisted_control_keys().unwrap();
    assert_eq!(keys.iter().find(|(id, _)| *id == input_a).unwrap().1, key_a);
    assert_eq!(keys.iter().find(|(id, _)| *id == input_b).unwrap().1, key_b);
    assert_ne!(key_a, key_b);
}

#[test]
fn identified_shadow_scopes_cannot_consume_another_scopes_state() {
    let mut doc = BaseDocument::new(DocumentConfig::default());
    let root = doc.root_node().id;
    let a = element(&mut doc, root, "div", "a");
    let b = element(&mut doc, root, "div", "b");
    doc.mutate().set_attribute_by_name(a, "id", "a").unwrap();
    doc.mutate().set_attribute_by_name(b, "id", "b").unwrap();
    let a_root = doc
        .mutate()
        .attach_shadow(
            a,
            crate::shadow::ShadowRootInit {
                open: false,
                ..Default::default()
            },
        )
        .unwrap();
    let b_root = doc
        .mutate()
        .attach_shadow(
            b,
            crate::shadow::ShadowRootInit {
                open: true,
                ..Default::default()
            },
        )
        .unwrap();
    let input_a = element(&mut doc, a_root, "input", "same");
    let input_b = element(&mut doc, b_root, "input", "same");
    doc.mutate().set_form_value(input_a, "private-a");
    doc.mutate().set_form_value(input_b, "private-b");
    let old = doc.capture_form_state().unwrap();
    doc.mutate().remove_node(input_a);
    let keys = doc.persisted_control_keys().unwrap();
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].0, input_b);
    assert_ne!(keys[0].1, old[0].key);
    assert_eq!(keys[0].1, old[1].key);
    doc.mutate().set_attribute_by_name(b, "id", "").unwrap();
    assert!(doc.capture_form_state().unwrap().is_empty());
}

#[test]
fn borrowed_oversized_values_and_metadata_are_rejected_before_capture() {
    let mut doc = BaseDocument::new(DocumentConfig::default());
    let root = doc.root_node().id;
    let input = element(&mut doc, root, "input", "large");
    // Install hostile backing data directly, bypassing the normal setter's own
    // allocations; capture must reject its borrowed size before projection.
    doc.get_node_mut(input)
        .unwrap()
        .element_data_mut()
        .unwrap()
        .form_state
        .value = Some("x".repeat(MAX_BYTES + 1).into());
    assert!(matches!(doc.capture_form_state(), Err(FormStateLimit)));
    doc.get_node_mut(input)
        .unwrap()
        .element_data_mut()
        .unwrap()
        .form_state
        .value = Some("small".into());
    doc.mutate()
        .set_attribute_by_name(input, "type", "x".repeat(1024))
        .unwrap();
    assert!(matches!(doc.capture_form_state(), Err(FormStateLimit)));
    doc.mutate()
        .set_attribute_by_name(input, "type", "text")
        .unwrap();
    assert_eq!(doc.capture_form_state().unwrap().len(), 1);
}

#[test]
fn ambient_file_paths_are_rejected_and_file_bytes_remain_immutable() {
    let mut doc = BaseDocument::new(DocumentConfig::default());
    let root = doc.root_node().id;
    let custom = element(&mut doc, root, "x-control", "file");
    doc.register_form_associated(custom);
    doc.set_internals_form_value(
        custom,
        CustomFormValue::Null,
        CustomFormValue::Single(EntryValue::File("/private/not-authorized".into())),
    );
    assert!(matches!(doc.capture_form_state(), Err(FormStateLimit)));
    let file = FormFile {
        last_modified: 123.0,
        name: "safe.txt".into(),
        content_type: "text/plain".into(),
        bytes: b"original".as_slice().into(),
    };
    doc.set_internals_form_value(
        custom,
        CustomFormValue::Null,
        CustomFormValue::Single(EntryValue::FileContents(file.clone())),
    );
    let snapshot = doc.capture_form_state().unwrap();
    doc.set_internals_form_value(custom, CustomFormValue::Null, CustomFormValue::Null);
    let ControlState::Custom(CustomFormValue::Single(EntryValue::FileContents(saved))) =
        &snapshot[0].state
    else {
        panic!("snapshot retains its file variant")
    };
    assert_eq!(saved, &file);
    assert_eq!(saved.bytes.as_ref(), b"original");
}

#[test]
fn changed_option_values_reject_restoration_without_partial_selectedness() {
    let mut doc = BaseDocument::new(DocumentConfig::default());
    let root = doc.root_node().id;
    let select = element(&mut doc, root, "select", "select");
    let a = element(&mut doc, select, "option", "a");
    let b = element(&mut doc, select, "option", "b");
    doc.mutate().set_attribute_by_name(a, "value", "a").unwrap();
    doc.mutate().set_attribute_by_name(b, "value", "b").unwrap();
    doc.mutate().set_select_index(select, 1);
    let snapshot = doc.capture_form_state().unwrap();
    doc.mutate().set_select_index(select, 0);
    doc.mutate()
        .set_attribute_by_name(b, "value", "changed")
        .unwrap();
    assert_eq!(
        doc.restore_control_state(select, &snapshot[0].state),
        Err(RestoreError::Mismatch)
    );
    assert_eq!(doc.selected_options(select), vec![a]);
}

#[test]
fn multiple_selection_cannot_restore_into_single_select() {
    let mut doc = BaseDocument::new(DocumentConfig::default());
    let root = doc.root_node().id;
    let select = element(&mut doc, root, "select", "select");
    doc.mutate()
        .set_attribute_by_name(select, "multiple", "")
        .unwrap();
    let a = element(&mut doc, select, "option", "a");
    let b = element(&mut doc, select, "option", "b");
    doc.mutate().write_option_selectedness(a, true, Some(true));
    doc.mutate().write_option_selectedness(b, true, Some(true));
    let saved = doc.capture_form_state().unwrap();
    doc.restore_control_state(select, &saved[0].state).unwrap();
    assert_eq!(doc.selected_options(select), vec![a, b]);
    doc.mutate().remove_attribute_by_name(select, "multiple");
    doc.mutate().set_select_index(select, 0);
    assert_ne!(doc.persisted_control_keys().unwrap()[0].1, saved[0].key);
    assert_eq!(
        doc.restore_control_state(select, &saved[0].state),
        Err(RestoreError::Mismatch)
    );
    assert_eq!(doc.selected_options(select), vec![a]);
}

#[test]
fn shadow_form_occurrences_are_independent_of_light_dom_forms() {
    let mut doc = BaseDocument::new(DocumentConfig::default());
    let root = doc.root_node().id;
    let host = element(&mut doc, root, "div", "host");
    doc.mutate()
        .set_attribute_by_name(host, "id", "host")
        .unwrap();
    let shadow = doc
        .mutate()
        .attach_shadow(host, crate::shadow::ShadowRootInit::default())
        .unwrap();
    let form = element(&mut doc, shadow, "form", "");
    let input = element(&mut doc, form, "input", "same");
    let saved = doc.persisted_control_keys().unwrap()[0].1.clone();
    element(&mut doc, root, "form", "");
    // Move the host after the new light-DOM form so a global counter would
    // change the shadow form occurrence and fail this assertion.
    doc.mutate().append_children(root, &[host]);
    let keys = doc.persisted_control_keys().unwrap();
    assert_eq!(keys[0].0, input);
    assert_eq!(keys[0].1, saved);
}

#[test]
fn oversized_target_text_is_rejected_before_restoration() {
    for tag in ["input", "textarea"] {
        let mut doc = BaseDocument::new(DocumentConfig::default());
        let root = doc.root_node().id;
        let input = element(&mut doc, root, tag, "large");
        doc.get_node_mut(input)
            .unwrap()
            .element_data_mut()
            .unwrap()
            .form_state
            .value = Some("x".repeat(MAX_BYTES + 1).into());
        assert_eq!(
            doc.restore_control_state(input, &ControlState::Text("small".into())),
            Err(RestoreError::Limit)
        );
        assert!(
            doc.get_node(input)
                .unwrap()
                .element_data()
                .unwrap()
                .form_state
                .value
                .as_ref()
                .unwrap()
                .retained_bytes()
                > MAX_BYTES
        );
    }
}

#[test]
fn excessive_shadow_depth_is_rejected_before_form_ownership_work() {
    let mut doc = BaseDocument::new(DocumentConfig::default());
    let mut parent = doc.root_node().id;
    for _ in 0..65 {
        let host = element(&mut doc, parent, "div", "");
        parent = doc
            .mutate()
            .attach_shadow(host, crate::shadow::ShadowRootInit::default())
            .unwrap();
    }
    assert!(matches!(doc.capture_form_state(), Err(FormStateLimit)));
}

#[test]
fn deep_light_dom_hosts_are_admitted_before_ancestry_work() {
    let mut doc = BaseDocument::new(DocumentConfig::default());
    let mut parent = doc.root_node().id;
    for _ in 0..MAX_TREE_DEPTH + 1 {
        let host = element(&mut doc, parent, "div", "");
        doc.mutate()
            .attach_shadow(host, crate::shadow::ShadowRootInit::default())
            .unwrap();
        parent = host;
    }
    assert!(matches!(doc.capture_form_state(), Err(FormStateLimit)));
}

#[test]
fn custom_occurrences_do_not_shift_with_definition_registration_order() {
    let mut doc = BaseDocument::new(DocumentConfig::default());
    let root = doc.root_node().id;
    let form = element(&mut doc, root, "form", "form");
    let a = element(&mut doc, form, "x-late", "duplicate");
    let b = element(&mut doc, form, "x-late", "duplicate");
    doc.register_form_associated(b);
    let key = doc
        .persisted_control_keys()
        .unwrap()
        .into_iter()
        .find(|(id, _)| *id == b)
        .unwrap()
        .1;
    assert_eq!(key.occurrence, 1);
    doc.register_form_associated(a);
    let after = doc
        .persisted_control_keys()
        .unwrap()
        .into_iter()
        .find(|(id, _)| *id == b)
        .unwrap()
        .1;
    assert_eq!(key, after);
}

#[test]
fn textarea_radio_groups_and_authorized_file_inputs_restore_live_state() {
    let mut doc = BaseDocument::new(DocumentConfig::default());
    let root = doc.root_node().id;
    let form = element(&mut doc, root, "form", "form");
    let area = element(&mut doc, form, "textarea", "area");
    let a = element(&mut doc, form, "input", "radio");
    let b = element(&mut doc, form, "input", "radio");
    for id in [a, b] {
        doc.mutate()
            .set_attribute_by_name(id, "type", "radio")
            .unwrap();
    }
    let file = element(&mut doc, form, "input", "file");
    doc.mutate()
        .set_attribute_by_name(file, "type", "file")
        .unwrap();
    let saved_file = FormFile {
        name: "saved.txt".into(),
        content_type: "text/plain".into(),
        last_modified: 42.0,
        bytes: b"bytes".as_slice().into(),
    };
    doc.mutate().set_form_value(area, "saved area");
    doc.mutate().set_checkedness(b, true);
    doc.mutate()
        .set_form_files(file, vec![saved_file.clone()])
        .unwrap();
    let saved = doc.capture_form_state().unwrap();
    doc.mutate().set_form_value(area, "changed");
    doc.mutate().set_checkedness(a, true);
    doc.mutate().set_form_files(file, vec![]).unwrap();
    for (id, key) in doc.persisted_control_keys().unwrap() {
        let state = &saved.iter().find(|s| s.key == key).unwrap().state;
        doc.restore_control_state(id, state).unwrap();
    }
    assert_eq!(doc.form_value(area).as_deref(), Some("saved area"));
    assert!(!doc.checkedness(a));
    assert!(doc.checkedness(b));
    let again = doc.capture_form_state().unwrap();
    let ControlState::Files(files) = &again
        .iter()
        .find(|s| s.key.name.as_str_lossy() == "file")
        .unwrap()
        .state
    else {
        panic!("file control snapshots preserve their variant")
    };
    assert_eq!(files, &[saved_file]);
}
#[test]
fn identified_duplicate_controls_keep_identity_when_predecessor_is_detached() {
    let mut doc = BaseDocument::new(DocumentConfig::default());
    let root = doc.root_node().id;
    let a = element(&mut doc, root, "x-state", "same");
    let b = element(&mut doc, root, "x-state", "same");
    doc.mutate().set_attribute_by_name(a, "id", "a").unwrap();
    doc.mutate().set_attribute_by_name(b, "id", "b").unwrap();
    doc.register_form_associated(a);
    doc.register_form_associated(b);
    let key = doc
        .persisted_control_keys()
        .unwrap()
        .into_iter()
        .find(|(id, _)| *id == b)
        .unwrap()
        .1;
    doc.mutate().remove_node(a);
    assert_eq!(doc.persisted_control_keys().unwrap()[0].1, key);
}
