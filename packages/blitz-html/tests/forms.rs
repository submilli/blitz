//! Form control value and checkedness (the HTML dirty value/checkedness
//! model), and the `input`/`change` events of checkable inputs.

mod common;

use blitz_dom::{Document, EventDriver, EventHandler, NodeId};
use blitz_traits::events::{DomEvent, EventState};
use common::{parse, q};
use keyboard_types::Modifiers;

#[test]
fn value_defaults_to_the_attribute_until_dirty() {
    let mut doc = parse("<input id=i value=default><textarea id=t>text</textarea>");
    let (i, t) = (q(&doc, "#i"), q(&doc, "#t"));
    assert_eq!(doc.form_value(i).as_deref(), Some("default"));
    assert_eq!(doc.form_value(t).as_deref(), Some("text"));

    doc.mutate().set_form_value(i, "typed");
    assert_eq!(doc.form_value(i).as_deref(), Some("typed"));
    // The attribute holds the default value and no longer drives the value.
    assert_eq!(doc.attribute_by_name(i, "value").unwrap().value, "default");
    doc.mutate().set_attribute_by_name(i, "value", "new default").unwrap();
    assert_eq!(doc.form_value(i).as_deref(), Some("typed"));
}

#[test]
fn checkedness_follows_the_attribute_until_dirty() {
    let mut doc = parse("<input id=c type=checkbox>");
    let c = q(&doc, "#c");
    assert!(!doc.checkedness(c));
    doc.mutate().set_attribute_by_name(c, "checked", "").unwrap();
    assert!(doc.checkedness(c), "adding the attribute checks a clean control");
    doc.mutate().remove_attribute_by_name(c, "checked");
    assert!(!doc.checkedness(c));

    doc.mutate().set_checkedness(c, true);
    doc.mutate().remove_attribute_by_name(c, "checked");
    assert!(doc.checkedness(c), "a dirty control ignores the attribute");
}

#[test]
fn checking_a_radio_unchecks_its_group() {
    let mut doc = parse("<input id=a type=radio name=g checked><input id=b type=radio name=g>");
    let (a, b) = (q(&doc, "#a"), q(&doc, "#b"));
    doc.mutate().set_checkedness(b, true);
    assert!(doc.checkedness(b) && !doc.checkedness(a));
}

#[test]
fn select_value_is_the_selected_option() {
    let doc = parse("<select id=s><option value=a>A</option><option selected>  B  </option></select>");
    assert_eq!(doc.form_value(q(&doc, "#s")).as_deref(), Some("B"));
}

struct Recorder(Vec<String>);

impl EventHandler for &mut Recorder {
    fn handle_event(&mut self, _: &[NodeId], event: &mut DomEvent, _: &mut dyn Document, _: &mut EventState) {
        self.0.push(event.name().to_string());
    }
}

#[test]
fn clicking_a_checkbox_toggles_it_and_fires_input_then_change() {
    let mut doc = parse("<input id=c type=checkbox>");
    let c = q(&doc, "#c");
    let click = DomEvent::new(c, doc.get_node(c).unwrap().synthetic_click_event(Modifiers::empty()));
    let mut recorder = Recorder(Vec::new());
    EventDriver::new(&mut doc, &mut recorder).handle_dom_event(click);
    // (Focus events follow: clicking also focuses the checkbox.)
    assert_eq!(recorder.0[..3], ["click", "input", "change"]);
    assert!(doc.checkedness(c), "toggled even before layout");
}
