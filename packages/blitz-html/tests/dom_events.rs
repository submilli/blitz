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

type DispatchView = (
    u64,
    Option<EventTargetId>,
    Option<EventTargetId>,
    Vec<EventTargetId>,
);

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
    composed: bool,
    related: Option<EventTargetId>,
    target: Option<EventTargetId>,
    path: Vec<EventTargetId>,
    views: Vec<DispatchView>,
    detach: Option<blitz_dom::NodeId>,
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
            composed: false,
            related: None,
            target: None,
            path: Vec::new(),
            views: Vec::new(),
            detach: None,
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
    fn composed(&self) -> bool {
        self.composed
    }
    fn related_target(&self) -> Option<EventTargetId> {
        self.related
    }
    fn set_related_target(&mut self, target: Option<EventTargetId>) {
        self.related = target;
    }
    fn set_composed_path(&mut self, path: Vec<EventTargetId>) {
        self.path = path;
    }
    fn set_target(&mut self, target: Option<EventTargetId>) {
        self.target = target;
    }
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
        self.views.push((
            listener.callback,
            self.target,
            self.related,
            self.path.clone(),
        ));
        if let Some(node) = self.detach.take() {
            self.doc.borrow_mut().mutate().remove_node(node);
        }
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

#[test]
fn closed_paths_and_related_targets_use_pre_dispatch_topology() {
    use blitz_dom::shadow::ShadowRootInit;
    let mut doc = parse("<body><div id=host></div><i id=outside></i></body>");
    let host = q(&doc, "#host");
    let outside = q(&doc, "#outside");
    let root = doc
        .mutate()
        .attach_shadow(
            host,
            ShadowRootInit {
                open: false,
                ..Default::default()
            },
        )
        .unwrap();
    doc.mutate()
        .set_inner_html_detaching(root, "<b id=inside></b><i></i>");
    let inside = doc.get_node(root).unwrap().children[0];
    let sibling = doc.get_node(root).unwrap().children[1];
    for (id, callback) in [(inside, 1), (root, 2), (host, 3)] {
        doc.event_listeners.add(
            EventTargetId::Node(id),
            "test",
            callback,
            ListenerOptions::default(),
        );
    }
    let doc = Rc::new(RefCell::new(doc));
    let mut runner = Host::new(doc.clone(), "test", true);
    runner.composed = true;
    runner.related = Some(EventTargetId::Node(outside));
    runner.detach = Some(inside);
    dispatch(&mut runner, EventTargetId::Node(inside));
    assert_eq!(runner.views.len(), 3);
    assert!(runner.views[0].3.contains(&EventTargetId::Node(root)));
    assert!(!runner.views[2].3.contains(&EventTargetId::Node(root)));
    assert!(!runner.views[2].3.contains(&EventTargetId::Node(inside)));
    assert_eq!(runner.views[2].1, Some(EventTargetId::Node(host)));
    assert_eq!(runner.target, Some(EventTargetId::Node(host)));
    doc.borrow_mut()
        .mutate()
        .pre_insert(inside, root, None)
        .unwrap();
    runner.views.clear();
    runner.related = Some(EventTargetId::Node(sibling));
    dispatch(&mut runner, EventTargetId::Node(inside));
    assert_eq!(
        runner.views.iter().map(|v| v.0).collect::<Vec<_>>(),
        vec![1, 2]
    );
}
#[test]
fn noncomposed_slotted_event_reaches_its_light_tree() {
    use blitz_dom::shadow::ShadowRootInit;
    let mut doc = parse("<body><div id=host><b id=light></b></div></body>");
    let host = q(&doc, "#host");
    let light = q(&doc, "#light");
    let root = doc
        .mutate()
        .attach_shadow(host, ShadowRootInit::default())
        .unwrap();
    doc.mutate().set_inner_html_detaching(root, "<slot></slot>");
    let slot = doc.get_node(root).unwrap().children[0];
    let path = doc.composed_event_path(EventTargetId::Node(light), false);
    assert!(path.contains(&EventTargetId::Node(slot)));
    assert!(path.contains(&EventTargetId::Node(host)));
    assert_eq!(path.last(), Some(&EventTargetId::Window));
}
#[test]
fn flattened_slot_fallback_preserves_order_and_bounds() {
    use blitz_dom::shadow::ShadowRootInit;
    let mut doc = parse("<body><div id=host></div></body>");
    let host = q(&doc, "#host");
    let root = doc
        .mutate()
        .attach_shadow(host, ShadowRootInit::default())
        .unwrap();
    doc.mutate().set_inner_html_detaching(root,"<slot><slot><b id=a></b></slot><i id=b></i><slot><em id=c></em></slot><!-- omitted --></slot>");
    let slot = doc.get_node(root).unwrap().children[0];
    let flat = doc.flattened_assigned_nodes(slot, 32).unwrap();
    let names: Vec<_> = flat
        .iter()
        .map(|&id| doc.attribute_by_name(id, "id").unwrap().value.to_string())
        .collect();
    assert_eq!(names, vec!["a", "b", "c"]);
    assert!(doc.flattened_assigned_nodes(slot, 2).is_err());
}

#[test]
fn shadow_stylesheet_retention_tracks_active_owner() {
    use blitz_dom::shadow::ShadowRootInit;
    let mut doc = parse("<body><div id=host></div></body>");
    let host = q(&doc, "#host");
    let root = doc
        .mutate()
        .attach_shadow(host, ShadowRootInit::default())
        .unwrap();
    doc.mutate()
        .set_inner_html_detaching(root, "<style>b{color:red}</style><b></b>");
    let owner = doc.get_node(root).unwrap().children[0];
    let sheet = doc.retain_stylesheet(owner).unwrap();
    assert!(doc.stylesheet_is_current(&sheet));
    assert_eq!(doc.retained_stylesheet_rule_count(&sheet, &[]), Some(1));
    let styled = doc.get_node(root).unwrap().children[1];
    doc.resolve(0.0);
    assert_eq!(doc.resolved_style_value(styled, "color"), "rgb(255, 0, 0)");
    doc.retained_stylesheet_insert_rule(&sheet, &[], "b{color:blue}", 1)
        .unwrap();
    doc.resolve(0.0);
    assert_eq!(doc.resolved_style_value(styled, "color"), "rgb(0, 0, 255)");
    doc.retained_stylesheet_delete_rule(&sheet, &[], 1).unwrap();
    doc.resolve(0.0);
    assert_eq!(doc.resolved_style_value(styled, "color"), "rgb(255, 0, 0)");
    doc.mutate().remove_node(owner);
    assert!(!doc.stylesheet_is_current(&sheet));
    assert_eq!(doc.retained_stylesheet_rule_count(&sheet, &[]), Some(1));
}

#[test]
fn deeply_nested_closed_roots_dispatch_without_per_ancestor_path_copies() {
    use blitz_dom::shadow::ShadowRootInit;
    let mut doc = parse("<body><div id=host></div></body>");
    let outside = q(&doc, "#host");
    let mut host = outside;
    for _ in 0..256 {
        let root = doc
            .mutate()
            .attach_shadow(host, ShadowRootInit::default())
            .unwrap();
        doc.mutate().set_inner_html_detaching(root, "<div></div>");
        host = doc.get_node(root).unwrap().children[0];
    }
    doc.event_listeners.add(
        EventTargetId::Node(outside),
        "test",
        1,
        ListenerOptions::default(),
    );
    let mut runner = Host::new(Rc::new(RefCell::new(doc)), "test", true);
    runner.composed = true;
    dispatch(&mut runner, EventTargetId::Node(host));
    assert_eq!(runner.views.len(), 1);
    assert!(!runner.views[0].3.contains(&EventTargetId::Node(host)));
    assert_eq!(runner.views[0].1, Some(EventTargetId::Node(outside)));
}
