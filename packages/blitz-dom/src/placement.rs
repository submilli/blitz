//! Activation that follows a node its listeners moved into another document.
//!
//! An embedder with a separate arena per document can let a listener adopt
//! the event's target into another document, which rebuilds the node there
//! under a new ID. The steps that follow the listeners (checkable activation,
//! form activation and default actions) belong to the node, not to the ID it
//! had: they run where the node lives when the listeners return, as in
//! [DOM § 2.9](https://dom.spec.whatwg.org/#concept-event-dispatch), where
//! the activation target is a node. The embedder says where that is
//! ([`Placement`]); Blitz relocates the pending steps into this document, or
//! hands them to the embedder to relocate into another one.

use crate::{BaseDocument, CheckableActivation, FormAction, NodeId, NodeLease};

/// Where a node lives once an event's listeners return.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    /// In the dispatching document, under this ID: the node's own, unless a
    /// listener moved it into another document and back.
    Here(NodeId),
    /// In another document.
    Elsewhere,
}

impl Placement {
    /// The node's ID in the dispatching document, if it lives there.
    pub fn here(self) -> Option<NodeId> {
        match self {
            Self::Here(id) => Some(id),
            Self::Elsewhere => None,
        }
    }
}

/// A script click's activation behaviour, saved before its listeners run.
pub struct ClickActivation {
    checkable: Option<CheckableActivation>,
    form_target: Option<(NodeId, NodeLease)>,
    canceled: bool,
    coordinates: Option<(f64, f64)>,
}

/// What a click's activation did, for the embedder to complete: fire `input`
/// and `change` at a checkable input, and run a form action.
pub struct Activated {
    pub checkable: Option<NodeId>,
    pub form: Option<FormAction>,
    _form_leases: Vec<NodeLease>,
}

impl ClickActivation {
    pub(crate) fn new(
        checkable: Option<CheckableActivation>,
        form_target: Option<(NodeId, NodeLease)>,
        canceled: bool,
        coordinates: Option<(f64, f64)>,
    ) -> Self {
        Self {
            checkable,
            form_target,
            canceled,
            coordinates,
        }
    }

    /// The node whose placement decides where the activation runs.
    pub fn node(&self) -> Option<NodeId> {
        self.checkable
            .as_ref()
            .map(CheckableActivation::target)
            .or(self.form_target.as_ref().map(|&(id, _)| id))
    }

    /// This activation in `doc`. `map` gives a node's ID in `doc`, or `None`
    /// when the node is not there; a step whose node is not there is dropped.
    pub fn relocate(
        self,
        doc: &BaseDocument,
        mut map: impl FnMut(NodeId) -> Option<NodeId>,
    ) -> Self {
        let checkable = self
            .checkable
            .and_then(|activation| activation.relocate(doc, &mut map));
        let form_target = self
            .form_target
            .and_then(|(id, _)| map(id))
            .map(|id| (id, doc.lease_node(id)));
        Self {
            checkable,
            form_target,
            ..self
        }
    }

    /// Finish the checkable activation, or else activate the form control,
    /// in `doc`, where the activation's nodes live.
    pub fn run(self, doc: &mut BaseDocument) -> Activated {
        let checkable = self
            .checkable
            .and_then(|activation| activation.finish(doc, self.canceled));
        let form = if !self.canceled && checkable.is_none() {
            self.form_target
                .and_then(|(id, _)| doc.activate_form_control(id, self.coordinates))
        } else {
            None
        };
        let form_leases = form
            .into_iter()
            .flat_map(FormAction::nodes)
            .map(|id| doc.lease_node(id))
            .collect();
        Activated {
            checkable,
            form,
            _form_leases: form_leases,
        }
    }
}

#[cfg(test)]
mod tests;
