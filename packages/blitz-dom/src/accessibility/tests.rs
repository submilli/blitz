use super::*;
use crate::{DocumentConfig, QualName, local_name, ns};
fn document() -> BaseDocument {
    BaseDocument::new(DocumentConfig::default())
}
fn append(doc: &mut BaseDocument, tag: &str, text: &str) -> crate::NodeId {
    let id = doc
        .mutate()
        .create_element(QualName::new(None, ns!(html), tag.into()), vec![]);
    let root = doc.root_node().id;
    doc.mutate().append_children(root, &[id]);
    doc.mutate().set_text_content(id, text);
    id
}
#[test]
fn oversized_names_are_rejected_before_tree_output() {
    let mut doc = document();
    let button = append(&mut doc, "button", "");
    doc.mutate().set_attribute(
        button,
        QualName::new(None, ns!(), local_name!("aria-label")),
        "x".repeat(4096),
    );
    assert!(
        doc.try_build_accessibility_tree(&AccessibilityBudget::new(100_000, 1024))
            .is_err()
    );
}
#[test]
fn repeated_references_share_one_work_and_text_budget() {
    let mut doc = document();
    let label = append(&mut doc, "span", "A visible name");
    doc.mutate()
        .set_attribute(label, QualName::new(None, ns!(), local_name!("id")), "name");
    let button = append(&mut doc, "button", "");
    doc.mutate().set_attribute(
        button,
        QualName::new(None, ns!(), local_name!("aria-labelledby")),
        "name ".repeat(100),
    );
    assert!(
        doc.try_build_accessibility_tree(&AccessibilityBudget::new(100_000, 1024))
            .is_err()
    );
    assert!(
        doc.try_build_accessibility_tree(&AccessibilityBudget::new(100, 100_000))
            .is_err()
    );
}
#[test]
fn deep_name_walks_refuse_without_stack_overflow() {
    let mut doc = document();
    let mut parent = append(&mut doc, "button", "");
    for _ in 0..140 {
        let child = doc
            .mutate()
            .create_element(QualName::new(None, ns!(html), "span".into()), vec![]);
        doc.mutate().append_children(parent, &[child]);
        parent = child;
    }
    doc.mutate().set_text_content(parent, "Deep");
    assert!(
        doc.try_build_accessibility_tree(&AccessibilityBudget::new(100_000, 100_000))
            .is_err()
    );
}
#[test]
fn ordinary_names_and_remaining_budget_are_preserved() {
    let mut doc = document();
    let button = append(&mut doc, "button", "Continue");
    let budget = AccessibilityBudget::new(100_000, 100_000);
    let tree = doc.try_build_accessibility_tree(&budget).unwrap();
    assert!(
        tree.nodes
            .iter()
            .any(|(id, node)| id.0 == button.as_u64() && node.label() == Some("Continue"))
    );
    let remaining = budget.remaining();
    assert!(remaining.0 < 100_000 && remaining.1 < 100_000);
}

#[test]
fn link_base_refusal_is_a_tree_failure() {
    let mut doc = BaseDocument::new(DocumentConfig {
        base_url: Some(format!("https://example.test/{}", "x".repeat(4096))),
        ..DocumentConfig::default()
    });
    let link = append(&mut doc, "a", "Link");
    doc.mutate()
        .set_attribute(link, QualName::new(None, ns!(), "href".into()), "x");
    assert!(
        doc.try_build_accessibility_tree(&AccessibilityBudget::new(100_000, 1024))
            .is_err()
    );
}
#[test]
fn empty_references_are_labels_without_full_name_fallback() {
    let mut doc = document();
    let name = append(&mut doc, "span", "");
    doc.mutate()
        .set_attribute(name, QualName::new(None, ns!(), "id".into()), "n");
    let input = append(&mut doc, "input", "");
    for (key, value) in [
        ("aria-labelledby", "n"),
        ("aria-label", "Old"),
        ("placeholder", "NotLabel"),
        ("title", "NotLabel"),
    ] {
        doc.mutate()
            .set_attribute(input, QualName::new(None, ns!(), key.into()), value);
    }
    assert_eq!(
        doc.try_accessibility_labels(input, &AccessibilityBudget::new(100_000, 100_000))
            .unwrap(),
        [""]
    );
}
#[test]
fn current_control_values_are_admitted_before_cloning() {
    let mut doc = document();
    let input = append(&mut doc, "input", "");
    doc.mutate().set_form_value(input, &"x".repeat(4096));
    assert!(
        doc.try_build_accessibility_tree(&AccessibilityBudget::new(100_000, 1024))
            .is_err()
    );
}

#[test]
fn label_association_admits_compared_attribute_bytes() {
    let mut doc = document();
    let label = append(&mut doc, "label", "Unrelated");
    doc.mutate().set_attribute(
        label,
        QualName::new(None, ns!(), "for".into()),
        "x".repeat(4096),
    );
    let input = append(&mut doc, "input", "");
    doc.mutate()
        .set_attribute(input, QualName::new(None, ns!(), "id".into()), "i");
    assert!(
        doc.try_accessibility_labels(input, &AccessibilityBudget::new(100_000, 1024))
            .is_err()
    );
}

#[test]
fn shadow_label_association_uses_the_same_admitted_tree() {
    let mut doc = document();
    let host = append(&mut doc, "div", "");
    let root = doc
        .mutate()
        .attach_shadow(host, crate::shadow::ShadowRootInit::default())
        .unwrap();
    let label = append(&mut doc, "label", "Shadow label");
    let input = append(&mut doc, "input", "");
    doc.mutate().append_children(root, &[label, input]);
    doc.mutate().set_attribute(
        label,
        QualName::new(None, ns!(), "for".into()),
        "x".repeat(4096),
    );
    doc.mutate()
        .set_attribute(input, QualName::new(None, ns!(), "id".into()), "i");
    assert!(
        doc.try_accessibility_labels(input, &AccessibilityBudget::new(100_000, 1024))
            .is_err()
    );
}
