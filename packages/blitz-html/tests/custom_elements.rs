//! Custom element state (`:defined`) and reactions.

mod common;

use blitz_dom::custom_elements::{
    CustomElementReaction, CustomElementState, is_valid_custom_element_name,
};
use common::{parse, q};

fn matches(doc: &blitz_dom::BaseDocument, selector: &str) -> Vec<blitz_dom::NodeId> {
    doc.query_selector_all(selector)
        .unwrap()
        .into_iter()
        .collect()
}

#[test]
fn valid_custom_element_names() {
    for name in ["my-el", "a-", "x-1.2_3", "math-α"] {
        assert!(is_valid_custom_element_name(name), "{name}");
    }
    for name in [
        "div",
        "My-el",
        "my-El",
        "-x",
        "1-x",
        "font-face",
        "annotation-xml",
        "a b-c",
    ] {
        assert!(!is_valid_custom_element_name(name), "{name}");
    }
}

#[test]
fn defined_matches_built_ins_and_upgraded_custom_elements() {
    let mut doc = parse("<div id=d></div><my-el id=m></my-el><svg><my-el id=s></my-el></svg>");
    let (d, m, s) = (q(&doc, "#d"), q(&doc, "#m"), q(&doc, "#s"));
    assert_eq!(doc.custom_element_state(m), CustomElementState::Undefined);
    // Only HTML-namespace elements can be custom elements.
    assert_eq!(
        doc.custom_element_state(s),
        CustomElementState::Uncustomized
    );
    assert!(matches(&doc, ":defined").contains(&d));
    assert_eq!(matches(&doc, ":not(:defined)"), vec![m]);

    doc.set_custom_element_state(m, CustomElementState::Custom);
    assert!(matches(&doc, ":not(:defined)").is_empty());
    doc.set_custom_element_state(m, CustomElementState::Failed);
    assert_eq!(matches(&doc, ":not(:defined)"), vec![m]);
}

#[test]
fn defined_restyles() {
    let mut doc = parse(
        "<style>my-el { display: block; height: 10px } my-el:not(:defined) { display: none }</style><my-el id=m></my-el>",
    );
    let m = q(&doc, "#m");
    doc.resolve(0.0);
    assert_eq!(doc.get_node(m).unwrap().final_layout().size.height, 0.0);
    doc.set_custom_element_state(m, CustomElementState::Custom);
    doc.resolve(0.0);
    assert!(doc.get_node(m).unwrap().final_layout().size.height > 0.0);
}

#[test]
fn reactions_for_connection_and_attribute_changes() {
    let mut doc = parse("<div id=host></div><my-el id=m></my-el>");
    let (host, m) = (q(&doc, "#host"), q(&doc, "#m"));
    assert!(
        doc.take_custom_element_reactions().is_empty(),
        "off until enabled"
    );
    doc.set_custom_element_reactions(true);

    // Moving a custom element disconnects and reconnects it; built-in
    // elements produce nothing.
    let mut mutator = doc.mutate();
    mutator.pre_insert(m, host, None).unwrap();
    // Undefined elements get no attribute reactions (their upgrade replays
    // attributes instead)...
    mutator.set_attribute_by_name(m, "title", "one").unwrap();
    drop(mutator);
    // ...custom ones do.
    doc.set_custom_element_state(m, CustomElementState::Custom);
    let mut mutator = doc.mutate();
    mutator.set_attribute_by_name(m, "title", "two").unwrap();
    mutator.remove_attribute_by_name(m, "title");
    mutator.remove_child(host, m).unwrap();
    drop(mutator);

    let attr = |old: Option<&str>, new: Option<&str>| CustomElementReaction::AttributeChanged {
        element: m,
        name: "title".into(),
        namespace: None,
        old_value: old.map(Into::into),
        new_value: new.map(Into::into),
    };
    assert_eq!(
        doc.take_custom_element_reactions(),
        vec![
            CustomElementReaction::Disconnected(m),
            CustomElementReaction::Connected(m),
            attr(Some("one"), Some("two")),
            attr(Some("two"), None),
            CustomElementReaction::Disconnected(m),
        ]
    );
}
