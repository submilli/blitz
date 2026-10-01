//! Native selection state is shared by selectors, form values and mutations.
mod common;
use common::{parse, q};

#[test]
fn default_selection_and_no_selection_are_real_state() {
    let mut doc = parse("<select id=s><option id=a>a</option><option id=b>b</option></select>");
    let (s, a, b) = (q(&doc, "#s"), q(&doc, "#a"), q(&doc, "#b"));
    assert_eq!(doc.selected_options(s), vec![a]);
    doc.mutate().set_select_index(s, -1);
    assert!(doc.selected_options(s).is_empty());
    assert_eq!(doc.form_value(s), None);
    doc.mutate().set_option_selectedness(b, true);
    assert_eq!(doc.selected_options(s), vec![b]);
    assert!(doc.attribute_by_name(b, "selected").is_none());
    assert_eq!(doc.query_selector_in(s, ":checked").unwrap(), Some(b));
}

#[test]
fn dirty_selection_does_not_follow_defaults_and_reset_clears_dirty() {
    let mut doc = parse(
        "<form id=f><select id=s><option id=a>a</option><option id=b selected>b</option></select></form>",
    );
    let (f, s, a, b) = (q(&doc, "#f"), q(&doc, "#s"), q(&doc, "#a"), q(&doc, "#b"));
    doc.mutate().set_option_selectedness(b, false);
    doc.mutate()
        .set_attribute_by_name(b, "selected", "")
        .unwrap();
    assert_eq!(doc.selected_options(s), vec![a]);
    doc.mutate().reset_form_controls(f).unwrap();
    assert_eq!(doc.selected_options(s), vec![b]);
    doc.mutate().remove_attribute_by_name(b, "selected");
    assert_eq!(doc.selected_options(s), vec![a]);
}

#[test]
fn insertion_removal_and_move_reconcile_owners() {
    let mut doc = parse(
        "<select id=s><option id=a>a</option></select><select id=t><option id=b selected>b</option></select>",
    );
    let (s, t, a, b) = (q(&doc, "#s"), q(&doc, "#t"), q(&doc, "#a"), q(&doc, "#b"));
    doc.mutate().pre_insert(b, s, Some(a)).unwrap();
    assert!(doc.selected_options(t).is_empty());
    assert_eq!(doc.selected_options(s), vec![b]);
    doc.mutate().remove_node(b);
    assert!(doc.option_selectedness(b));
    assert_eq!(doc.selected_options(s), vec![a]);
}

#[test]
fn duplicate_values_choose_one_even_when_multiple() {
    let mut doc = parse(
        "<select id=s multiple><option id=a value=x>a</option><option id=b value=x>b</option></select>",
    );
    let (s, a, b) = (q(&doc, "#s"), q(&doc, "#a"), q(&doc, "#b"));
    doc.mutate().set_option_selectedness(a, true);
    doc.mutate().set_option_selectedness(b, true);
    assert_eq!(doc.selected_options(s), vec![a, b]);
    doc.mutate().set_select_value(s, "x");
    assert_eq!(doc.selected_options(s), vec![a]);
    doc.mutate().set_select_value(s, "missing");
    assert!(doc.selected_options(s).is_empty());
}

#[test]
fn disabled_options_are_skipped_by_default_but_can_be_selected() {
    let mut doc = parse(
        "<select id=s><optgroup disabled><option id=a>a</option></optgroup><option id=b disabled>b</option><option id=c>c</option></select>",
    );
    let (s, a, c) = (q(&doc, "#s"), q(&doc, "#a"), q(&doc, "#c"));
    assert_eq!(doc.selected_options(s), vec![c]);
    doc.mutate().set_select_index(s, 0);
    assert_eq!(doc.selected_options(s), vec![a]);
}

#[test]
fn option_text_uses_html_space_and_excludes_scripts() {
    let mut doc = parse("<div id=o></div>");
    let o = q(&doc, "#o");
    let text = blitz_dom::DomString::from_utf16(vec![32, 0xd800, 11, 32, 32, 120, 32]);
    doc.mutate().set_text_content(o, &text);
    assert_eq!(&*doc.option_text(o).to_utf16(), &[0xd800, 11, 32, 120]);
}

#[test]
fn parsed_selected_options_match_checked() {
    let doc = parse("<select multiple><option id=a selected>a</option></select>");
    assert_eq!(q(&doc, ":checked"), q(&doc, "#a"));
}

#[test]
fn reset_radios_batches_groups_and_clears_every_dirty_flag() {
    let html = format!(
        "<form id=f>{}</form>",
        (0..1024)
            .map(|i| format!("<input type=RADIO name=g checked id=r{i}>"))
            .collect::<String>()
    );
    let mut doc = parse(&html);
    let form = q(&doc, "#f");
    doc.mutate().reset_form_controls(form).unwrap();
    let radios = doc.query_selector_all("input").unwrap();
    assert_eq!(radios.iter().filter(|&&id| doc.checkedness(id)).count(), 1);
    assert!(doc.checkedness(*radios.last().unwrap()));
    for id in radios {
        assert!(
            !doc.get_node(id)
                .unwrap()
                .element_data()
                .unwrap()
                .form_state
                .checked_dirty
        );
    }
}

#[test]
fn fragment_context_does_not_select_temporary_options() {
    let mut doc = parse("<select id=s multiple></select>");
    let s = q(&doc, "#s");
    doc.mutate()
        .set_inner_html_detaching(s, "<option selected>a</option><option selected>b</option>");
    assert_eq!(doc.selected_options(s).len(), 2);
    doc.mutate().remove_attribute_by_name(s, "multiple");
    doc.mutate().set_select_index(s, -1);
    doc.mutate()
        .insert_adjacent_html(s, "beforeend", "<option>c</option>")
        .unwrap();
    assert_eq!(doc.form_value(s).as_deref(), Some("a"));
}

#[test]
fn native_clone_updates_selection_ownership() {
    let mut doc =
        parse("<select id=s><option id=a>a</option><option id=b selected>b</option></select>");
    let original = q(&doc, "#s");
    let copy = doc.deep_clone_node(original);
    let options = doc.select_options(copy);
    assert_eq!(doc.option_select(options[0]), Some(copy));
    doc.mutate().set_option_selectedness(options[0], true);
    assert_eq!(doc.selected_options(copy), vec![options[0]]);
    assert_eq!(doc.form_value(original).as_deref(), Some("b"));
}

#[test]
fn fallback_carries_disabled_groups_through_deep_subtrees() {
    let mut doc = parse("<select id=s></select><optgroup id=g disabled></optgroup>");
    let (s, g) = (q(&doc, "#s"), q(&doc, "#g"));
    let mut parent = g;
    for _ in 0..1024 {
        let child = doc.mutate().create_element(
            blitz_dom::QualName::new(None, blitz_dom::ns!(html), "div".into()),
            Vec::new(),
        );
        doc.mutate().append_children(parent, &[child]);
        parent = child;
    }
    for _ in 0..1024 {
        let child = doc.mutate().create_element(
            blitz_dom::QualName::new(None, blitz_dom::ns!(html), "option".into()),
            Vec::new(),
        );
        doc.mutate().append_children(parent, &[child]);
    }
    doc.mutate().append_children(s, &[g]);
    assert_eq!(doc.select_options(s).len(), 1024);
    assert!(doc.selected_options(s).is_empty());
}
