mod common;
use blitz_dom::{EventHandler, FormAction, NoopEventHandler, ValidationError};
use common::{parse, q};

#[test]
fn native_patterns_share_scalar_and_form_validation() {
    let mut doc = parse(
        r#"<form id=f><input id=i pattern="a|ab" value=ab><input id=j pattern="[" value=x></form>"#,
    );
    let (f, i) = (q(&doc, "#f"), q(&doc, "#i"));
    assert!(doc.invalid_form_controls(f).unwrap().is_empty());
    doc.mutate().set_form_value(i, "abc");
    assert!(doc.control_validity(i).unwrap().pattern_mismatch);
    assert_eq!(doc.invalid_form_controls(f).unwrap(), [i]);
    doc.mutate()
        .set_attribute_by_name(i, "pattern", "")
        .unwrap();
    assert!(doc.control_validity(i).unwrap().pattern_mismatch);
    doc.mutate().set_form_value(i, "");
    assert!(doc.control_validity(i).unwrap().valid());
}

#[test]
fn exhaustion_aborts_validation_and_native_actions_record_the_error() {
    let mut doc = parse("<form id=f><input id=i value=a></form>");
    let (f, i) = (q(&doc, "#f"), q(&doc, "#i"));
    doc.mutate()
        .set_attribute_by_name(i, "pattern", "a".repeat(4097))
        .unwrap();
    assert_eq!(doc.control_validity(i), Err(ValidationError));
    assert_eq!(doc.invalid_form_controls(f), Err(ValidationError));
    NoopEventHandler.handle_form_action(
        FormAction::Submit {
            form: f,
            submitter: None,
        },
        &mut doc,
    );
    assert_eq!(doc.take_validation_error(), Some(ValidationError));
    assert_eq!(doc.take_validation_error(), None);
    doc.mutate()
        .set_attribute_by_name(i, "pattern", "a")
        .unwrap();
    assert!(doc.invalid_form_controls(f).unwrap().is_empty());
}

#[test]
fn form_validation_shares_its_budget_across_controls() {
    let pattern = "a".repeat(256);
    let html = format!(
        "<form id=f>{}</form>",
        format!("<input pattern='{pattern}' value='{pattern}'>").repeat(128)
    );
    let doc = parse(&html);
    let first = q(&doc, "input");
    assert!(doc.control_validity(first).unwrap().valid());
    assert_eq!(
        doc.invalid_form_controls(q(&doc, "#f")),
        Err(ValidationError)
    );
}

#[test]
fn pattern_admission_includes_value_normalization() {
    let mut doc = parse("<form id=f><input id=i type=email multiple required pattern='.*'></form>");
    let (f, i) = (q(&doc, "#f"), q(&doc, "#i"));
    doc.mutate().set_form_value(i, &"a,".repeat(300_000));
    assert_eq!(doc.control_validity(i), Err(ValidationError));
    assert_eq!(doc.invalid_form_controls(f), Err(ValidationError));
    doc.mutate().set_form_value(i, "a@example.com");
    assert!(doc.control_validity(i).unwrap().valid());
}

#[test]
fn pattern_source_keeps_original_utf16_units() {
    let mut doc = parse("<input id=i value='�'>");
    let i = q(&doc, "#i");
    let source = blitz_dom::DomString::from_utf16(vec![0xd800]);
    doc.mutate()
        .set_attribute_by_name(i, "pattern", &source)
        .unwrap();
    assert_eq!(doc.attribute_by_name(i, "pattern").unwrap().value, source);
    // The scalar replacement character must not match a lone-surrogate source.
    assert!(doc.control_validity(i).unwrap().pattern_mismatch);
}
