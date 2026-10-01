//! One resumable native focus transition for script and physical input.
use super::{EventDriver, EventHandler, GeneratedEvent};
use crate::{BaseDocument, NodeId, NodeLease};
use blitz_traits::events::{BlitzFocusEvent, DomEvent, DomEventData, EventState};

pub(crate) fn queue_focus(
    doc: &BaseDocument,
    target: Option<NodeId>,
    emit: &mut dyn FnMut(GeneratedEvent),
) {
    let target = target.and_then(|id| doc.nearest_non_anonymous_ancestor(id));
    if target != doc.focus_node_id {
        emit(GeneratedEvent::Focus(target));
    }
}

/// Holds both endpoints across callbacks. Each step releases the document before
/// the embedder delivers the returned event; nested focus changes supersede it.
pub struct FocusTransition {
    old: Option<NodeId>,
    target: Option<NodeId>,
    epoch: u64,
    phase: Phase,
    _retained: Vec<NodeLease>,
}
#[derive(Clone, Copy)]
enum Phase {
    Blur,
    FocusOut,
    Focus,
    FocusIn,
    Done,
}

impl BaseDocument {
    /// Start focus update steps with current style and native eligibility.
    pub fn begin_focus_transition(&mut self, target: Option<NodeId>) -> Option<FocusTransition> {
        self.flush_style_and_layout(0.0);
        if target == self.focus_node_id
            || target.is_some_and(|id| {
                !self
                    .get_node(id)
                    .is_some_and(crate::Node::is_programmatically_focusable)
            })
        {
            return None;
        }
        let old = self.focus_node_id;
        let retained = old
            .into_iter()
            .chain(target)
            .map(|id| self.lease_node(id))
            .collect();
        self.clear_focus();
        self.shell_provider.request_redraw();
        Some(FocusTransition {
            old,
            target,
            epoch: self.focus_epoch,
            phase: Phase::Blur,
            _retained: retained,
        })
    }
}

impl FocusTransition {
    /// Advance one event at a time. Recheck generation and eligibility after
    /// callbacks; a superseded destination receives no stale focus events.
    pub fn next_event(&mut self, doc: &mut BaseDocument) -> Option<DomEvent> {
        loop {
            match self.phase {
                Phase::Blur => {
                    self.phase = Phase::FocusOut;
                    if let Some(old) = self.old {
                        return Some(event(old, self.target, DomEventData::Blur));
                    }
                }
                Phase::FocusOut => {
                    self.phase = Phase::Focus;
                    if let Some(old) = self.old {
                        let related = (doc.focus_epoch == self.epoch)
                            .then_some(self.target)
                            .flatten();
                        return Some(event(old, related, DomEventData::FocusOut));
                    }
                }
                Phase::Focus => {
                    self.phase = Phase::Done;
                    let target = self.target.filter(|_| doc.focus_epoch == self.epoch)?;
                    doc.flush_style_and_layout(0.0);
                    if !doc
                        .get_node(target)
                        .is_some_and(crate::Node::is_programmatically_focusable)
                    {
                        return None;
                    }
                    doc.set_focus_to(target);
                    doc.clamp_text_input_scroll(target);
                    doc.shell_provider.request_redraw();
                    self.epoch = doc.focus_epoch;
                    self.phase = Phase::FocusIn;
                    return Some(event(target, self.old, DomEventData::Focus));
                }
                Phase::FocusIn => {
                    self.phase = Phase::Done;
                    let target = self.target.filter(|_| doc.focus_epoch == self.epoch)?;
                    return Some(event(target, self.old, DomEventData::FocusIn));
                }
                Phase::Done => return None,
            }
        }
    }
}
fn event(
    target: NodeId,
    related_target: Option<NodeId>,
    kind: fn(BlitzFocusEvent) -> DomEventData,
) -> DomEvent {
    DomEvent::new(target, kind(BlitzFocusEvent { related_target }))
}

impl<Handler: EventHandler> EventDriver<'_, Handler> {
    pub(super) fn run_focus_transition(&mut self, target: Option<NodeId>) {
        let Some(mut transition) = self.doc.inner_mut().begin_focus_transition(target) else {
            return;
        };
        loop {
            let event = transition.next_event(&mut self.doc.inner_mut());
            let Some(mut event) = event else {
                break;
            };
            self.run_handler_event(&mut event, EventState::default());
        }
    }
}
