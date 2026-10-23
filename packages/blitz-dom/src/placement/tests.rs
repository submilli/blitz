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
    let mut click = DomEvent::new(target, click);
    // This scenario models a physical click, including its focus default.
    click.pointer_activation = true;
    EventDriver::new(&mut document, handler).handle_dom_event(click);
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

/// Places one node elsewhere and records the events it sees.
struct MovedAway {
    node: NodeId,
    log: Rc<RefCell<Vec<String>>>,
}

impl EventHandler for MovedAway {
    fn handle_event(
        &mut self,
        _: &[NodeId],
        event: &mut DomEvent,
        _: &mut dyn Document,
        _: &mut EventState,
    ) {
        self.log.borrow_mut().push(event.name().to_string());
    }
    fn placement(&self, node: NodeId) -> Placement {
        if node == self.node {
            Placement::Elsewhere
        } else {
            Placement::Here(node)
        }
    }
}

#[test]
fn a_pointer_click_at_a_target_moved_away_is_not_dispatched() {
    for pointer in [true, false] {
        let mut document = BaseDocument::new(DocumentConfig::default());
        let target = checkbox(&mut document);
        let data = document.nodes[target].synthetic_click_event(Modifiers::empty());
        let mut click = DomEvent::new(target, data);
        click.pointer_activation = pointer;
        let mut document = Rc::new(RefCell::new(document));
        let log = Rc::new(RefCell::new(Vec::new()));
        let handler = MovedAway {
            node: target,
            log: log.clone(),
        };
        EventDriver::new(&mut document, handler).handle_dom_event(click);
        // A programmatic click still reaches the node.
        let expected: &[&str] = if pointer { &[] } else { &["click"] };
        assert_eq!(*log.borrow(), expected, "pointer={pointer}");
        assert_eq!(document.borrow().checkedness(target), !pointer);
    }
}

struct NativeKey {
    source: Option<Rc<RefCell<BaseDocument>>>,
    destination: Rc<RefCell<BaseDocument>>,
    target: NodeId,
    stage: &'static str,
    moved: Rc<Cell<Option<NodeId>>>,
    handed_off: Rc<RefCell<Option<ListenedEvent>>>,
    seen: Rc<RefCell<Vec<String>>>,
    cancel: bool,
}
impl NativeKey {
    fn move_target(&mut self, doc: &mut BaseDocument) {
        self.moved.set(Some(transfer(
            doc,
            self.target,
            &mut self.destination.borrow_mut(),
        )));
    }
}
impl EventHandler for NativeKey {
    fn handle_event(
        &mut self,
        _: &[NodeId],
        event: &mut DomEvent,
        doc: &mut dyn Document,
        _: &mut EventState,
    ) {
        self.seen.borrow_mut().push(event.name().to_owned());
        if event.name() == self.stage {
            self.move_target(&mut doc.inner_mut());
        }
    }
    fn before_input(&mut self, _: NodeId, _: &blitz_traits::events::BlitzKeyEvent) -> bool {
        self.seen.borrow_mut().push("beforeinput".into());
        if self.stage == "beforeinput" {
            let source = self
                .source
                .clone()
                .expect("the beforeinput test retains its source");
            self.move_target(&mut source.borrow_mut());
        }
        !self.cancel
    }
    fn placement(&self, node: NodeId) -> Placement {
        if node == self.target && self.moved.get().is_some() {
            Placement::Elsewhere
        } else {
            Placement::Here(node)
        }
    }
    fn complete_elsewhere(&mut self, event: ListenedEvent) {
        *self.handed_off.borrow_mut() = Some(event);
    }
}
fn key() -> blitz_traits::events::BlitzKeyEvent {
    blitz_traits::events::BlitzKeyEvent {
        key: keyboard_types::Key::Character("a".into()),
        code: keyboard_types::Code::KeyA,
        modifiers: Modifiers::empty(),
        location: keyboard_types::Location::Standard,
        is_auto_repeating: false,
        is_composing: false,
        state: blitz_traits::events::KeyState::Pressed,
        text: Some("a".into()),
    }
}
#[test]
fn adoption_during_keydown_or_keypress_suppresses_later_editing_steps() {
    for stage in ["keydown", "keypress"] {
        let mut source = BaseDocument::new(DocumentConfig::default());
        let target = input(&mut source, "text", None);
        source.flush_style_and_layout(0.0);
        source.set_focus_to(target);
        let seen = Rc::new(RefCell::new(Vec::new()));
        let handed_off = Rc::new(RefCell::new(None));
        let moved = Rc::new(Cell::new(None));
        let handler = NativeKey {
            source: None,
            destination: Rc::new(RefCell::new(BaseDocument::new(DocumentConfig::default()))),
            target,
            stage,
            moved: moved.clone(),
            handed_off: handed_off.clone(),
            seen: seen.clone(),
            cancel: false,
        };
        EventDriver::new(&mut source, handler).handle_dom_event(DomEvent::new(
            target,
            blitz_traits::events::DomEventData::KeyDown(key()),
        ));
        assert!(moved.get().is_some());
        assert!(handed_off.borrow().is_none());
        assert_eq!(
            *seen.borrow(),
            if stage == "keydown" {
                vec!["keydown"]
            } else {
                vec!["keydown", "keypress"]
            }
        );
    }
}
#[test]
fn canceled_beforeinput_leaves_the_control_value_unchanged() {
    let mut source = BaseDocument::new(DocumentConfig::default());
    let target = input(&mut source, "text", None);
    source.flush_style_and_layout(0.0);
    source.set_focus_to(target);
    let seen = Rc::new(RefCell::new(Vec::new()));
    let handler = NativeKey {
        source: None,
        destination: Rc::new(RefCell::new(BaseDocument::new(DocumentConfig::default()))),
        target,
        stage: "never",
        moved: Rc::new(Cell::new(None)),
        handed_off: Rc::new(RefCell::new(None)),
        seen: seen.clone(),
        cancel: true,
    };
    EventDriver::new(&mut source, handler).handle_dom_event(DomEvent::new(
        target,
        blitz_traits::events::DomEventData::KeyDown(key()),
    ));
    assert_eq!(*seen.borrow(), ["keydown", "keypress", "beforeinput"]);
    assert_eq!(source.form_value(target), Some(String::new()));
}

#[test]
fn admitted_text_input_follows_a_beforeinput_adoption_without_transferring_focus() {
    let mut document = BaseDocument::new(DocumentConfig::default());
    let target = input(&mut document, "text", None);
    document.flush_style_and_layout(0.0);
    document.set_focus_to(target);
    let mut source = Rc::new(RefCell::new(document));
    let destination = Rc::new(RefCell::new(BaseDocument::new(DocumentConfig::default())));
    let moved = Rc::new(Cell::new(None));
    let handed_off = Rc::new(RefCell::new(None));
    let seen = Rc::new(RefCell::new(Vec::new()));
    let handler = NativeKey {
        source: Some(source.clone()),
        destination: destination.clone(),
        target,
        stage: "beforeinput",
        moved: moved.clone(),
        handed_off: handed_off.clone(),
        seen: seen.clone(),
        cancel: false,
    };
    EventDriver::new(&mut source, handler).handle_dom_event(DomEvent::new(
        target,
        blitz_traits::events::DomEventData::KeyDown(key()),
    ));
    let moved = moved.get().unwrap();
    let event = handed_off
        .borrow_mut()
        .take()
        .unwrap()
        .relocate(&destination.borrow(), |id| (id == target).then_some(moved))
        .unwrap();
    let log = Rc::new(RefCell::new(Vec::new()));
    let mut destination = destination;
    EventDriver::new(&mut destination, Recorder(log.clone())).complete(event);
    assert_eq!(destination.borrow().form_value(moved), Some("a".into()));
    assert_eq!(destination.borrow().focus_node_id, None);
    assert_eq!(*seen.borrow(), ["keydown", "keypress", "beforeinput"]);
    assert_eq!(log.borrow()[0].0, "input");
}

#[test]
fn moved_pointer_press_hands_off_focus_without_mouse_compatibility_or_drag_state() {
    use blitz_traits::events::UiEvent;
    let mut source = BaseDocument::new(DocumentConfig::default());
    let target = input(&mut source, "text", None);
    source.resolve(0.0);
    let data = source.nodes[target].synthetic_click_event_data(Modifiers::empty());
    let moved = Rc::new(Cell::new(None));
    let handed_off = Rc::new(RefCell::new(None));
    let seen = Rc::new(RefCell::new(Vec::new()));
    let destination = Rc::new(RefCell::new(BaseDocument::new(DocumentConfig::default())));
    let handler = NativeKey {
        source: None,
        destination: destination.clone(),
        target,
        stage: "pointerdown",
        moved: moved.clone(),
        handed_off: handed_off.clone(),
        seen: seen.clone(),
        cancel: false,
    };
    EventDriver::new(&mut source, handler).handle_ui_event(UiEvent::PointerDown(data));
    let moved = moved.get().expect("the pointer listener moved its target");
    let event = handed_off
        .borrow_mut()
        .take()
        .expect("focus is handed off")
        .relocate(&destination.borrow(), |id| (id == target).then_some(moved))
        .unwrap();
    let log = Rc::new(RefCell::new(Vec::new()));
    let mut destination = destination;
    EventDriver::new(&mut destination, Recorder(log.clone())).complete(event);
    assert_eq!(destination.borrow().focus_node_id, Some(moved));
    assert_eq!(source.drag_mode, crate::events::DragMode::None);
    assert_eq!(
        destination.borrow().drag_mode,
        crate::events::DragMode::None
    );
    assert!(!seen.borrow().iter().any(|name| name == "mousedown"));
}
