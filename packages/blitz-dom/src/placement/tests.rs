//! A click whose listener moves its checkbox into another document finishes
//! its activation there, in script dispatch and in the native driver.
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use blitz_traits::events::{DomEvent, EventState};
use keyboard_types::Modifiers;

use crate::dom_events::{
    DispatchHost, EventPhase, EventTargetId, Listener, ListenerOptions, dispatch,
};
use crate::foreign_import::ImportMode;
use crate::{
    BaseDocument, ClickActivation, Document, DocumentConfig, EventDriver, EventHandler,
    ListenedEvent, NodeId, Placement, QualName, ns,
};

fn checkbox(doc: &mut BaseDocument) -> NodeId {
    input(doc, "checkbox", None)
}

fn input(doc: &mut BaseDocument, kind: &str, name: Option<&str>) -> NodeId {
    let mut mutator = doc.mutate();
    let input = mutator.create_element(QualName::new(None, ns!(html), "input".into()), vec![]);
    mutator.set_attribute(input, QualName::new(None, ns!(), "type".into()), kind);
    if let Some(name) = name {
        mutator.set_attribute(input, QualName::new(None, ns!(), "name".into()), name);
    }
    let root = mutator.doc.root_node_id;
    mutator.append_children(root, &[input]);
    input
}

/// Move `node` from `source` into `destination`, as an embedder's adoption
/// does: rebuild it there, connected, and remove the source copy.
fn transfer(source: &mut BaseDocument, node: NodeId, destination: &mut BaseDocument) -> NodeId {
    let root = destination.root_node_id;
    let moved = {
        let mut mutator = destination.mutate();
        let imported = mutator
            .try_import_foreign(source, node, root, ImportMode::Transfer)
            .unwrap();
        let moved = imported.root();
        mutator.append_children(root, &[moved]);
        moved
    };
    source.mutate().remove_node(node);
    moved
}

/// Script dispatch of a click at a checkbox whose listener moves it.
struct ScriptClick {
    document: RefCell<BaseDocument>,
    elsewhere: RefCell<BaseDocument>,
    target: NodeId,
    /// Move the checkbox back into `document` under a new ID instead.
    and_back: bool,
    cancel: bool,
    moved: Cell<Option<NodeId>>,
    activated_here: Vec<NodeId>,
    handed_off: Option<ClickActivation>,
    followed: Vec<NodeId>,
}

impl ScriptClick {
    fn new(cancel: bool, and_back: bool) -> Self {
        let mut document = BaseDocument::new(DocumentConfig::default());
        let target = checkbox(&mut document);
        document.event_listeners.add(
            EventTargetId::Node(target),
            "click",
            1,
            ListenerOptions::default(),
        );
        Self {
            document: RefCell::new(document),
            elsewhere: RefCell::new(BaseDocument::new(DocumentConfig::default())),
            target,
            and_back,
            cancel,
            moved: Cell::new(None),
            activated_here: Vec::new(),
            handed_off: None,
            followed: Vec::new(),
        }
    }
}

impl DispatchHost for ScriptClick {
    fn document(&self) -> std::cell::Ref<'_, BaseDocument> {
        self.document.borrow()
    }
    fn document_mut(&self) -> std::cell::RefMut<'_, BaseDocument> {
        self.document.borrow_mut()
    }
    fn event_type(&self) -> String {
        "click".into()
    }
    fn bubbles(&self) -> bool {
        true
    }
    fn handles_click_activation(&self) -> bool {
        true
    }
    fn checkable_activated(&mut self, target: NodeId) {
        self.activated_here.push(target);
    }
    fn set_phase(&mut self, _: EventPhase, _: Option<EventTargetId>) {}
    fn set_target(&mut self, _: Option<EventTargetId>) {}
    fn set_in_passive_listener(&mut self, _: bool) {}
    fn propagation_stopped(&self) -> bool {
        false
    }
    fn immediate_propagation_stopped(&self) -> bool {
        false
    }
    fn canceled(&self) -> bool {
        self.cancel
    }
    fn clear_propagation_flags(&mut self) {}
    fn invoke(&mut self, _: &Listener) {
        let mut document = self.document.borrow_mut();
        let mut elsewhere = self.elsewhere.borrow_mut();
        let mut moved = transfer(&mut document, self.target, &mut elsewhere);
        if self.and_back {
            moved = transfer(&mut elsewhere, moved, &mut document);
        }
        self.moved.set(Some(moved));
    }
    fn follow(&mut self, node: NodeId) {
        self.followed.push(node);
    }
    fn placement(&self, node: NodeId) -> Placement {
        match self.moved.get() {
            Some(moved) if node == self.target && self.and_back => Placement::Here(moved),
            Some(_) if node == self.target => Placement::Elsewhere,
            _ => Placement::Here(node),
        }
    }
    fn activate_elsewhere(&mut self, activation: ClickActivation) {
        self.handed_off = Some(activation);
    }
}

#[test]
fn a_script_click_hands_a_moved_checkbox_activation_to_the_embedder() {
    for cancel in [false, true] {
        let mut host = ScriptClick::new(cancel, false);
        let target = EventTargetId::Node(host.target);
        dispatch(&mut host, target);
        assert!(
            host.activated_here.is_empty(),
            "nothing completes on the old copy"
        );
        let moved = host.moved.get().unwrap();
        let target = host.target;
        let activation = host
            .handed_off
            .take()
            .unwrap()
            .relocate(&host.elsewhere.borrow(), |id| {
                (id == target).then_some(moved)
            });
        let mut elsewhere = host.elsewhere.borrow_mut();
        let activated = activation.run(&mut elsewhere);
        // A canceled click rolls back the moved copy's checkedness.
        assert_eq!(elsewhere.checkedness(moved), !cancel);
        assert_eq!(activated.checkable, (!cancel).then_some(moved));
        assert_eq!(activated.form, None);
    }
}

#[test]
fn a_script_click_activates_a_checkbox_moved_away_and_back_under_its_new_id() {
    let mut host = ScriptClick::new(false, true);
    let target = EventTargetId::Node(host.target);
    dispatch(&mut host, target);
    let moved = host.moved.get().unwrap();
    assert_ne!(moved, host.target);
    assert!(host.handed_off.is_none());
    assert_eq!(host.activated_here, [moved]);
    assert!(host.document.borrow().checkedness(moved));
}

#[test]
fn a_script_click_follows_the_radio_that_was_checked_before() {
    let mut host = ScriptClick::new(false, false);
    let (previous, radio) = {
        let mut document = host.document.borrow_mut();
        let previous = input(&mut document, "radio", Some("r"));
        document.mutate().set_checkedness(previous, true);
        (previous, input(&mut document, "radio", Some("r")))
    };
    host.target = radio;
    host.document.borrow_mut().event_listeners.add(
        EventTargetId::Node(radio),
        "click",
        1,
        ListenerOptions::default(),
    );
    dispatch(&mut host, EventTargetId::Node(radio));
    assert_eq!(host.followed, [previous]);
}

/// The native driver's handler for a click whose listener moves the
/// checkbox into `elsewhere`.
struct NativeClick {
    elsewhere: Rc<RefCell<BaseDocument>>,
    target: NodeId,
    moved: Rc<Cell<Option<NodeId>>>,
    handed_off: Rc<RefCell<Option<ListenedEvent>>>,
}

impl EventHandler for NativeClick {
    fn handle_event(
        &mut self,
        _: &[NodeId],
        event: &mut DomEvent,
        doc: &mut dyn Document,
        _: &mut EventState,
    ) {
        if event.target == self.target && event.name() == "click" {
            let moved = transfer(
                &mut doc.inner_mut(),
                self.target,
                &mut self.elsewhere.borrow_mut(),
            );
            self.moved.set(Some(moved));
        }
    }
    fn placement(&self, node: NodeId) -> Placement {
        if node == self.target && self.moved.get().is_some() {
            return Placement::Elsewhere;
        }
        Placement::Here(node)
    }
    fn complete_elsewhere(&mut self, event: ListenedEvent) {
        *self.handed_off.borrow_mut() = Some(event);
    }
}

/// Records the events a driver over the destination dispatches.
struct Recorder(Rc<RefCell<Vec<(String, NodeId, bool)>>>);

impl EventHandler for Recorder {
    fn handle_event(
        &mut self,
        _: &[NodeId],
        event: &mut DomEvent,
        _: &mut dyn Document,
        _: &mut EventState,
    ) {
        self.0.borrow_mut().push((
            event.name().to_string(),
            event.target,
            event.checkable_completion,
        ));
    }
}

#[test]
fn the_driver_completes_a_moved_checkbox_click_in_its_new_document() {
    let mut document = BaseDocument::new(DocumentConfig::default());
    let target = checkbox(&mut document);
    let click = document.nodes[target].synthetic_click_event(Modifiers::empty());
    let mut document = Rc::new(RefCell::new(document));
    let elsewhere = Rc::new(RefCell::new(BaseDocument::new(DocumentConfig::default())));
    let moved = Rc::new(Cell::new(None));
    let handed_off = Rc::new(RefCell::new(None));
    let handler = NativeClick {
        elsewhere: elsewhere.clone(),
        target,
        moved: moved.clone(),
        handed_off: handed_off.clone(),
    };
    EventDriver::new(&mut document, handler).handle_dom_event(DomEvent::new(target, click));
    let moved = moved.get().unwrap();
    let event = handed_off.borrow_mut().take().unwrap();
    assert_eq!(event.target(), target);
    let event = event
        .relocate(&elsewhere.borrow(), |id| (id == target).then_some(moved))
        .unwrap();
    let log = Rc::new(RefCell::new(Vec::new()));
    let mut elsewhere = elsewhere;
    EventDriver::new(&mut elsewhere, Recorder(log.clone())).complete(event);
    assert!(elsewhere.borrow().checkedness(moved));
    // The click's default action focuses the moved checkbox there too.
    let log = log.borrow();
    let log: Vec<_> = log
        .iter()
        .map(|(name, id, completion)| (name.as_str(), *id, *completion))
        .collect();
    assert_eq!(
        log,
        [
            ("input", moved, true),
            ("change", moved, true),
            ("focus", moved, false),
            ("focusin", moved, false)
        ]
    );
}
