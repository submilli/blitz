//! Native driver continuations outlive callbacks that detach and collect nodes.
mod common;
use blitz_dom::{Document, EventDriver, EventHandler, NodeId};
use blitz_traits::events::{DomEvent, DomEventData, EventState, UiEvent};
use common::{parse, q};
use std::{cell::RefCell, rc::Rc};

struct Collector {
    target: NodeId,
    trigger: &'static str,
    events: Rc<RefCell<Vec<String>>>,
}
impl EventHandler for Collector {
    fn handle_event(
        &mut self,
        _chain: &[NodeId],
        event: &mut DomEvent,
        doc: &mut dyn Document,
        _: &mut EventState,
    ) {
        if event.target != self.target {
            return;
        }
        self.events.borrow_mut().push(event.data.name().to_owned());
        let mut doc = doc.inner_mut();
        if event.data.name() == self.trigger {
            let parent = doc.get_node(self.target).unwrap().parent.unwrap();
            doc.mutate().remove_child(parent, self.target).unwrap();
        }
        assert!(!doc.reclaim_detached_nodes(&[]).contains(&self.target));
        assert!(doc.get_node(self.target).is_some());
    }
}

#[test]
fn pointer_transition_chains_survive_detachment_and_collection() {
    let mut doc = parse(
        "<div id=old style='position:absolute;left:0;top:0;width:100px;height:100px'></div><div id=new style='position:absolute;left:200px;top:0;width:100px;height:100px'></div>",
    );
    doc.resolve(0.0);
    let old = q(&doc, "#old");
    let new = q(&doc, "#new");
    let mut old_pointer = doc
        .get_node(old)
        .unwrap()
        .synthetic_click_event_data(Default::default());
    old_pointer.buttons = Default::default();
    let mut new_pointer = doc
        .get_node(new)
        .unwrap()
        .synthetic_click_event_data(Default::default());
    new_pointer.buttons = Default::default();
    let events = Rc::new(RefCell::new(Vec::new()));
    let mut shared = Rc::new(RefCell::new(doc));
    {
        let mut driver = EventDriver::new(
            &mut shared,
            Collector {
                target: old,
                trigger: "pointerout",
                events: events.clone(),
            },
        );
        driver.handle_ui_event(UiEvent::PointerMove(old_pointer));
        events.borrow_mut().clear();
        driver.handle_ui_event(UiEvent::PointerMove(new_pointer));
    }
    assert_eq!(
        &*events.borrow(),
        &["pointerout", "mouseout", "pointerleave", "mouseleave"]
    );
    assert!(
        shared
            .borrow_mut()
            .reclaim_detached_nodes(&[])
            .contains(&old)
    );
}

#[test]
fn queued_event_target_survives_callback_until_default_action_finishes() {
    let mut doc = parse("<div id=target></div>");
    doc.resolve(0.0);
    let target = q(&doc, "#target");
    let events = Rc::new(RefCell::new(Vec::new()));
    let mut shared = Rc::new(RefCell::new(doc));
    let click = shared
        .borrow()
        .get_node(target)
        .unwrap()
        .synthetic_click_event_data(Default::default());
    EventDriver::new(
        &mut shared,
        Collector {
            target,
            trigger: "click",
            events: events.clone(),
        },
    )
    .handle_dom_event(DomEvent::new(target, DomEventData::Click(click)));
    assert_eq!(&*events.borrow(), &["click"]);
    assert!(
        shared
            .borrow_mut()
            .reclaim_detached_nodes(&[])
            .contains(&target)
    );
}
