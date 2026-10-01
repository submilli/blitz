//! Parser-only associations expire on the same mutations as HTML form owners.
mod common;
use common::{parse, q};

const TABLE: &str = "<table id=t><form id=f><tr><td><input id=c name=c value=x></table>";

#[test]
fn parser_owner_and_batch_survive_unrelated_mutation() {
    let mut doc = parse(TABLE);
    let (form, control) = (q(&doc, "#f"), q(&doc, "#c"));
    assert_eq!(doc.form_owner(control), Some(form));
    assert_eq!(doc.form_controls(form), vec![control]);
    doc.mutate()
        .set_attribute_by_name(control, "class", "test")
        .unwrap();
    doc.mutate()
        .set_attribute_by_name(form, "id", "renamed")
        .unwrap();
    assert_eq!(doc.form_owner(control), Some(form));
    assert_eq!(doc.form_controls(form), vec![control]);
}

#[test]
fn shared_subtree_moves_preserve_owners_but_control_branch_moves_reset_them() {
    let mut doc = parse(TABLE);
    let (form, control, table) = (q(&doc, "#f"), q(&doc, "#c"), q(&doc, "#t"));
    let body = q(&doc, "body");
    doc.mutate().append_children(body, &[table]);
    assert_eq!(doc.form_owner(control), Some(form));
    let cell = q(&doc, "td");
    doc.mutate().append_children(body, &[cell]);
    assert_eq!(doc.form_owner(control), None);
    assert!(doc.form_controls(form).is_empty());
}

#[test]
fn removing_and_reinserting_owner_does_not_restore_parser_association() {
    let mut doc = parse(TABLE);
    let (form, control, table) = (q(&doc, "#f"), q(&doc, "#c"), q(&doc, "#t"));
    doc.mutate().remove_node(form);
    assert_eq!(doc.form_owner(control), None);
    doc.mutate().append_children(table, &[form]);
    assert_eq!(doc.form_owner(control), None);
    assert!(doc.form_controls(form).is_empty());
}

#[test]
fn explicit_form_attribute_replaces_parser_owner_and_removal_resets_it() {
    let mut doc = parse(TABLE);
    let (form, control) = (q(&doc, "#f"), q(&doc, "#c"));
    doc.mutate()
        .set_attribute_by_name(control, "form", "f")
        .unwrap();
    assert_eq!(doc.form_owner(control), Some(form));
    doc.mutate().remove_attribute_by_name(control, "form");
    assert_eq!(doc.form_owner(control), None);
}

#[test]
fn fragment_reparenting_resets_parser_magic() {
    let mut doc = parse("<form id=a><div id=b></div></form>");
    let container = q(&doc, "#b");
    doc.mutate().set_inner_html(
        container,
        "<table><tr><td></form><form id=c><input id=d></table><input id=e>",
    );
    assert_eq!(doc.form_owner(q(&doc, "#d")), Some(q(&doc, "#c")));
    assert_eq!(doc.form_owner(q(&doc, "#e")), Some(q(&doc, "#a")));
}

#[test]
fn image_form_attributes_do_not_reset_legacy_association() {
    let mut doc = parse("<table><form id=f><tr><td><img id=i></table>");
    let (form, image) = (q(&doc, "#f"), q(&doc, "#i"));
    assert_eq!(doc.form_owner(image), Some(form));
    doc.mutate()
        .set_attribute_by_name(image, "form", "ignored")
        .unwrap();
    assert_eq!(doc.form_owner(image), Some(form));
    doc.mutate().remove_attribute_by_name(image, "form");
    assert_eq!(doc.form_owner(image), Some(form));
}
