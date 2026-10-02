//! Custom form state participates in the same native algorithms as built-ins.
mod common;
use blitz_dom::{
    ValidityState,
    custom_elements::{CustomElementReaction, CustomElementState},
    custom_internals::CustomFormValue,
};
use blitz_traits::net::{Entry, EntryValue};
use common::{parse, q};

fn upgrade(doc: &mut blitz_dom::BaseDocument, id: blitz_dom::NodeId) {
    doc.register_form_associated(id);
    doc.set_custom_element_state(id, CustomElementState::Custom);
    doc.refresh_custom_form_association(id);
}
#[test]
fn custom_control_ownership_labels_values_and_disabled_validation() {
    let mut doc = parse(
        "<form id=f><label id=l for=c>Label</label><fieldset id=s><x-control id=c name=custom></x-control></fieldset></form><form id=g></form>",
    );
    let (form, control, fieldset, label, other) = (
        q(&doc, "#f"),
        q(&doc, "#c"),
        q(&doc, "#s"),
        q(&doc, "#l"),
        q(&doc, "#g"),
    );
    upgrade(&mut doc, control);
    assert_eq!(doc.form_owner(control), Some(form));
    assert_eq!(doc.custom_element_labels(control), vec![label]);
    doc.set_internals_form_value(
        control,
        CustomFormValue::Single(EntryValue::String("value".into())),
        CustomFormValue::Null,
    );
    assert_eq!(
        doc.form_entry_list(form, None).unwrap().0,
        vec![Entry {
            name: "custom".into(),
            value: EntryValue::String("value".into())
        }]
    );
    doc.set_internals_validity(
        control,
        ValidityState {
            custom_error: true,
            ..Default::default()
        },
        "invalid".into(),
        None,
    )
    .unwrap();
    assert_eq!(doc.invalid_form_controls(form).unwrap(), vec![control]);
    doc.mutate()
        .set_attribute_by_name(fieldset, "disabled", "")
        .unwrap();
    assert!(!doc.will_validate(control));
    assert!(doc.invalid_form_controls(form).unwrap().is_empty());
    assert!(doc.form_entry_list(form, None).unwrap().0.is_empty());
    doc.mutate().remove_attribute_by_name(fieldset, "disabled");
    doc.mutate()
        .set_attribute_by_name(control, "form", "g")
        .unwrap();
    assert_eq!(doc.form_owner(control), Some(other));
    assert!(doc.form_controls(form).iter().all(|&id| id != control));
    assert_eq!(doc.form_controls(other), vec![control]);
}
#[test]
fn removal_and_reinsertion_reset_association_and_reset_is_a_reaction() {
    let mut doc = parse("<form id=f><x-control id=c></x-control><fieldset id=s></fieldset></form>");
    doc.set_custom_element_reactions(true);
    let (form, control, fieldset) = (q(&doc, "#f"), q(&doc, "#c"), q(&doc, "#s"));
    upgrade(&mut doc, control);
    doc.take_custom_element_reactions();
    doc.mutate().append_children(fieldset, &[control]);
    let callbacks: Vec<_> = doc
        .take_custom_element_reactions()
        .into_iter()
        .filter_map(|r| match r {
            CustomElementReaction::FormAssociated { form, .. } => Some(form),
            _ => None,
        })
        .collect();
    assert_eq!(callbacks, vec![None, Some(form)]);
    doc.mutate().reset_form_controls(form).unwrap();
    assert!(
        doc.take_custom_element_reactions()
            .iter()
            .any(|r| matches!(r,CustomElementReaction::FormReset(id) if *id==control))
    );
}
#[test]
fn states_match_selectors_and_survive_an_empty_document() {
    let mut doc = parse("<x-control id=c></x-control>");
    let control = q(&doc, "#c");
    doc.set_custom_state(control, "ready", true);
    assert_eq!(
        doc.query_selector(":state(ready)").ok().flatten(),
        Some(control)
    );
    doc.clear_custom_states(control);
    assert!(doc.query_selector(":state(ready)").ok().flatten().is_none());
    let html = q(&doc, "html");
    doc.mutate().remove_node(html);
    doc.set_custom_state(control, "detached", true);
    doc.clear_custom_states(control);
}

#[test]
fn oversized_validation_message_preserves_previous_validity_and_anchor() {
    use blitz_dom::{DomString, custom_internals::ValidationMessageLimit};

    let mut doc =
        parse("<form><x-control id=c><input id=first><input id=second></x-control></form>");
    let (control, first, second) = (q(&doc, "#c"), q(&doc, "#first"), q(&doc, "#second"));
    upgrade(&mut doc, control);
    let original_validity = ValidityState {
        custom_error: true,
        ..Default::default()
    };
    let boundary_message: DomString = "x".repeat(256 * 1024).into();
    assert_eq!(boundary_message.retained_bytes(), 256 * 1024);
    doc.set_internals_validity(control, original_validity, boundary_message, Some(first))
        .unwrap();

    // The budget covers retained capacity, not only the visible string length.
    let mut spare_capacity = String::with_capacity(256 * 1024 + 1);
    spare_capacity.push('x');
    let oversized_messages = [
        DomString::from("x".repeat(256 * 1024 + 1)),
        DomString::from(spare_capacity),
        DomString::from_utf16(vec![0xd800; 128 * 1024 + 1]),
    ];
    for message in oversized_messages {
        assert_eq!(
            doc.set_internals_validity(control, ValidityState::default(), message, Some(second),),
            Err(ValidationMessageLimit),
        );
        assert_eq!(doc.control_validity(control).unwrap(), original_validity);
        assert_eq!(doc.internals_validation_anchor(control), first);
        let retained = doc.control_validation_message(control).unwrap().unwrap();
        assert_eq!(retained.as_str_lossy(), "x".repeat(256 * 1024));
    }
}

#[test]
fn form_elements_tracks_upgrade_state_and_excludes_only_image_inputs() {
    let mut doc = parse(
        "<form id=f><input id=image type=ImAgE><input id=text><button id=button></button><x-control id=custom></x-control><x-control id=failed></x-control></form>",
    );
    let (form, image, text, button, custom, failed) = (
        q(&doc, "#f"),
        q(&doc, "#image"),
        q(&doc, "#text"),
        q(&doc, "#button"),
        q(&doc, "#custom"),
        q(&doc, "#failed"),
    );
    assert_eq!(doc.form_elements(form), vec![text, button]);
    upgrade(&mut doc, custom);
    doc.register_form_associated(failed);
    doc.set_custom_element_state(failed, CustomElementState::Precustomized);
    doc.set_custom_element_state(failed, CustomElementState::Failed);
    assert_eq!(doc.form_elements(form), vec![text, button, custom]);
    assert_eq!(doc.form_controls(form), vec![image, text, button, custom]);

    doc.mutate()
        .set_attribute_by_name(image, "type", "text")
        .unwrap();
    assert_eq!(doc.form_elements(form), vec![image, text, button, custom]);
    doc.mutate()
        .set_attribute_by_name(image, "type", "IMAGE")
        .unwrap();
    assert_eq!(doc.form_elements(form), vec![text, button, custom]);
    assert_eq!(doc.form_controls(form), vec![image, text, button, custom]);
}
