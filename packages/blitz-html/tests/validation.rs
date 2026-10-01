//! Required and custom constraints share native control state.
mod common;
use common::{parse, q};

#[test]
fn required_values_custom_messages_and_reset_are_independent() {
    let mut doc =
        parse("<form id=f><input id=i required><textarea id=t required>text</textarea></form>");
    let (f, i) = (q(&doc, "#f"), q(&doc, "#i"));
    assert_eq!(doc.invalid_form_controls(f).unwrap(), vec![i]);
    doc.mutate().set_form_value(i, "value");
    assert!(doc.invalid_form_controls(f).unwrap().is_empty());
    doc.mutate().set_custom_validity(i, "custom".into());
    doc.mutate().reset_form_controls(f).unwrap();
    assert_eq!(
        doc.control_validation_message(i)
            .unwrap()
            .unwrap()
            .to_string(),
        "custom"
    );
    doc.mutate()
        .set_attribute_by_name(i, "disabled", "")
        .unwrap();
    assert!(!doc.will_validate(i));
    assert!(doc.invalid_form_controls(f).unwrap().is_empty());
}

#[test]
fn radio_requirement_follows_group_ownership_and_disabled_peer_checkedness() {
    let mut doc = parse(
        "<form id=f><input id=a type=radio name=x required readonly><input id=b type=radio name=x disabled></form><form id=g><input type=radio name=x checked></form>",
    );
    let (f, a, b) = (q(&doc, "#f"), q(&doc, "#a"), q(&doc, "#b"));
    assert!(doc.will_validate(a));
    assert_eq!(doc.invalid_form_controls(f).unwrap(), vec![a]);
    doc.mutate().set_checkedness(b, true);
    assert!(doc.invalid_form_controls(f).unwrap().is_empty());
    assert!(doc.control_validation_message(a).unwrap().is_none());
}

#[test]
fn select_placeholder_differs_from_empty_nonplaceholder_option() {
    let doc = parse(
        "<form id=f><select id=a required><option value=''>placeholder</option><option>valid</option></select><select id=b required multiple><option selected value=''>empty</option></select><select id=c required><optgroup><option value=''>empty</option></optgroup></select></form>",
    );
    assert_eq!(
        doc.invalid_form_controls(q(&doc, "#f")).unwrap(),
        vec![q(&doc, "#a")]
    );
}

#[test]
fn file_required_uses_selected_files_instead_of_value_attribute() {
    let doc = parse("<form id=f><input id=file type=file required value=not-a-file></form>");
    assert_eq!(
        doc.invalid_form_controls(q(&doc, "#f")).unwrap(),
        vec![q(&doc, "#file")]
    );
}

#[test]
fn batched_eligibility_matches_scalar_queries_after_fieldset_and_datalist_changes() {
    let mut doc = parse(
        "<form id=f><fieldset disabled><legend id=first><input id=a required><fieldset disabled><legend><input id=b required></legend><input id=c required></fieldset></legend><legend id=second><input id=d required></legend><input id=e required></fieldset><datalist id=list><input id=g required></datalist><input id=h required readonly></form>",
    );
    let form = q(&doc, "#f");
    let compare = |doc: &blitz_dom::BaseDocument| {
        let expected: Vec<_> = doc
            .form_controls(form)
            .into_iter()
            .filter(|&id| doc.control_validation_message(id).unwrap().is_some())
            .collect();
        assert_eq!(doc.invalid_form_controls(form).unwrap(), expected);
    };
    compare(&doc);
    let (first, second) = (q(&doc, "#first"), q(&doc, "#second"));
    doc.mutate().insert_nodes_before(first, &[second]);
    compare(&doc);
    let (list, input) = (q(&doc, "#list"), q(&doc, "#g"));
    doc.mutate().append_children(form, &[input]);
    doc.mutate().remove_node(list);
    compare(&doc);
}

#[test]
fn native_numeric_and_text_constraints_share_form_validation() {
    let mut doc = parse(
        "<form id=f><input id=n type=number min=2 max=8 step=3 value=4><input id=e type=email value=invalid><input id=d type=date min=2024-02-28 step=2 value=2024-02-29><input id=r type=range min=80 max=20 value=50></form>",
    );
    let (f, n, e, d, r) = (
        q(&doc, "#f"),
        q(&doc, "#n"),
        q(&doc, "#e"),
        q(&doc, "#d"),
        q(&doc, "#r"),
    );
    assert_eq!(doc.invalid_form_controls(f).unwrap(), vec![n, e, d]);
    assert!(doc.control_validity(n).unwrap().step_mismatch);
    assert!(doc.control_validity(e).unwrap().type_mismatch);
    assert!(doc.control_validity(d).unwrap().step_mismatch);
    assert!(doc.control_validity(r).unwrap().valid());
    doc.mutate().set_form_value(n, "5");
    doc.mutate().set_form_value(e, " a@b\n ");
    doc.mutate().set_form_value(d, "2024-03-01");
    assert!(doc.invalid_form_controls(f).unwrap().is_empty());
    doc.mutate().set_attribute_by_name(r, "min", "90").unwrap();
    doc.mutate().remove_attribute_by_name(r, "min");
    assert_eq!(doc.form_value(r).as_deref(), Some("20"));
}
