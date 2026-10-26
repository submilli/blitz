//! Input controllers must own counts and navigation without changing desktop defaults.
use crate::{
    BaseDocument, Document, DocumentConfig, EventDriver, EventHandler, NodeId, QualName, ns,
};
use blitz_traits::{
    events::{DomEvent, DomEventData, EventState, UiEvent},
    navigation::{NavigationOptions, NavigationProvider},
};
use std::{
    cell::RefCell,
    num::NonZeroU16,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

type Log = Rc<RefCell<Vec<(&'static str, u16)>>>;
struct Recorder(Log);
impl EventHandler for Recorder {
    fn handle_event(
        &mut self,
        _chain: &[NodeId],
        event: &mut DomEvent,
        doc: &mut dyn Document,
        _state: &mut EventState,
    ) {
        if matches!(
            event.data,
            DomEventData::Click(_) | DomEventData::DoubleClick(_)
        ) {
            self.0
                .borrow_mut()
                .push((event.data.name(), doc.inner().click_count));
        }
    }
}
fn element(doc: &mut BaseDocument, parent: NodeId, tag: &str) -> NodeId {
    let node = doc
        .mutate()
        .create_element(QualName::new(None, ns!(html), tag.into()), vec![]);
    doc.mutate().append_children(parent, &[node]);
    node
}
fn document(tag: &str) -> (BaseDocument, NodeId) {
    let mut doc = BaseDocument::new(DocumentConfig {
        base_url: Some("https://a.test/".into()),
        ..Default::default()
    });
    let root = doc.root_node().id;
    let html = element(&mut doc, root, "html");
    let body = element(&mut doc, html, "body");
    let target = element(&mut doc, body, tag);
    doc.mutate().set_attribute(
        target,
        QualName::new(None, ns!(), "style".into()),
        "display:block;width:100px;height:40px",
    );
    doc.resolve(0.0);
    (doc, target)
}
fn click(doc: &mut BaseDocument, target: NodeId, count: u16, log: &Log) {
    let pointer = doc.nodes[target].synthetic_click_event_data(keyboard_types::Modifiers::empty());
    for event in [
        UiEvent::PointerMove(pointer.clone()),
        UiEvent::PointerDown(pointer.clone()),
        UiEvent::PointerUp(pointer),
    ] {
        EventDriver::new(doc, Recorder(log.clone()))
            .with_click_count(NonZeroU16::new(count).unwrap())
            .handle_ui_event(event);
    }
}
#[test]
fn explicit_counts_survive_single_then_repeated_double_actions() {
    let (mut doc, target) = document("button");
    let log = Log::default();
    for count in [1, 1, 2, 1, 2] {
        click(&mut doc, target, count, &log);
    }
    assert_eq!(
        *log.borrow(),
        [
            ("click", 1),
            ("click", 1),
            ("click", 2),
            ("dblclick", 2),
            ("click", 1),
            ("click", 2),
            ("dblclick", 2)
        ]
    );
}
#[test]
fn automatic_pointer_count_saturates_instead_of_overflowing() {
    let (mut doc, target) = document("button");
    let log = Log::default();
    click(&mut doc, target, 1, &log);
    let pointer = doc.nodes[target].synthetic_click_event_data(keyboard_types::Modifiers::empty());
    doc.click_count = u16::MAX;
    doc.last_mousedown_time = Some(doc.now_ms());
    doc.mousedown_position = taffy::Point {
        x: pointer.page_x(),
        y: pointer.page_y(),
    };
    EventDriver::new(&mut doc, Recorder(log)).handle_ui_event(UiEvent::PointerDown(pointer));
    assert_eq!(doc.click_count, u16::MAX);
}
struct Navigation(AtomicUsize);
impl NavigationProvider for Navigation {
    fn navigate_to(&self, _options: NavigationOptions) {
        panic!("fragment must use its own hook")
    }
    fn navigate_fragment(&self, url: url::Url, _source: usize) -> bool {
        assert_eq!(url.as_str(), "https://a.test/#target");
        self.0.fetch_add(1, Ordering::Relaxed);
        true
    }
}
#[test]
fn fragment_navigation_reaches_embedder_and_honors_click_cancellation() {
    let (mut doc, target) = document("a");
    doc.mutate()
        .set_attribute(target, QualName::new(None, ns!(), "href".into()), "#target");
    doc.resolve(0.0);
    let navigation = Arc::new(Navigation(AtomicUsize::new(0)));
    doc.set_navigation_provider(navigation.clone());
    let pointer = doc.nodes[target].synthetic_click_event_data(keyboard_types::Modifiers::empty());
    let before = doc.viewport_scroll();
    EventDriver::new(&mut doc, Recorder(Log::default()))
        .handle_dom_event(DomEvent::new(target, DomEventData::Click(pointer.clone())));
    assert_eq!(navigation.0.load(Ordering::Relaxed), 1);
    assert_eq!(doc.viewport_scroll(), before);
    struct Cancel;
    impl EventHandler for Cancel {
        fn handle_event(
            &mut self,
            _chain: &[NodeId],
            _event: &mut DomEvent,
            _doc: &mut dyn Document,
            state: &mut EventState,
        ) {
            state.prevent_default()
        }
    }
    EventDriver::new(&mut doc, Cancel)
        .handle_dom_event(DomEvent::new(target, DomEventData::Click(pointer)));
    assert_eq!(navigation.0.load(Ordering::Relaxed), 1);
}

#[test]
fn programmatic_checkable_click_preserves_existing_focus() {
    let (mut doc, target) = document("input");
    doc.mutate().set_attribute(
        target,
        QualName::new(None, ns!(), "type".into()),
        "checkbox",
    );
    let body = doc.nodes[target].parent.unwrap();
    let previous = element(&mut doc, body, "input");
    doc.resolve(0.0);
    let log = Log::default();
    EventDriver::new(&mut doc, Recorder(log.clone())).focus(Some(previous));
    assert_eq!(doc.get_focussed_node_id(), Some(previous));
    let click = doc.nodes[target].synthetic_click_event(keyboard_types::Modifiers::empty());
    EventDriver::new(&mut doc, Recorder(log.clone()))
        .handle_dom_event(DomEvent::new(target, click));
    assert!(doc.checkedness(target));
    assert_eq!(doc.get_focussed_node_id(), Some(previous));
    assert_eq!(
        log.borrow()
            .iter()
            .map(|(name, _)| *name)
            .collect::<Vec<_>>(),
        ["click"]
    );
}

#[test]
fn programmatic_plain_click_preserves_focus_after_physical_double_click() {
    let (mut doc, previous) = document("input");
    let body = doc.nodes[previous].parent.unwrap();
    let target = element(&mut doc, body, "div");
    let log = Log::default();
    EventDriver::new(&mut doc, Recorder(log.clone())).focus(Some(previous));
    doc.click_count = 2;
    let click = doc.nodes[target].synthetic_click_event(keyboard_types::Modifiers::empty());
    EventDriver::new(&mut doc, Recorder(log.clone()))
        .handle_dom_event(DomEvent::new(target, click));
    assert_eq!(doc.get_focussed_node_id(), Some(previous));
    assert_eq!(
        log.borrow()
            .iter()
            .map(|(name, _)| *name)
            .collect::<Vec<_>>(),
        ["click"]
    );
}

#[test]
fn programmatic_summary_click_toggles_without_moving_focus() {
    let (mut doc, details) = document("details");
    let target = element(&mut doc, details, "summary");
    let body = doc.nodes[details].parent.unwrap();
    let previous = element(&mut doc, body, "input");
    doc.resolve(0.0);
    let log = Log::default();
    EventDriver::new(&mut doc, Recorder(log.clone())).focus(Some(previous));
    let click = doc.nodes[target].synthetic_click_event(keyboard_types::Modifiers::empty());
    EventDriver::new(&mut doc, Recorder(log)).handle_dom_event(DomEvent::new(target, click));
    assert!(
        doc.nodes[details]
            .element_data()
            .unwrap()
            .attr(crate::local_name!("open"))
            .is_some()
    );
    assert_eq!(doc.get_focussed_node_id(), Some(previous));
}

#[test]
fn physical_label_click_focuses_its_associated_checkable_control() {
    let (mut doc, label) = document("label");
    let body = doc.nodes[label].parent.unwrap();
    let target = element(&mut doc, body, "input");
    for (node, name, value) in [
        (label, "for", "choice"),
        (target, "id", "choice"),
        (target, "type", "checkbox"),
    ] {
        doc.mutate()
            .set_attribute(node, QualName::new(None, ns!(), name.into()), value);
    }
    doc.resolve(0.0);
    let log = Log::default();
    click(&mut doc, label, 1, &log);
    assert!(doc.checkedness(target));
    assert_eq!(doc.get_focussed_node_id(), Some(target));
}

#[test]
fn physical_disabled_label_does_not_focus_an_ancestor_of_the_control() {
    let (mut doc, label) = document("label");
    let body = doc.nodes[label].parent.unwrap();
    let ancestor = element(&mut doc, body, "div");
    let target = element(&mut doc, ancestor, "input");
    for (node, name, value) in [
        (label, "for", "choice"),
        (ancestor, "tabindex", "0"),
        (target, "id", "choice"),
        (target, "type", "checkbox"),
        (target, "disabled", ""),
    ] {
        doc.mutate()
            .set_attribute(node, QualName::new(None, ns!(), name.into()), value);
    }
    doc.resolve(0.0);
    click(&mut doc, label, 1, &Log::default());
    assert!(!doc.checkedness(target));
    assert_ne!(doc.get_focussed_node_id(), Some(ancestor));
    assert_ne!(doc.get_focussed_node_id(), Some(target));
}

#[test]
fn document_pointer_leave_clears_hover_and_dispatches_boundaries_once() {
    struct Boundaries {
        target: NodeId,
        log: Rc<RefCell<Vec<&'static str>>>,
    }
    impl EventHandler for Boundaries {
        fn handle_event(
            &mut self,
            _: &[NodeId],
            event: &mut DomEvent,
            _: &mut dyn Document,
            _: &mut EventState,
        ) {
            if event.target == self.target
                && matches!(
                    event.data,
                    DomEventData::PointerOut(_)
                        | DomEventData::MouseOut(_)
                        | DomEventData::PointerLeave(_)
                        | DomEventData::MouseLeave(_)
                        | DomEventData::PointerOver(_)
                        | DomEventData::MouseOver(_)
                )
            {
                self.log.borrow_mut().push(event.data.name());
            }
        }
    }
    let (mut doc, target) = document("button");
    let log = Rc::new(RefCell::new(Vec::new()));
    let pointer = doc.nodes[target].synthetic_click_event_data(keyboard_types::Modifiers::empty());
    EventDriver::new(
        &mut doc,
        Boundaries {
            target,
            log: log.clone(),
        },
    )
    .handle_pointer_move(&pointer);
    assert_eq!(doc.get_hover_node_id(), Some(target));
    log.borrow_mut().clear();
    EventDriver::new(
        &mut doc,
        Boundaries {
            target,
            log: log.clone(),
        },
    )
    .handle_pointer_leave(&pointer);
    assert_eq!(
        *log.borrow(),
        ["pointerout", "mouseout", "pointerleave", "mouseleave"]
    );
    assert_eq!(doc.get_hover_node_id(), None);
    assert!(!doc.refresh_hover());
    EventDriver::new(
        &mut doc,
        Boundaries {
            target,
            log: log.clone(),
        },
    )
    .handle_pointer_leave(&pointer);
    assert_eq!(log.borrow().len(), 4);
    EventDriver::new(
        &mut doc,
        Boundaries {
            target,
            log: log.clone(),
        },
    )
    .handle_pointer_move(&pointer);
    assert_eq!(doc.get_hover_node_id(), Some(target));
    assert_eq!(&log.borrow()[4..], ["pointerover", "mouseover"]);
}
