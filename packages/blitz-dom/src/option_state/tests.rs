//! Normalized option text admission must count actual output, not raw subtrees.
use crate::{BaseDocument, DomString, NodeId, QualName, ns};

fn element(doc: &mut BaseDocument, tag: &str) -> NodeId {
    doc.mutate()
        .try_create_element(QualName::new(None, ns!(html), tag.into()), vec![])
        .expect("test document admits fixed element")
}
fn text(doc: &mut BaseDocument, parent: NodeId, text: impl Into<DomString>) -> NodeId {
    let text = doc
        .mutate()
        .try_create_text_node(text)
        .expect("test document admits text");
    doc.mutate()
        .pre_insert(text, parent, None)
        .expect("test text can be appended");
    text
}
#[test]
fn normalized_text_admission_checks_exact_output_including_pending_space() {
    let mut doc = BaseDocument::new(Default::default());
    let option = element(&mut doc, "option");
    text(&mut doc, option, " \tfirst\r\n");
    let span = element(&mut doc, "span");
    doc.mutate()
        .pre_insert(span, option, None)
        .expect("test span can be appended");
    text(&mut doc, span, " \u{000c}second \t");
    assert_eq!(
        doc.option_text_with_limit(option, 12)
            .unwrap()
            .as_str_lossy(),
        "first second"
    );
    assert!(doc.option_text_with_limit(option, 11).is_none());
    assert!(doc.option_text_with_limit(option, 5).is_none());
    assert_eq!(doc.option_text(option).as_str_lossy(), "first second");
}
#[test]
fn ignored_scripts_and_whitespace_admit_without_raw_subtree_overrejection() {
    let mut doc = BaseDocument::new(Default::default());
    let option = element(&mut doc, "option");
    text(&mut doc, option, " \t\r\n".repeat(30_000));
    let script = element(&mut doc, "script");
    doc.mutate()
        .pre_insert(script, option, None)
        .expect("test script can be appended");
    text(&mut doc, script, "ignored".repeat(30_000));
    assert!(doc.option_text_with_limit(option, 0).unwrap().is_empty());
    text(&mut doc, option, "x  ");
    assert_eq!(
        doc.option_text_with_limit(option, 1)
            .unwrap()
            .as_str_lossy(),
        "x"
    );
    assert!(doc.option_text_with_limit(option, 0).is_none());
}
#[test]
fn surrogate_pairs_and_lone_surrogates_are_admitted_as_code_units() {
    let mut doc = BaseDocument::new(Default::default());
    let option = element(&mut doc, "option");
    text(
        &mut doc,
        option,
        DomString::from_utf16(vec![0xd800, 0x20, 0xd83d]),
    );
    text(
        &mut doc,
        option,
        DomString::from_utf16(vec![0xde00, 0x20, 0xdc00]),
    );
    let expected = [0xd800, 0x20, 0xd83d, 0xde00, 0x20, 0xdc00];
    assert_eq!(
        &*doc.option_text_with_limit(option, 6).unwrap().to_utf16(),
        &expected
    );
    assert!(doc.option_text_with_limit(option, 5).is_none());
    assert_eq!(&*doc.option_text(option).to_utf16(), &expected);
}
#[test]
fn zero_and_large_limits_handle_empty_and_non_html_space() {
    let mut doc = BaseDocument::new(Default::default());
    let option = element(&mut doc, "option");
    assert!(doc.option_text_with_limit(option, 0).unwrap().is_empty());
    text(&mut doc, option, "\u{a0} \u{2003}");
    assert_eq!(
        doc.option_text_with_limit(option, usize::MAX)
            .unwrap()
            .as_str_lossy(),
        "\u{a0} \u{2003}"
    );
    assert!(doc.option_text_with_limit(option, 2).is_none());
    assert_eq!(
        doc.option_text_with_limit(option, 3)
            .unwrap()
            .as_str_lossy(),
        "\u{a0} \u{2003}"
    );
}

#[test]
fn html_aware_text_excludes_svg_scripts_and_image_descendants() {
    let mut doc = BaseDocument::new(Default::default());
    let option = element(&mut doc, "option");
    text(&mut doc, option, "a");
    let svg_script = doc
        .mutate()
        .try_create_element(QualName::new(None, ns!(svg), "script".into()), vec![])
        .expect("test SVG script admission");
    let image = element(&mut doc, "img");
    doc.mutate()
        .pre_insert(svg_script, option, None)
        .expect("test SVG script placement");
    doc.mutate()
        .pre_insert(image, option, None)
        .expect("test image placement");
    text(&mut doc, svg_script, "ignored SVG");
    text(&mut doc, image, "ignored image child");
    text(&mut doc, option, "b");
    assert_eq!(
        doc.option_text_with_limit(option, 2)
            .expect("exact normalized limit")
            .as_str_lossy(),
        "ab"
    );
    assert_eq!(doc.option_text(option).as_str_lossy(), "ab");
    assert!(doc.option_text_with_limit(option, 1).is_none());
}

fn select_with_values(doc: &mut BaseDocument, values: &[DomString]) -> (NodeId, Vec<NodeId>) {
    let select = element(doc, "select");
    let mut options = Vec::new();
    for value in values {
        let option = element(doc, "option");
        doc.mutate()
            .set_attribute(option, QualName::new(None, ns!(), "value".into()), value);
        doc.mutate()
            .pre_insert(option, select, None)
            .expect("test option placement");
        options.push(option);
    }
    (select, options)
}

#[test]
fn scalar_select_values_do_not_alias_isolated_surrogates() {
    let mut doc = BaseDocument::new(Default::default());
    let (select, options) = select_with_values(
        &mut doc,
        &[
            DomString::from_utf16(vec![0xd800]),
            DomString::from("\u{fffd}"),
        ],
    );
    doc.mutate().set_select_value(select, "\u{fffd}");
    assert_eq!(doc.selected_option(select), Some(options[1]));
    assert!(!doc.option_selectedness(options[0]));
}

#[test]
fn dom_select_values_match_exact_utf16_and_only_the_first_option() {
    let mut doc = BaseDocument::new(Default::default());
    let high = DomString::from_utf16(vec![0xd800]);
    let low = DomString::from_utf16(vec![0xdc00]);
    let (select, options) = select_with_values(
        &mut doc,
        &[
            high.clone(),
            low.clone(),
            high.clone(),
            DomString::from("\u{fffd}"),
        ],
    );
    doc.mutate().set_attribute(
        select,
        QualName::new(None, ns!(), "multiple".into()),
        &DomString::from(""),
    );
    doc.mutate().set_select_value_dom(select, &low);
    assert_eq!(doc.selected_options(select), vec![options[1]]);
    doc.mutate().set_select_value_dom(select, &high);
    assert_eq!(doc.selected_options(select), vec![options[0]]);
    assert_eq!(doc.form_value_dom(select).as_ref(), Some(&high));
    doc.mutate()
        .set_select_value_dom(select, &DomString::from_utf16(vec![0xd801]));
    assert!(doc.selected_options(select).is_empty());
}
