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
        if list.iter().any(|l| {
            l.event_type == event_type && l.callback == callback && l.capture == options.capture
        }) {
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
        let pos = list.iter().position(|l| {
            l.event_type == event_type && l.callback == callback && l.capture == capture
        })?;
        let listener = list.remove(pos);
        listener.removed.set(true);
        if list.is_empty() {
            self.map.remove(&target);
        }
        Some(listener.callback)
    }

    fn remove_entry(&mut self, target: EventTargetId, listener: &Rc<Listener>) {
        if let Some(list) = self.map.get_mut(&target) {
            list.retain(|l| !Rc::ptr_eq(l, listener));
            if list.is_empty() {
                self.map.remove(&target);
            }
        }
        listener.removed.set(true);
    }

    /// Forget a dead target's whole bucket, invalidating snapshots retained by
    /// an in-progress dispatch just like explicit listener removal does.
    pub fn remove_target(&mut self, target: EventTargetId) {
        if let Some(listeners) = self.map.remove(&target) {
            for listener in listeners {
                listener.removed.set(true);
            }
        }
    }

    /// A snapshot of the listeners for `event_type` on `target`.
    pub fn listeners(&self, target: EventTargetId, event_type: &str) -> Vec<Rc<Listener>> {
        self.map
            .get(&target)
            .map(|list| {
                list.iter()
                    .filter(|l| l.event_type == event_type)
                    .cloned()
                    .collect()
            })
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
    /// True only for MouseEvent click dispatch that owns element activation.
    /// Native driver dispatch returns false because the driver owns its defaults.
    fn handles_click_activation(&self) -> bool {
        false
    }
    /// Embedder dispatches trusted input/change after successful activation.
    fn checkable_activated(&mut self, _target: NodeId) {}
    /// Coordinates for activation of a script-dispatched MouseEvent, in viewport
    /// CSS pixels. Native/programmatic driver dispatch already owns its defaults.
    fn activation_coordinates(&self) -> Option<(f64, f64)> {
        None
    }
    /// Run validation and cancelable submit/reset without retaining a DOM borrow.
    fn form_activated(&mut self, _action: crate::FormAction) {}
    /// Clear dispatch-only state before activation callbacks can redispatch the
    /// same event. Embedders retain continuation roots until dispatch returns.
    fn dispatch_finished(&mut self) {}
    /// Whether the event crosses shadow boundaries (`composed`): user input
    /// events do, script events only when created so.
    fn composed(&self) -> bool {
        false
    }
    /// The original related target, before retargeting for a listener.
    fn related_target(&self) -> Option<EventTargetId> {
        None
    }
    fn set_related_target(&mut self, _target: Option<EventTargetId>) {}
    /// Snapshot visible to the listener whose current target is about to be set.
    fn set_composed_path(&mut self, _path: Vec<EventTargetId>) {}
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
        self.composed_event_path(target, false)
    }

    /// The event path through shadow trees: a slotted node's parent is its
    /// slot, and a shadow root's is its host when the event is `composed`
    /// (otherwise the path ends at the shadow root).
    pub fn composed_event_path(&self, target: EventTargetId, composed: bool) -> Vec<EventTargetId> {
        let EventTargetId::Node(node) = target else {
            return vec![target];
        };
        let mut path = Vec::new();
        let mut current = Some(node);
        let mut reached_document = false;
        while let Some(id) = current {
            path.push(EventTargetId::Node(id));
            let Some(n) = self.get_node(id) else { break };
            current = if let Some(data) = &n.shadow_root_data {
                if !composed && self.tree_root(node) == id {
                    break;
                }
                Some(data.host)
            } else if let Some(slot) = n.parent.and_then(|_| self.assigned_slot(id)) {
                Some(slot)
            } else {
                n.parent
            };
            if current.is_none() && id == self.root_node().id {
                reached_document = true;
            }
        }
        if reached_document {
            path.push(EventTargetId::Window);
        }
        path
    }

    /// Retarget `target` against `current` (the DOM's "retarget"): while the
    /// target is in a shadow tree that does not contain `current`, the target
    /// becomes that tree's host. Listeners outside a component see its host.
    pub fn retarget(&self, target: EventTargetId, current: EventTargetId) -> EventTargetId {
        let EventTargetId::Node(mut node) = target else {
            return target;
        };
        loop {
            let root = self.tree_root(node);
            let Some(host) = self.shadow_host_of(root) else {
                return EventTargetId::Node(node);
            };
            if let EventTargetId::Node(current) = current
                && self.is_shadow_including_inclusive_ancestor(root, current)
            {
                return EventTargetId::Node(node);
            }
            node = host;
        }
    }

    /// Whether `ancestor` is `node` or one of its ancestors, crossing from
    /// shadow roots to their hosts.
    pub fn is_shadow_including_inclusive_ancestor(&self, ancestor: NodeId, node: NodeId) -> bool {
        let mut current = Some(node);
        while let Some(id) = current {
            if id == ancestor {
                return true;
            }
            let Some(n) = self.get_node(id) else {
                return false;
            };
            current = n.shadow_root_data.as_ref().map(|d| d.host).or(n.parent);
        }
        false
    }
}

/// One immutable dispatch entry. Topology and retargeting are captured before script.
struct PathEntry {
    current: EventTargetId,
    target: EventTargetId,
    related: Option<EventTargetId>,
    closed_root: Option<NodeId>,
}

impl BaseDocument {
    fn closest_closed_root(&self, target: EventTargetId) -> Option<NodeId> {
        let EventTargetId::Node(mut node) = target else {
            return None;
        };
        loop {
            let root = self.tree_root(node);
            let data = self.get_node(root)?.shadow_root_data.as_ref()?;
            if !data.open {
                return Some(root);
            }
            node = data.host;
        }
    }
}

/// Shared parent links keep snapshot storage linear even with nested closed roots.
fn closed_ancestors(doc: &BaseDocument, path: &[PathEntry]) -> HashMap<NodeId, Option<NodeId>> {
    let mut parents = HashMap::new();
    for entry in path {
        let mut current = entry.closed_root;
        while let Some(root) = current {
            if parents.contains_key(&root) {
                break;
            }
            let parent = doc
                .shadow_host_of(root)
                .and_then(|host| doc.closest_closed_root(EventTargetId::Node(host)));
            parents.insert(root, parent);
            current = parent;
        }
    }
    parents
}
fn visible_path(
    path: &[PathEntry],
    entry: &PathEntry,
    parents: &HashMap<NodeId, Option<NodeId>>,
) -> Vec<EventTargetId> {
    let mut visible_roots = std::collections::HashSet::new();
    let mut current = entry.closed_root;
    while let Some(root) = current {
        visible_roots.insert(root);
        current = parents.get(&root).copied().flatten();
    }
    path.iter()
        .filter(|candidate| {
            candidate
                .closed_root
                .is_none_or(|root| visible_roots.contains(&root))
        })
        .map(|candidate| candidate.current)
        .collect()
}

fn prepare_entry(
    host: &mut dyn DispatchHost,
    path: &[PathEntry],
    entry: &PathEntry,
    phase: EventPhase,
    closed_parents: &HashMap<NodeId, Option<NodeId>>,
) {
    host.set_target(Some(entry.target));
    host.set_related_target(entry.related);
    host.set_composed_path(visible_path(path, entry, closed_parents));
    host.set_phase(phase, Some(entry.current));
}

/// Dispatch from a snapshot. Reparenting inside a listener cannot reveal a closed root.
pub fn dispatch(host: &mut dyn DispatchHost, target: EventTargetId) -> bool {
    let related = host.related_target();
    let (path, closed_parents, outside, outside_related) = {
        let doc = host.document();
        let path: Vec<PathEntry> = doc
            .composed_event_path(target, host.composed())
            .into_iter()
            .map(|current| PathEntry {
                current,
                target: doc.retarget(target, current),
                related: related.map(|r| doc.retarget(r, current)),
                closed_root: doc.closest_closed_root(current),
            })
            .filter(|entry| entry.related != Some(entry.target))
            .collect();
        let parents = closed_ancestors(&doc, &path);
        (
            path,
            parents,
            doc.retarget(target, EventTargetId::Window),
            related.map(|r| doc.retarget(r, EventTargetId::Window)),
        )
    };
    let form_target = if host.handles_click_activation() {
        let doc = host.document();
        path.iter()
            .take(if host.bubbles() { path.len() } else { 1 })
            .filter_map(|entry| match entry.current {
                EventTargetId::Node(id) => Some(id),
                _ => None,
            })
            .find(|&id| {
                doc.get_node(id)
                    .and_then(|n| n.element_data())
                    .is_some_and(|e| {
                        e.name.ns == crate::ns!(html)
                            && matches!(&*e.name.local, "input" | "button")
                    })
            })
    } else {
        None
    };
    let activation = if host.handles_click_activation() {
        let mut doc = host.document_mut();
        let activation_target = path
            .iter()
            .take(if host.bubbles() { path.len() } else { 1 })
            .filter_map(|entry| match entry.current {
                EventTargetId::Node(id) => Some(id),
                _ => None,
            })
            .find(|&id| doc.is_checkable_input(id));
        activation_target.and_then(|id| doc.begin_checkable_activation(id))
    } else {
        None
    };
    host.set_target(Some(target));
    for entry in path.iter().skip(1).rev() {
        if host.propagation_stopped() {
            break;
        }
        let phase = if entry.target == entry.current {
            EventPhase::AtTarget
        } else {
            EventPhase::Capturing
        };
        if has_listeners(host, entry.current, true) {
            prepare_entry(host, &path, entry, phase, &closed_parents);
            invoke_listeners(host, entry.current, Some(true));
        }
    }
    if !host.propagation_stopped()
        && let Some(entry) = path.first()
    {
        prepare_entry(host, &path, entry, EventPhase::AtTarget, &closed_parents);
        invoke_listeners(host, entry.current, Some(true));
        if !host.propagation_stopped() {
            invoke_listeners(host, entry.current, Some(false));
        }
    }
    for entry in path.iter().skip(1) {
        if host.propagation_stopped() {
            break;
        }
        let at_target = entry.target == entry.current;
        if !host.bubbles() && !at_target {
            continue;
        }
        let phase = if at_target {
            EventPhase::AtTarget
        } else {
            EventPhase::Bubbling
        };
        if has_listeners(host, entry.current, false) {
            prepare_entry(host, &path, entry, phase, &closed_parents);
            invoke_listeners(host, entry.current, Some(false));
        }
    }
    host.set_target(Some(outside));
    host.set_related_target(outside_related);
    host.set_composed_path(Vec::new());
    host.set_phase(EventPhase::None, None);
    host.clear_propagation_flags();
    host.dispatch_finished();
    let canceled = host.canceled();
    let activated = activation.and_then(|a| a.finish(&mut host.document_mut(), canceled));
    let action = if !canceled && activated.is_none() {
        let coordinates = host.activation_coordinates();
        form_target.and_then(|id| host.document_mut().activate_form_control(id, coordinates))
    } else {
        None
    };
    let _action_roots: Vec<_> = {
        let doc = host.document();
        action
            .into_iter()
            .flat_map(|a| a.nodes())
            .map(|id| doc.lease_node(id))
            .collect()
    };
    if let Some(target) = activated {
        host.checkable_activated(target);
    }
    if let Some(action) = action {
        host.form_activated(action);
    }
    !host.canceled()
}

fn has_listeners(host: &dyn DispatchHost, target: EventTargetId, capture: bool) -> bool {
    host.document()
        .event_listeners
        .listeners(target, &host.event_type())
        .iter()
        .any(|listener| listener.capture == capture)
}

/// "Inner invoke": run matching listeners registered on `target` (with the
/// given capture flag), in order, against a snapshot of the list.
fn invoke_listeners(host: &mut dyn DispatchHost, target: EventTargetId, capture: Option<bool>) {
    let event_type = host.event_type();
    let listeners = host
        .document()
        .event_listeners
        .listeners(target, &event_type);
    for listener in listeners {
        if listener.removed.get() {
            continue;
        }
        if capture.is_some_and(|c| c != listener.capture) {
            continue;
        }
        if listener.once {
            host.document_mut()
                .event_listeners
                .remove_entry(target, &listener);
        }
        host.set_in_passive_listener(listener.passive);
        host.invoke(&listener);
        host.set_in_passive_listener(false);
        if host.immediate_propagation_stopped() {
            break;
        }
    }
}

#[cfg(test)]
mod lifetime_tests {
    use super::*;

    #[test]
    fn removal_and_once_dispatch_release_empty_target_buckets() {
        let mut listeners = EventListeners::default();
        for id in 0..20_000 {
            let target = EventTargetId::Other(id);
            listeners.add(target, "x", id, ListenerOptions::default());
            let snapshot = listeners.listeners(target, "x");
            if id % 2 == 0 {
                listeners.remove(target, "x", id, false);
            } else {
                listeners.remove_entry(target, &snapshot[0]);
            }
            assert!(snapshot[0].removed.get());
            assert!(listeners.map.is_empty());
        }
    }

    #[test]
    fn dead_target_teardown_invalidates_all_dispatch_snapshots() {
        let mut listeners = EventListeners::default();
        let target = EventTargetId::Other(1);
        listeners.add(target, "x", 1, ListenerOptions::default());
        listeners.add(target, "x", 2, ListenerOptions::default());
        let snapshot = listeners.listeners(target, "x");
        listeners.remove_target(target);
        assert!(snapshot.iter().all(|listener| listener.removed.get()));
        assert!(listeners.map.is_empty());
    }
}
