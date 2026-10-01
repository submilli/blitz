//! Disabledness follows current ancestry, independently of attribute reflection.
mod common;
use common::{parse, q};

#[test]
fn fieldset_uses_only_first_legend_and_checks_outer_fieldsets() {
    let mut doc = parse(
        "<fieldset id=f disabled><legend id=l><input id=a><fieldset disabled><input id=b></fieldset></legend><input id=c><legend><input id=d></legend></fieldset><div id=e disabled></div>",
    );
    let ids = ["#a", "#b", "#c", "#d", "#e"].map(|s| q(&doc, s));
    for (id, expected) in ids.into_iter().zip([false, true, true, true, false]) {
        assert_eq!(doc.get_node(id).unwrap().is_disabled(), expected);
    }
    let f = q(&doc, "#f");
    let l = q(&doc, "#l");
    doc.mutate().pre_insert(ids[2], l, None).unwrap();
    assert!(!doc.get_node(ids[2]).unwrap().is_disabled());
    doc.mutate().remove_attribute_by_name(f, "disabled");
    assert!(!doc.get_node(ids[3]).unwrap().is_disabled());
    assert!(doc.get_node(ids[1]).unwrap().is_disabled());
}

#[test]
fn option_rules_do_not_inherit_select_or_fieldset_disabledness() {
    let mut doc = parse(
        "<fieldset disabled><select disabled><option id=a>A</option><optgroup id=g disabled><option id=b>B</option></optgroup></select></fieldset>",
    );
    let (a, b, g) = (q(&doc, "#a"), q(&doc, "#b"), q(&doc, "#g"));
    assert!(!doc.get_node(a).unwrap().is_disabled());
    assert!(doc.get_node(b).unwrap().is_disabled());
    doc.mutate().remove_attribute_by_name(g, "disabled");
    assert!(!doc.get_node(b).unwrap().is_disabled());
    assert!(!doc.get_node(g).unwrap().is_disabled());
}

#[test]
fn selector_state_follows_legend_reordering_and_attribute_removal() {
    let mut doc = parse(
        "<fieldset id=f disabled><legend id=first><input id=a></legend><legend id=second><input id=b></legend></fieldset>",
    );
    let (f, first, second, a, b) = (
        q(&doc, "#f"),
        q(&doc, "#first"),
        q(&doc, "#second"),
        q(&doc, "#a"),
        q(&doc, "#b"),
    );
    assert_eq!(q(&doc, "input:disabled"), b);
    doc.mutate().pre_insert(second, f, Some(first)).unwrap();
    assert_eq!(q(&doc, "input:disabled"), a);
    assert_eq!(q(&doc, "input:enabled"), b);
    doc.mutate().remove_attribute_by_name(f, "disabled");
    assert!(doc.query_selector("input:disabled").unwrap().is_none());
}
