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
