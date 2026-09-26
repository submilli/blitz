//! Mutation records (the data behind `MutationObserver`).

mod common;

use blitz_dom::mutations::MutationRecord;
use common::{parse, q};

#[test]
fn nothing_is_recorded_until_recording_is_on() {
    let mut doc = parse("<div id=a></div>");
    let a = q(&doc, "#a");
    doc.mutate().set_attribute_by_name(a, "title", "x").unwrap();
    assert!(doc.take_mutation_records().is_empty());
    assert!(!doc.is_recording_mutations());
}

#[test]
fn child_list_attribute_and_character_data_records() {
    let mut doc = parse("<div id=a><p id=p>text</p></div>");
    let (a, p) = (q(&doc, "#a"), q(&doc, "#p"));
    let text = doc.get_node(p).unwrap().children[0];
    doc.set_mutation_recording(true);

    let mut m = doc.mutate();
    m.set_attribute_by_name(a, "title", "one").unwrap();
    m.set_attribute_by_name(a, "title", "two").unwrap();
    let span = m.create_text_node("s");
    m.pre_insert(span, a, None).unwrap();
    m.remove_child(a, p).unwrap();
    m.set_character_data(text, "changed");
    drop(m);

    let records = doc.take_mutation_records();
    assert_eq!(
        records,
        vec![
            MutationRecord::Attributes { target: a, name: "title".into(), namespace: None, old_value: None },
            MutationRecord::Attributes { target: a, name: "title".into(), namespace: None, old_value: Some("one".into()) },
            MutationRecord::ChildList { target: a, added: vec![span], removed: vec![], previous_sibling: Some(p), next_sibling: None },
            MutationRecord::ChildList { target: a, added: vec![], removed: vec![p], previous_sibling: None, next_sibling: Some(span) },
            MutationRecord::CharacterData { target: text, old_value: "text".into() },
        ]
    );
    assert!(doc.take_mutation_records().is_empty(), "records are drained");
}

#[test]
fn moving_a_node_records_its_removal_and_insertion() {
    let mut doc = parse("<div id=a><b id=b></b></div><div id=c></div>");
    let (a, b, c) = (q(&doc, "#a"), q(&doc, "#b"), q(&doc, "#c"));
    doc.set_mutation_recording(true);
    doc.mutate().pre_insert(b, c, None).unwrap();
    let records = doc.take_mutation_records();
    assert_eq!(records.len(), 2);
    assert!(matches!(&records[0], MutationRecord::ChildList { target, removed, .. } if *target == a && removed == &vec![b]));
    assert!(matches!(&records[1], MutationRecord::ChildList { target, added, .. } if *target == c && added == &vec![b]));
}
