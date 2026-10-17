//! Bounded transition-completion notifications, produced by style and consumed by script.
use crate::{BaseDocument, NodeId, NodeLease};
use std::collections::VecDeque;

pub(crate) const MAX_TRANSITION_ENDS: usize = 1024;

/// A completed transition retains its target until the embedder dispatches it.
pub struct TransitionEnd {
    pub target: NodeLease,
    pub property_name: String,
    pub pseudo_element: String,
    pub elapsed_time: f64,
    pub(crate) scheduled_end: f64,
    pub(crate) generation: u64,
}
#[derive(Default)]
pub(crate) struct TransitionEnds {
    pending: VecDeque<TransitionEnd>,
    pub(crate) overflowed: bool,
}
impl BaseDocument {
    pub(crate) fn queue_transition_end(
        &mut self,
        node: NodeId,
        property_name: String,
        pseudo_element: String,
        elapsed_time: f64,
        scheduled_end: f64,
        generation: u64,
    ) {
        if self.transition_ends.pending.len() >= MAX_TRANSITION_ENDS {
            self.transition_ends.overflowed = true;
            return;
        }
        let target = self.lease_node(node);
        self.transition_ends.pending.push_back(TransitionEnd {
            target,
            property_name,
            pseudo_element,
            elapsed_time,
            scheduled_end,
            generation,
        });
    }
    /// Advance time without resolving style or dispatching script. Synchronous
    /// geometry reads use the same committed clock as rendering opportunities.
    pub fn advance_animation_time(&mut self, seconds: f64) {
        if seconds.is_finite() {
            self.animation_time = self.animation_time.max(seconds);
        }
    }
    /// Drain only notifications already produced; dispatch must release the DOM borrow.
    /// Scheduled times sort first, then CSS transition composite order within a tree.
    /// https://www.w3.org/TR/css-transitions-2/#animation-composite-order
    pub fn take_transition_ends(&mut self) -> Vec<TransitionEnd> {
        let mut events: Vec<_> = self.transition_ends.pending.drain(..).collect();
        events.sort_by(|a, b| {
            a.scheduled_end
                .total_cmp(&b.scheduled_end)
                .then_with(|| {
                    let (a, b) = (a.target.id(), b.target.id());
                    let (ar, br) = (self.tree_root(a), self.tree_root(b));
                    if ar == br {
                        self.compare_document_order(a, b)
                    } else {
                        ar.cmp(&br)
                    }
                })
                .then_with(|| pseudo_order(&a.pseudo_element).cmp(&pseudo_order(&b.pseudo_element)))
                .then_with(|| a.generation.cmp(&b.generation))
                .then_with(|| a.property_name.cmp(&b.property_name))
        });
        events
    }
    pub fn take_transition_end_overflow(&mut self) -> bool {
        std::mem::take(&mut self.transition_ends.overflowed)
    }
    /// Active animations and queued completions need a rendering opportunity.
    /// A synchronous style read can finish a transition before event dispatch.
    pub fn has_pending_css_animation_work(&self) -> bool {
        self.has_active_animations || !self.transition_ends.pending.is_empty()
    }
}

// Pseudo-elements surround the owning element and its children in composite order.
// https://www.w3.org/TR/css-transitions-2/#animation-composite-order
fn pseudo_order(pseudo: &str) -> (u8, &str) {
    (
        match pseudo {
            "" => 0,
            "::marker" => 1,
            "::before" => 2,
            "::after" => 4,
            _ => 3,
        },
        pseudo,
    )
}
