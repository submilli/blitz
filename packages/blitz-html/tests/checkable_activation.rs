//! Engine-owned activation state, rollback, native labels and continuation leases.
mod common;
use blitz_dom::{Document, EventDriver, EventHandler, NodeId};
use blitz_traits::events::{DomEvent, EventState};
use common::{parse, q};
use std::{cell::RefCell, rc::Rc};

struct Observer {
    control: NodeId,
    cancel: bool,
    new_type: Option<&'static str>,
    remove_previous: Option<NodeId>,
    log: Rc<RefCell<Vec<String>>>,
}
impl EventHandler for Observer {
    fn handle_event(
        &mut self,
        _: &[NodeId],
        event: &mut DomEvent,
        doc: &mut dyn Document,
        state: &mut EventState,
    ) {
        if event.target != self.control {
            return;
        }
        let mut doc = doc.inner_mut();
        self.log.borrow_mut().push(format!(
            "{}:{}:{}",
            event.name(),
            doc.checkedness(self.control),
            event.checkable_completion
        ));
        if event.name() == "click" {
            if let Some(kind) = self.new_type {
                doc.mutate()
                    .set_attribute_by_name(self.control, "type", kind)
                    .unwrap();
            }
            if let Some(previous) = self.remove_previous {
                doc.mutate().remove_node(previous);
                assert!(!doc.reclaim_detached_nodes(&[]).contains(&previous));
            }
            if self.cancel {
                state.prevent_default();
            }
        }
    }
}
fn drive(html: &str, selector: &str, cancel: bool, remove: bool) -> (Vec<String>, bool) {
    drive_with_type(html, selector, cancel, remove, None)
}
fn drive_with_type(
    html: &str,
    selector: &str,
    cancel: bool,
    remove: bool,
    new_type: Option<&'static str>,
) -> (Vec<String>, bool) {
    let doc = parse(html);
    let control = q(&doc, "#c");
    let target = q(&doc, selector);
    let previous = remove.then(|| q(&doc, "#a"));
    let event = doc
        .get_node(target)
        .unwrap()
        .synthetic_click_event(Default::default());
    let mut shared = Rc::new(RefCell::new(doc));
    let log = Rc::new(RefCell::new(Vec::new()));
    EventDriver::new(
        &mut shared,
        Observer {
            control,
            cancel,
            new_type,
            remove_previous: previous,
            log: log.clone(),
        },
    )
    .handle_dom_event(DomEvent::new(target, event));
    let checked = shared.borrow().checkedness(control);
    let events = log.borrow().clone();
    (events, checked)
}
#[test]
fn native_checkbox_pre_activation_and_completion() {
    assert_eq!(
        drive("<input id=c type=checkbox>", "#c", false, false),
        (
            vec![
                "click:true:false".into(),
                "input:true:true".into(),
                "change:true:true".into(),
                "focus:true:false".into(),
                "focusin:true:false".into()
            ],
            true
        )
    );
}
#[test]
fn native_cancel_rolls_back_before_completion() {
    assert_eq!(
        drive("<input id=c type=checkbox>", "#c", true, false),
        (vec!["click:true:false".into()], false)
    );
}
#[test]
fn native_label_queues_control_click_and_obeys_cancellation() {
    let html = "<label id=l for=c>control</label><input id=c type=checkbox>";
    assert!(drive(html, "#l", false, false).1);
    assert_eq!(
        drive(html, "#l", true, false),
        (vec!["click:true:false".into()], false)
    );
}
#[test]
fn native_radio_rollback_preserves_detached_previous_until_callback_returns() {
    let html = "<input id=a type=radio name=g checked><input id=c type=radio name=g>";
    // Detachment changes the group, so canceled restoration leaves the target checked.
    assert_eq!(
        drive(html, "#c", true, true),
        (vec!["click:true:false".into()], true)
    );
}
#[test]
fn radio_group_uses_form_tree_and_nonempty_names() {
    let mut doc = parse(
        "<form id=f><input id=a type=radio name=g checked></form><input id=c type=radio name=g>",
    );
    let a = q(&doc, "#a");
    let c = q(&doc, "#c");
    doc.mutate().set_checkedness(c, true);
    assert!(doc.checkedness(a));
    doc.mutate().set_attribute_by_name(c, "form", "f").unwrap();
    doc.mutate().set_checkedness(c, true);
    assert!(!doc.checkedness(a));
    doc.mutate().set_attribute_by_name(a, "name", "").unwrap();
    doc.mutate().set_attribute_by_name(c, "name", "").unwrap();
    doc.mutate().set_checkedness(a, true);
    assert!(doc.checkedness(c));
}
#[test]
fn canceled_checkbox_restores_indeterminate() {
    let mut doc = parse("<input id=c type=checkbox>");
    let c = q(&doc, "#c");
    doc.get_node_mut(c)
        .unwrap()
        .element_data_mut()
        .unwrap()
        .form_state
        .indeterminate = true;
    let saved = doc.begin_checkable_activation(c).unwrap();
    assert!(doc.checkedness(c));
    assert!(
        !doc.get_node(c)
            .unwrap()
            .element_data()
            .unwrap()
            .form_state
            .indeterminate
    );
    assert_eq!(saved.finish(&mut doc, true), None);
    assert!(!doc.checkedness(c));
    assert!(
        doc.get_node(c)
            .unwrap()
            .element_data()
            .unwrap()
            .form_state
            .indeterminate
    );
}
#[test]
fn explicit_form_group_lookup_scales_linearly() {
    // Enough explicit form references to catch repeated full-tree owner searches.
    let html = format!(
        "<form id=f></form>{}",
        (0..4096)
            .map(|i| format!("<input id=r{i} type=radio name=g form=f>"))
            .collect::<String>()
    );
    let mut doc = parse(&html);
    let first = q(&doc, "#r0");
    let last = q(&doc, "#r4095");
    doc.mutate().set_checkedness(first, true);
    doc.mutate().set_checkedness(last, true);
    assert!(!doc.checkedness(first));
    assert!(doc.checkedness(last));
}

#[test]
fn wrapped_label_control_type_mutation_does_not_reforward() {
    let result = drive_with_type(
        "<label id=l><input id=c type=checkbox></label>",
        "#l",
        false,
        false,
        Some("hidden"),
    );
    assert_eq!(result.0, vec!["click:true:false"]);
}

#[test]
fn batched_form_ownership_matches_single_lookup() {
    let mut doc = parse(
        "<div id=duplicate></div><form id=duplicate><input id=a type=radio name=g></form><form id=f></form><input id=c type=radio name=g form=f><input id=d type=radio name=g form=duplicate>",
    );
    let a = q(&doc, "#a");
    let c = q(&doc, "#c");
    let d = q(&doc, "#d");
    doc.mutate().set_checkedness(a, true);
    doc.mutate().set_checkedness(c, true);
    doc.mutate().set_checkedness(d, true);
    assert!(doc.checkedness(a) && doc.checkedness(c) && doc.checkedness(d));
    // The first non-form duplicate ID prevents explicit ownership.
    assert_eq!(doc.form_owner(d), None);
}
