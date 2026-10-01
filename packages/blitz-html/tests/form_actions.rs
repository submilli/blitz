//! Native activation resolves current ownership and defers script to the driver.
mod common;
use blitz_dom::{Document, EventDriver, EventHandler, FormAction, ImplicitSubmission, NodeId};
use blitz_traits::events::{DomEvent, EventState};
use common::{parse, q};

#[test]
fn implicit_submission_uses_tree_order_and_live_ownership() {
    let mut doc = parse(
        "<button id=first form=f></button><form id=f><input id=text><input id=second><button id=inside></button></form><form id=g></form>",
    );
    let input = q(&doc, "#text");
    let first = q(&doc, "#first");
    let inside = q(&doc, "#inside");
    assert_eq!(
        doc.implicit_submission(input),
        Some(ImplicitSubmission::Click(first))
    );
    doc.mutate()
        .set_attribute_by_name(first, "disabled", "")
        .unwrap();
    assert_eq!(doc.implicit_submission(input), None);
    doc.mutate()
        .set_attribute_by_name(first, "form", "g")
        .unwrap();
    assert_eq!(
        doc.implicit_submission(input),
        Some(ImplicitSubmission::Click(inside))
    );
    doc.mutate()
        .set_attribute_by_name(inside, "type", "button")
        .unwrap();
    assert_eq!(doc.implicit_submission(input), None);
    let second = q(&doc, "#second");
    doc.mutate()
        .set_attribute_by_name(second, "type", "hidden")
        .unwrap();
    assert_eq!(
        doc.implicit_submission(input),
        Some(ImplicitSubmission::Submit(q(&doc, "#f")))
    );
    doc.mutate()
        .set_attribute_by_name(second, "type", "unknown")
        .unwrap();
    assert_eq!(doc.implicit_submission(input), None);
}

struct Actions<'a> {
    actions: &'a mut Vec<FormAction>,
    reassociate: Option<NodeId>,
    cancel: bool,
}
impl EventHandler for Actions<'_> {
    fn handle_event(
        &mut self,
        _: &[NodeId],
        _: &mut DomEvent,
        doc: &mut dyn Document,
        state: &mut EventState,
    ) {
        if let Some(button) = self.reassociate.take() {
            doc.inner_mut()
                .mutate()
                .set_attribute_by_name(button, "form", "g")
                .unwrap();
        }
        if self.cancel {
            state.prevent_default();
        }
    }
    fn handle_form_action(&mut self, action: FormAction, doc: &mut dyn Document) {
        // Acquiring a mutable borrow here catches accidental driver borrow retention.
        let mut doc = doc.inner_mut();
        doc.reclaim_detached_nodes(&[]);
        self.actions.push(action);
    }
}

#[test]
fn driver_activates_current_form_after_click_and_honors_cancellation() {
    for (kind, reset) in [("INVALID", false), ("SuBmIt", false), ("ReSeT", true)] {
        let mut doc = parse(&format!(
            "<form id=f><button id=b type='{kind}'><span id=child>go</span></button></form><form id=g></form>"
        ));
        let button = q(&doc, "#b");
        let target = q(&doc, "#child");
        let form = q(&doc, "#g");
        let click = doc
            .get_node(target)
            .unwrap()
            .synthetic_click_event(keyboard_types::Modifiers::empty());
        let mut actions = vec![];
        EventDriver::new(
            &mut doc,
            Actions {
                actions: &mut actions,
                reassociate: Some(button),
                cancel: false,
            },
        )
        .handle_dom_event(DomEvent::new(target, click.clone()));
        assert_eq!(
            actions,
            vec![if reset {
                FormAction::Reset { form }
            } else {
                FormAction::Submit {
                    form,
                    submitter: Some(button),
                }
            }]
        );
        actions.clear();
        EventDriver::new(
            &mut doc,
            Actions {
                actions: &mut actions,
                reassociate: None,
                cancel: true,
            },
        )
        .handle_dom_event(DomEvent::new(target, click));
        assert!(actions.is_empty());
    }
}
