//! The DOM event dispatch algorithm in `blitz_dom::dom_events`, driven by a
//! recording host instead of a script engine.

mod common;

use std::cell::{Ref, RefCell, RefMut};
use std::rc::Rc;

use blitz_dom::BaseDocument;
use blitz_dom::dom_events::{
    DispatchHost, EventPhase, EventTargetId, Listener, ListenerOptions, dispatch,
};
use common::{parse, q};

/// Listener behaviour, by callback handle.
#[derive(Clone, Copy, PartialEq)]
enum Behaviour {
    Log,
    StopPropagation,
    StopImmediate,
    PreventDefault,
    /// Remove the listener with handle 99 on the same target.
    RemoveOther,
}

struct Host {
    doc: Rc<RefCell<BaseDocument>>,
    event_type: String,
    bubbles: bool,
    cancelable: bool,
    phase: EventPhase,
    current: Option<EventTargetId>,
    stop: bool,
    stop_immediate: bool,
    canceled: bool,
    passive: bool,
    behaviours: Vec<(u64, Behaviour)>,
    log: Vec<String>,
}

impl Host {
    fn new(doc: Rc<RefCell<BaseDocument>>, event_type: &str, bubbles: bool) -> Self {
        Host {
            doc,
            event_type: event_type.into(),
            bubbles,
            cancelable: true,
            phase: EventPhase::None,
            current: None,
            stop: false,
            stop_immediate: false,
            canceled: false,
            passive: false,
            behaviours: Vec::new(),
            log: Vec::new(),
        }
    }
}

impl DispatchHost for Host {
    fn document(&self) -> Ref<'_, BaseDocument> {
        self.doc.borrow()
    }
    fn document_mut(&self) -> RefMut<'_, BaseDocument> {
        self.doc.borrow_mut()
    }
    fn event_type(&self) -> String {
        self.event_type.clone()
    }
    fn bubbles(&self) -> bool {
        self.bubbles
    }
    fn set_phase(&mut self, phase: EventPhase, current: Option<EventTargetId>) {
        self.phase = phase;
        self.current = current;
    }
    fn set_target(&mut self, _: Option<EventTargetId>) {}
    fn set_in_passive_listener(&mut self, passive: bool) {
        self.passive = passive;
    }
    fn propagation_stopped(&self) -> bool {
        self.stop
    }
    fn immediate_propagation_stopped(&self) -> bool {
        self.stop_immediate
    }
    fn canceled(&self) -> bool {
        self.canceled
    }
    fn clear_propagation_flags(&mut self) {
        self.stop = false;
        self.stop_immediate = false;
    }
    fn invoke(&mut self, listener: &Listener) {
        self.log
            .push(format!("{}@{:?}", listener.callback, self.phase));
        let behaviour = self
            .behaviours
            .iter()
            .find(|(h, _)| *h == listener.callback)
            .map(|(_, b)| *b)
            .unwrap_or(Behaviour::Log);
        match behaviour {
            Behaviour::Log => {}
            Behaviour::StopPropagation => self.stop = true,
            Behaviour::StopImmediate => {
                self.stop = true;
                self.stop_immediate = true;
            }
            Behaviour::PreventDefault => {
                if self.cancelable && !self.passive {
                    self.canceled = true;
                }
            }
            Behaviour::RemoveOther => {
                let target = self.current.unwrap();
                self.doc
                    .borrow_mut()
                    .event_listeners
                    .remove(target, &self.event_type, 99, false);
            }
        }
    }
}

fn node(doc: &Rc<RefCell<BaseDocument>>, selector: &str) -> EventTargetId {
    EventTargetId::Node(q(&doc.borrow(), selector))
}

fn add(
    doc: &Rc<RefCell<BaseDocument>>,
    target: EventTargetId,
    handle: u64,
    options: ListenerOptions,
) {
    doc.borrow_mut()
        .event_listeners
        .add(target, "x", handle, options);
}

const CAPTURE: ListenerOptions = ListenerOptions {
    capture: true,
    passive: false,
    once: false,
};
const BUBBLE: ListenerOptions = ListenerOptions {
    capture: false,
    passive: false,
    once: false,
};

fn doc() -> Rc<RefCell<BaseDocument>> {
    Rc::new(RefCell::new(parse("<div id=outer><p id=inner></p></div>")))
}

#[test]
fn capture_then_target_then_bubble_up_to_the_window() {
    let doc = doc();
    let (outer, inner) = (node(&doc, "#outer"), node(&doc, "#inner"));
    add(&doc, EventTargetId::Window, 1, CAPTURE);
    add(&doc, outer, 2, CAPTURE);
    add(&doc, inner, 3, BUBBLE);
    add(&doc, inner, 4, CAPTURE);
    add(&doc, outer, 5, BUBBLE);
    add(&doc, EventTargetId::Window, 6, BUBBLE);
    let mut host = Host::new(doc.clone(), "x", true);
    assert!(dispatch(&mut host, inner));
    assert_eq!(
        host.log,
        [
            "1@Capturing",
            "2@Capturing",
            "4@AtTarget",
            "3@AtTarget",
            "5@Bubbling",
            "6@Bubbling"
        ]
    );
    assert_eq!(host.phase, EventPhase::None);
}

#[test]
fn non_bubbling_events_skip_the_bubble_phase() {
    let doc = doc();
    let (outer, inner) = (node(&doc, "#outer"), node(&doc, "#inner"));
    add(&doc, outer, 1, CAPTURE);
    add(&doc, outer, 2, BUBBLE);
    add(&doc, inner, 3, BUBBLE);
    let mut host = Host::new(doc.clone(), "x", false);
    dispatch(&mut host, inner);
    assert_eq!(host.log, ["1@Capturing", "3@AtTarget"]);
}

#[test]
fn stop_propagation_and_stop_immediate_propagation() {
    let doc = doc();
    let (outer, inner) = (node(&doc, "#outer"), node(&doc, "#inner"));
    add(&doc, inner, 1, BUBBLE);
    add(&doc, inner, 2, BUBBLE);
    add(&doc, outer, 3, BUBBLE);
    let mut host = Host::new(doc.clone(), "x", true);
    host.behaviours = vec![(1, Behaviour::StopPropagation)];
    dispatch(&mut host, inner);
    assert_eq!(
        host.log,
        ["1@AtTarget", "2@AtTarget"],
        "the target's other listeners still run"
    );

    let mut host = Host::new(doc.clone(), "x", true);
    host.behaviours = vec![(1, Behaviour::StopImmediate)];
    dispatch(&mut host, inner);
    assert_eq!(host.log, ["1@AtTarget"]);
}

#[test]
fn prevent_default_cancels_unless_passive() {
    let doc = doc();
    let inner = node(&doc, "#inner");
    add(&doc, inner, 1, BUBBLE);
    let mut host = Host::new(doc.clone(), "x", true);
    host.behaviours = vec![(1, Behaviour::PreventDefault)];
    assert!(!dispatch(&mut host, inner));

    let doc = self::doc();
    let inner = node(&doc, "#inner");
    add(
        &doc,
        inner,
        1,
        ListenerOptions {
            passive: true,
            ..BUBBLE
        },
    );
    let mut host = Host::new(doc.clone(), "x", true);
    host.behaviours = vec![(1, Behaviour::PreventDefault)];
    assert!(
        dispatch(&mut host, inner),
        "passive listeners cannot cancel"
    );
}

#[test]
fn once_listeners_run_once_and_duplicates_are_ignored() {
    let doc = doc();
    let inner = node(&doc, "#inner");
    add(
        &doc,
        inner,
        1,
        ListenerOptions {
            once: true,
            ..BUBBLE
        },
    );
    assert!(!doc.borrow_mut().event_listeners.add(
        inner,
        "x",
        1,
        ListenerOptions {
            once: true,
            ..BUBBLE
        }
    ));
    let mut host = Host::new(doc.clone(), "x", true);
    dispatch(&mut host, inner);
    dispatch(&mut host, inner);
    assert_eq!(host.log, ["1@AtTarget"]);
}

#[test]
fn listeners_removed_during_dispatch_do_not_run() {
    let doc = doc();
    let inner = node(&doc, "#inner");
    add(&doc, inner, 1, BUBBLE);
    add(&doc, inner, 99, BUBBLE);
    let mut host = Host::new(doc.clone(), "x", true);
    host.behaviours = vec![(1, Behaviour::RemoveOther)];
    dispatch(&mut host, inner);
    assert_eq!(host.log, ["1@AtTarget"]);
}

#[test]
fn detached_nodes_have_no_window_in_their_path() {
    let doc = doc();
    let detached = doc.borrow_mut().mutate().create_text_node("t");
    let path = doc.borrow().event_path(EventTargetId::Node(detached));
    assert_eq!(path, vec![EventTargetId::Node(detached)]);
}
