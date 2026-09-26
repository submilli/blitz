//! DOM Standard events: listener storage and the dispatch algorithm.
//!
//! Engine-agnostic: listeners are opaque `u64` callback handles owned by the
//! embedder (e.g. a JS function table), and the event object's flags live
//! with the embedder too. [`dispatch`] runs the algorithm from
//! <https://dom.spec.whatwg.org/#concept-event-dispatch> and calls back into
//! the embedder through [`DispatchHost`] for everything engine-specific.
//!
//! The dispatch loop never holds a borrow of the document while invoking a
//! listener, so listeners may freely mutate the DOM (including adding and
//! removing listeners) during dispatch.

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::{BaseDocument, NodeId};

/// Something events can be dispatched to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventTargetId {
    Node(NodeId),
    /// The document's `Window`.
    Window,
    /// A non-node target created by the embedder (e.g. `new EventTarget()`),
    /// identified by an embedder-assigned id.
    Other(u64),
}

/// A registered event listener.
#[derive(Debug)]
pub struct Listener {
    pub event_type: String,
    /// The embedder's handle for the callback.
    pub callback: u64,
    pub capture: bool,
    pub passive: bool,
    pub once: bool,
    /// Set when removed, so an in-progress dispatch skips it.
    pub removed: Cell<bool>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ListenerOptions {
    pub capture: bool,
    pub passive: bool,
    pub once: bool,
}

/// Listener lists per target, in registration order.
#[derive(Default)]
pub struct EventListeners {
    map: HashMap<EventTargetId, Vec<Rc<Listener>>>,
}

impl EventListeners {
    /// `addEventListener`: returns false (and adds nothing) if an equal
    /// listener (same type, callback and capture) is already registered.
    pub fn add(
        &mut self,
        target: EventTargetId,
        event_type: &str,
        callback: u64,
        options: ListenerOptions,
    ) -> bool {
        let list = self.map.entry(target).or_default();
        if list
            .iter()
            .any(|l| l.event_type == event_type && l.callback == callback && l.capture == options.capture)
        {
            return false;
        }
        list.push(Rc::new(Listener {
            event_type: event_type.to_string(),
            callback,
            capture: options.capture,
            passive: options.passive,
            once: options.once,
            removed: Cell::new(false),
        }));
        true
    }

    /// `removeEventListener`: returns the removed listener's callback handle.
    pub fn remove(
        &mut self,
        target: EventTargetId,
        event_type: &str,
        callback: u64,
        capture: bool,
    ) -> Option<u64> {
        let list = self.map.get_mut(&target)?;
        let pos = list
            .iter()
            .position(|l| l.event_type == event_type && l.callback == callback && l.capture == capture)?;
        let listener = list.remove(pos);
        listener.removed.set(true);
        Some(listener.callback)
    }

    fn remove_entry(&mut self, target: EventTargetId, listener: &Rc<Listener>) {
        if let Some(list) = self.map.get_mut(&target) {
            list.retain(|l| !Rc::ptr_eq(l, listener));
        }
        listener.removed.set(true);
    }

    /// A snapshot of the listeners for `event_type` on `target`.
    pub fn listeners(&self, target: EventTargetId, event_type: &str) -> Vec<Rc<Listener>> {
        self.map
            .get(&target)
            .map(|list| list.iter().filter(|l| l.event_type == event_type).cloned().collect())
            .unwrap_or_default()
    }

    /// Callback handles of every listener on `target` (e.g. to release them
    /// when the target goes away).
    pub fn callbacks_of(&self, target: EventTargetId) -> Vec<u64> {
        self.map
            .get(&target)
            .map(|list| list.iter().map(|l| l.callback).collect())
            .unwrap_or_default()
    }
}

/// `Event.eventPhase`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventPhase {
    None = 0,
    Capturing = 1,
    AtTarget = 2,
    Bubbling = 3,
}

/// The embedder side of a dispatch: the event object's state and how to
/// call a listener.
pub trait DispatchHost {
    fn document(&self) -> std::cell::Ref<'_, BaseDocument>;
    fn document_mut(&self) -> std::cell::RefMut<'_, BaseDocument>;
    fn event_type(&self) -> String;
    fn bubbles(&self) -> bool;
    /// Update the event's `eventPhase` and `currentTarget`.
    fn set_phase(&mut self, phase: EventPhase, current_target: Option<EventTargetId>);
    fn set_target(&mut self, target: Option<EventTargetId>);
    fn set_in_passive_listener(&mut self, passive: bool);
    fn propagation_stopped(&self) -> bool;
    fn immediate_propagation_stopped(&self) -> bool;
    fn canceled(&self) -> bool;
    /// Reset the stop-propagation flags once dispatch completes.
    fn clear_propagation_flags(&mut self);
    /// Call the listener. Exceptions are the embedder's to report; the
    /// algorithm carries on with the next listener either way. A `once`
    /// listener has already been removed when this is called, so the
    /// embedder can release its callback handle afterwards.
    fn invoke(&mut self, listener: &Listener);
}

impl BaseDocument {
    /// The event path for `target`: the target, its ancestors, and (for
    /// nodes in the document) the `Window` last.
    pub fn event_path(&self, target: EventTargetId) -> Vec<EventTargetId> {
        let EventTargetId::Node(node) = target else {
            return vec![target];
        };
        let mut path = Vec::new();
        let mut current = Some(node);
        while let Some(id) = current {
            path.push(EventTargetId::Node(id));
            current = self.get_node(id).and_then(|n| n.parent);
        }
        if self.is_connected(node) {
            path.push(EventTargetId::Window);
        }
        path
    }
}

/// Dispatch the event to `target`. Returns false if it was canceled
/// (`preventDefault()` in a non-passive listener of a cancelable event).
pub fn dispatch(host: &mut dyn DispatchHost, target: EventTargetId) -> bool {
    host.set_target(Some(target));
    let path = host.document().event_path(target);
    let bubbles = host.bubbles();

    // Capturing phase: root to target (excluding the target).
    for &current in path.iter().skip(1).rev() {
        if host.propagation_stopped() {
            break;
        }
        host.set_phase(EventPhase::Capturing, Some(current));
        invoke_listeners(host, current, Some(true));
    }

    // At target: capture listeners first, then non-capture, as the DOM
    // Standard now specifies.
    if !host.propagation_stopped() {
        host.set_phase(EventPhase::AtTarget, Some(target));
        invoke_listeners(host, target, Some(true));
        if !host.propagation_stopped() {
            invoke_listeners(host, target, Some(false));
        }
    }

    // Bubbling phase: target's parent to root.
    if bubbles {
        for &current in path.iter().skip(1) {
            if host.propagation_stopped() {
                break;
            }
            host.set_phase(EventPhase::Bubbling, Some(current));
            invoke_listeners(host, current, Some(false));
        }
    }

    host.set_phase(EventPhase::None, None);
    host.clear_propagation_flags();
    !host.canceled()
}

/// "Inner invoke": run matching listeners registered on `target` (with the
/// given capture flag), in order, against a snapshot of the list.
fn invoke_listeners(host: &mut dyn DispatchHost, target: EventTargetId, capture: Option<bool>) {
    let event_type = host.event_type();
    let listeners = host.document().event_listeners.listeners(target, &event_type);
    for listener in listeners {
        if listener.removed.get() {
            continue;
        }
        if capture.is_some_and(|c| c != listener.capture) {
            continue;
        }
        if listener.once {
            host.document_mut().event_listeners.remove_entry(target, &listener);
        }
        host.set_in_passive_listener(listener.passive);
        host.invoke(&listener);
        host.set_in_passive_listener(false);
        if host.immediate_propagation_stopped() {
            break;
        }
    }
}
