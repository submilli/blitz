//! Output's default/live mode is owned by the document, including reset.
mod common;
use common::{parse, q};

#[test]
fn output_default_and_live_values_follow_text_and_reset() {
    let mut doc = parse("<form id=f><output id=o>initial</output></form>");
    let (form, output) = (q(&doc, "#f"), q(&doc, "#o"));
    assert_eq!(doc.output_value(output).unwrap().to_string(), "initial");
    doc.mutate()
        .set_output_value(output, "live".into())
        .unwrap();
    assert_eq!(doc.output_value(output).unwrap().to_string(), "live");
    assert_eq!(
        doc.output_default_value(output).unwrap().to_string(),
        "initial"
    );
    assert!(doc.attribute_by_name(output, "value").is_none());
    doc.mutate()
        .set_output_default_value(output, "default".into())
        .unwrap();
    assert_eq!(doc.output_value(output).unwrap().to_string(), "live");
    doc.mutate().reset_form_controls(form).unwrap();
    assert_eq!(doc.output_value(output).unwrap().to_string(), "default");
    doc.mutate().try_set_text_content(output, "edited").unwrap();
    assert_eq!(
        doc.output_default_value(output).unwrap().to_string(),
        "edited"
    );
}

#[test]
fn output_reset_uses_current_explicit_form_ownership() {
    let mut doc = parse("<form id=f></form><output id=o form=f>initial</output>");
    let (form, output) = (q(&doc, "#f"), q(&doc, "#o"));
    doc.mutate()
        .set_output_value(output, "live".into())
        .unwrap();
    doc.mutate().reset_form_controls(form).unwrap();
    assert_eq!(doc.output_value(output).unwrap().to_string(), "initial");
    doc.mutate()
        .set_output_value(output, "live".into())
        .unwrap();
    doc.mutate()
        .set_attribute_by_name(output, "form", "missing")
        .unwrap();
    doc.mutate().reset_form_controls(form).unwrap();
    assert_eq!(doc.output_value(output).unwrap().to_string(), "live");
}

#[test]
fn default_value_setter_updates_text_only_in_default_mode() {
    let mut doc = parse("<output id=o></output>");
    let output = q(&doc, "#o");
    doc.mutate()
        .set_output_default_value(output, "default".into())
        .unwrap();
    assert_eq!(doc.output_value(output).unwrap().to_string(), "default");
    doc.mutate()
        .set_output_value(output, "live".into())
        .unwrap();
    doc.mutate().try_set_text_content(output, "edited").unwrap();
    assert_eq!(
        doc.output_default_value(output).unwrap().to_string(),
        "default"
    );
    assert_eq!(doc.output_value(output).unwrap().to_string(), "edited");
}

#[test]
fn rejected_output_allocation_keeps_mode_and_can_retry() {
    use blitz_dom::{BaseDocument, DocumentConfig, QualName, ns};
    use blitz_html::DocumentHtmlParser;
    let mut doc = BaseDocument::new(DocumentConfig {
        node_limit: Some(32),
        ..Default::default()
    });
    DocumentHtmlParser::parse_into_mutator(
        &mut doc.mutate(),
        "<form id=f><output id=o>default</output></form>",
    );
    let (form, output) = (q(&doc, "#f"), q(&doc, "#o"));
    doc.mutate()
        .set_output_value(output, "live".into())
        .unwrap();
    let mut nodes = Vec::new();
    while let Ok(id) = doc
        .mutate()
        .try_create_element(QualName::new(None, ns!(html), "div".into()), vec![])
    {
        nodes.push(id);
    }
    assert!(
        doc.mutate()
            .set_output_value(output, "replacement".into())
            .is_err()
    );
    assert!(doc.mutate().reset_form_controls(form).is_err());
    assert_eq!(doc.output_value(output).unwrap().to_string(), "live");
    assert_eq!(
        doc.output_default_value(output).unwrap().to_string(),
        "default"
    );
    doc.mutate().remove_and_drop_node(nodes.pop().unwrap());
    doc.mutate().reset_form_controls(form).unwrap();
    assert_eq!(doc.output_value(output).unwrap().to_string(), "default");
    assert_eq!(
        doc.output_default_value(output).unwrap().to_string(),
        "default"
    );
}
