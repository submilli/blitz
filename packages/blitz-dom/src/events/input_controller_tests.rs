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

struct FrameRecorder {
    doc: BaseDocument,
    events: Rc<RefCell<Vec<UiEvent>>>,
}
impl Document for FrameRecorder {
    fn inner(&self) -> crate::DocGuard<'_> {
        crate::DocGuard::Ref(&self.doc)
    }
    fn inner_mut(&mut self) -> crate::DocGuardMut<'_> {
        crate::DocGuardMut::Ref(&mut self.doc)
    }
    fn handle_ui_event(&mut self, event: UiEvent) {
        self.events.borrow_mut().push(event);
    }
}
fn bordered_frame() -> (BaseDocument, NodeId, Rc<RefCell<Vec<UiEvent>>>) {
    let (mut doc, host) = document("iframe");
    doc.embedder_loads_iframes = true;
    doc.mutate().set_attribute(host,QualName::new(None,ns!(),"style".into()),"display:block;width:100px;height:100px;border:10px solid black;padding:5px;transform:scale(2);transform-origin:0 0");
    let events = Rc::new(RefCell::new(Vec::new()));
    let (child, _) = document("button");
    doc.set_sub_document(
        host,
        Box::new(FrameRecorder {
            doc: child,
            events: events.clone(),
        }),
    );
    doc.resolve(0.0);
    (doc, host, events)
}
fn frame_point(
    doc: &BaseDocument,
    host: NodeId,
    offset: f32,
) -> blitz_traits::events::BlitzPointerEvent {
    let bounds = doc.get_client_bounding_rect(host).unwrap();
    let mut pointer =
        doc.nodes[host].synthetic_click_event_data(keyboard_types::Modifiers::empty());
    pointer.coords.client_x = bounds.x as f32 + offset * 2.0;
    pointer.coords.client_y = bounds.y as f32 + offset * 2.0;
    pointer.coords.page_x = pointer.coords.client_x;
    pointer.coords.page_y = pointer.coords.client_y;
    pointer
}
#[test]
fn frame_edges_keep_border_and_padding_clicks_in_parent() {
    for offset in [1.0, 12.0] {
        let (mut doc, host, child_events) = bordered_frame();
        let pointer = frame_point(&doc, host, offset);
        let log = Log::default();
        for event in [
            UiEvent::PointerMove(pointer.clone()),
            UiEvent::PointerDown(pointer.clone()),
            UiEvent::PointerUp(pointer),
        ] {
            EventDriver::new(&mut doc, Recorder(log.clone()))
                .with_click_count(NonZeroU16::new(1).unwrap())
                .handle_ui_event(event);
        }
        assert!(
            child_events.borrow().is_empty(),
            "offset={offset}: border/padding forwarded into child"
        );
        assert_eq!(
            *log.borrow(),
            [("click", 1)],
            "offset={offset}: parent click swallowed"
        );
    }
}
#[test]
fn frame_edges_project_transformed_content_into_child_coordinates() {
    let (mut doc, host, child_events) = bordered_frame();
    let pointer = frame_point(&doc, host, 20.0);
    EventDriver::new(&mut doc, Recorder(Log::default()))
        .handle_ui_event(UiEvent::PointerMove(pointer));
    let events = child_events.borrow();
    assert_eq!(events.len(), 1);
    let UiEvent::PointerMove(pointer) = &events[0] else {
        panic!("forwarded pointer move")
    };
    assert!(
        (pointer.coords.client_x - 5.0).abs() < 0.01,
        "{}",
        pointer.coords.client_x
    );
    assert!(
        (pointer.coords.client_y - 5.0).abs() < 0.01,
        "{}",
        pointer.coords.client_y
    );
    assert!((pointer.coords.page_x - 5.0).abs() < 0.01);
    assert!((pointer.coords.page_y - 5.0).abs() < 0.01);
}
