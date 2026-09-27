//! Documents read time from the embedder's `Clock` (`DocumentConfig::clock`),
//! so time-dependent behaviour such as double-click detection follows it.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use blitz_dom::{BaseDocument, Clock, DocumentConfig, EventDriver, EventHandler};
use blitz_html::{DocumentHtmlParser, HtmlProvider};
use blitz_traits::events::{DomEvent, DomEventData, EventState, UiEvent};
use blitz_traits::node_id::NodeId;
use blitz_traits::shell::{ColorScheme, Viewport};
use common::q;

/// A clock the test advances by hand.
#[derive(Default)]
struct ManualClock(AtomicU64);

impl Clock for ManualClock {
    fn now_ms(&self) -> f64 {
        self.0.load(Ordering::Relaxed) as f64
    }
}

#[derive(Default)]
struct Recorder(Vec<&'static str>);

impl EventHandler for &mut Recorder {
    fn handle_event(
        &mut self,
        _: &[NodeId],
        event: &mut DomEvent,
        _: &mut dyn blitz_dom::Document,
        _: &mut EventState,
    ) {
        if matches!(event.data, DomEventData::DoubleClick(_)) {
            self.0.push("dblclick");
        }
    }
}

fn doc_with(clock: Arc<ManualClock>) -> BaseDocument {
    let mut doc = BaseDocument::new(DocumentConfig {
        viewport: Some(Viewport::new(800, 600, 1.0, ColorScheme::Light)),
        html_parser_provider: Some(Arc::new(HtmlProvider)),
        clock: Some(clock),
        ..Default::default()
    });
    DocumentHtmlParser::parse_into_mutator(
        &mut doc.mutate(),
        "<div id=target style='width:200px;height:100px'>x</div>",
    );
    doc.resolve(0.0);
    doc
}

/// Press and release the primary button on `#target` at the current time.
fn click(doc: &mut BaseDocument, recorder: &mut Recorder) {
    let event = doc
        .get_node(q(doc, "#target"))
        .unwrap()
        .synthetic_click_event_data(Default::default());
    let mut driver = EventDriver::new(doc, recorder);
    driver.handle_ui_event(UiEvent::PointerDown(event.clone()));
    driver.handle_ui_event(UiEvent::PointerUp(event));
}

#[test]
fn double_click_detection_follows_the_document_clock() {
    let clock = Arc::new(ManualClock::default());
    let mut doc = doc_with(clock.clone());
    let mut recorder = Recorder::default();

    clock.0.store(1_000, Ordering::Relaxed);
    click(&mut doc, &mut recorder);
    // 400 ms later: within the double-click interval.
    clock.0.store(1_400, Ordering::Relaxed);
    click(&mut doc, &mut recorder);
    assert_eq!(recorder.0, ["dblclick"]);

    // A long pause, then two clicks 600 ms apart: too slow.
    clock.0.store(5_000, Ordering::Relaxed);
    click(&mut doc, &mut recorder);
    clock.0.store(5_600, Ordering::Relaxed);
    click(&mut doc, &mut recorder);
    assert_eq!(recorder.0, ["dblclick"]);
}

#[test]
fn documents_expose_their_clock() {
    let clock = Arc::new(ManualClock::default());
    let doc = doc_with(clock.clone());
    clock.0.store(42, Ordering::Relaxed);
    assert_eq!(doc.now_ms(), 42.0);
}
