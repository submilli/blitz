//! Form state is UTF-16 even while its rendering editor is scalar text.
mod common;
use blitz_dom::DomString;
use common::{parse, q};

#[test]
fn input_values_sanitize_without_replacing_surrogates() {
    let mut doc = parse("<form id=f><input id=i></form>");
    let i = q(&doc, "#i");
    let f = q(&doc, "#f");
    let value = DomString::from_utf16(vec![0xd800, 13, 10, 0xdc00]);
    doc.mutate().set_form_value_dom(i, &value);
    assert_eq!(
        doc.form_value_dom(i).unwrap().to_utf16().as_ref(),
        [0xd800, 0xdc00]
    );
    doc.mutate()
        .set_attribute_by_name(i, "value", DomString::from_utf16(vec![0xdfff]))
        .unwrap();
    let clone = doc.mutate().clone_node(i, false);
    assert_eq!(doc.form_value_dom(clone), doc.form_value_dom(i));
    doc.mutate().reset_form_controls(f).unwrap();
    assert_eq!(doc.form_value_dom(i).unwrap().to_utf16().as_ref(), [0xdfff]);
    doc.resolve(0.0);
    assert_eq!(doc.form_value_dom(i).unwrap().to_utf16().as_ref(), [0xdfff]);
}

#[test]
fn textarea_pristine_clones_and_direct_child_defaults_remain_distinct() {
    let mut doc = parse("<textarea id=t>default</textarea>");
    let t = q(&doc, "#t");
    let shallow = doc.mutate().clone_node(t, false);
    assert_eq!(doc.form_value(shallow).as_deref(), Some("default"));
    let child = doc
        .mutate()
        .create_text_node(DomString::from_utf16(vec![0xd800]));
    doc.mutate().append_children(shallow, &[child]);
    assert_eq!(
        doc.form_value_dom(shallow).unwrap().to_utf16().as_ref(),
        [0xd800]
    );
    doc.mutate()
        .set_node_text(child, DomString::from_utf16(vec![0xdc00]));
    assert_eq!(
        doc.form_value_dom(shallow).unwrap().to_utf16().as_ref(),
        [0xdc00]
    );
    doc.mutate()
        .set_form_value_dom(shallow, &DomString::from_utf16(vec![0xdfff]));
    doc.mutate().set_node_text(child, "new default");
    assert_eq!(
        doc.form_value_dom(shallow).unwrap().to_utf16().as_ref(),
        [0xdfff]
    );
    let dirty_clone = doc.mutate().clone_node(shallow, true);
    assert_eq!(doc.form_value_dom(dirty_clone), doc.form_value_dom(shallow));
}

#[test]
fn native_replacements_preserve_code_units_and_refresh_joined_pairs() {
    use blitz_dom::{EventDriver, NoopEventHandler};
    use blitz_traits::events::{BlitzImeEvent, UiEvent};
    let mut doc = parse("<input id=i>");
    *doc.font_ctx().lock().unwrap() = blitz_dom::build_single_font_ctx(include_bytes!(
        "../../blitz-dom/assets/test-fonts/LiberationSans-Regular.ttf"
    ));
    let i = q(&doc, "#i");
    doc.mutate()
        .set_form_value_dom(i, &DomString::from_utf16(vec![0xd800, 120, 0xdc00]));
    doc.resolve(0.0);
    doc.set_focus_to(i);
    for command in [
        "moveToBeginningOfDocument:",
        "moveForward:",
        "deleteForward:",
    ] {
        EventDriver::new(&mut doc, NoopEventHandler)
            .handle_ui_event(UiEvent::AppleStandardKeybinding(command.into()));
    }
    assert_eq!(
        doc.form_value_dom(i).unwrap().to_utf16().as_ref(),
        [0xd800, 0xdc00]
    );
    EventDriver::new(&mut doc, NoopEventHandler)
        .handle_ui_event(UiEvent::Ime(BlitzImeEvent::Commit("!".into())));
    assert_eq!(
        doc.form_value_dom(i).unwrap().to_utf16().as_ref(),
        [0xd800, 0xdc00, 33]
    );
    for event in [
        BlitzImeEvent::Preedit("abc".into(), Some((3, 3))),
        BlitzImeEvent::Preedit("d".into(), Some((1, 1))),
        BlitzImeEvent::Commit("e".into()),
    ] {
        EventDriver::new(&mut doc, NoopEventHandler).handle_ui_event(UiEvent::Ime(event));
    }
    assert_eq!(
        doc.form_value_dom(i).unwrap().to_utf16().as_ref(),
        [0xd800, 0xdc00, 33, 101]
    );
}

#[test]
fn lossless_identity_keys_and_null_namespace_attributes_drive_form_algorithms() {
    let mut doc = parse(
        "<form id=f></form><form id=g></form><input id=i><input id=a type=radio><input id=b type=radio>",
    );
    let [f, g, i, a, b] = ["#f", "#g", "#i", "#a", "#b"].map(|s| q(&doc, s));
    doc.mutate()
        .set_attribute_by_name(f, "id", DomString::from_utf16(vec![0xd800]))
        .unwrap();
    doc.mutate().set_attribute_by_name(g, "id", "�").unwrap();
    doc.mutate().set_attribute_by_name(i, "form", "�").unwrap();
    assert_eq!(doc.form_owner(i), Some(g));
    assert!(!doc.form_controls(f).contains(&i));
    assert!(doc.form_controls(g).contains(&i));
    doc.mutate().append_children(g, &[a, b]);
    doc.mutate()
        .set_attribute_by_name(a, "name", DomString::from_utf16(vec![0xd800]))
        .unwrap();
    doc.mutate().set_attribute_by_name(b, "name", "�").unwrap();
    doc.mutate()
        .set_attribute_by_name(a, "required", "")
        .unwrap();
    doc.mutate().set_checkedness(b, true);
    assert!(doc.control_validity(a).unwrap().value_missing);
    doc.mutate().set_checkedness(a, true);
    assert!(doc.checkedness(a) && doc.checkedness(b));
    let required = blitz_dom::QualName::new(Some("x".into()), "urn:x".into(), "required".into());
    doc.mutate().set_attribute(i, required, "");
    assert!(!doc.control_validity(i).unwrap().value_missing);
}

fn foreign_attribute(name: &str, prefixed: bool) -> blitz_dom::QualName {
    blitz_dom::QualName::new(prefixed.then(|| "x".into()), "urn:x".into(), name.into())
}

#[test]
fn foreign_id_mutations_preserve_native_id_index() {
    for prefixed in [false, true] {
        let mut doc = parse("<input id=real>");
        let input = q(&doc, "#real");
        let name = foreign_attribute("id", prefixed);
        doc.mutate().set_attribute(input, name.clone(), "foreign");
        assert_eq!(doc.get_element_by_id("real"), Some(input));
        assert_eq!(doc.get_element_by_id("foreign"), None);
        doc.mutate().clear_attribute(input, name);
        assert_eq!(doc.get_element_by_id("real"), Some(input));
        assert_eq!(doc.get_element_by_id("foreign"), None);
    }
}

#[test]
fn prefixless_foreign_attributes_do_not_participate_in_form_algorithms() {
    let mut doc = parse(
        "<form id=f><input id=i><input id=r type=radio><select id=s><option id=a>A</option><option id=b>B</option></select></form><form id=g></form>",
    );
    let [f, g, i, r, s, a, b] = ["#f", "#g", "#i", "#r", "#s", "#a", "#b"].map(|s| q(&doc, s));
    for (id, name, value) in [
        (i, "form", "g"),
        (i, "required", ""),
        (r, "required", ""),
        (r, "checked", ""),
        (i, "value", "foreign"),
        (s, "multiple", ""),
        (b, "selected", ""),
    ] {
        doc.mutate()
            .set_attribute(id, foreign_attribute(name, false), value);
    }
    assert_eq!(doc.attribute_by_name(i, "form").unwrap().value, "g");
    assert!(doc.null_attribute(i, "form").is_none());
    assert_eq!(doc.form_owner(i), Some(f));
    assert!(doc.form_controls(f).contains(&i));
    assert!(!doc.form_controls(g).contains(&i));
    assert!(doc.invalid_form_controls(f).unwrap().is_empty());
    doc.mutate().set_form_value(i, "changed");
    doc.mutate().set_checkedness(r, true);
    doc.mutate().reset_form_controls(f).unwrap();
    assert_eq!(doc.form_value(i).as_deref(), Some(""));
    assert!(!doc.checkedness(r));
    assert_eq!(doc.selected_options(s), vec![a]);
    assert_eq!(doc.get_node(i).unwrap().attr("value".into()), None);
}

#[test]
fn reflected_value_modes_write_empty_namespace_attributes() {
    let mut doc = parse("<input id=i type=hidden><input id=t>");
    let [i, t] = ["#i", "#t"].map(|s| q(&doc, s));
    for id in [i, t] {
        doc.mutate()
            .set_attribute(id, foreign_attribute("value", false), "foreign");
        doc.mutate().set_form_value(id, "live");
    }
    doc.mutate()
        .set_attribute_by_name(t, "type", "hidden")
        .unwrap();
    for id in [i, t] {
        assert_eq!(doc.null_attribute(id, "value").unwrap().value, "live");
        assert_eq!(doc.attribute_by_name(id, "value").unwrap().value, "foreign");
        assert_eq!(doc.form_value(id).as_deref(), Some("live"));
    }
}
