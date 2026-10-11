//! Activation that follows a node its listeners moved into another document.
//!
//! An embedder with a separate arena per document can let a listener adopt
//! the event's target into another document, which rebuilds the node there
//! under a new ID. The steps that follow the listeners (checkable activation,
//! form activation and default actions) belong to the node, not to the ID it
//! had: they run where the node lives when the listeners return, as in
//! [DOM § 2.9](https://dom.spec.whatwg.org/#concept-event-dispatch), where
//! the activation target is a node. The embedder says where that is
//! ([`Placement`]); Blitz relocates the pending steps
//! ([`crate::ClickActivation`], [`crate::ListenedEvent`]) into this document,
//! or hands them to the embedder to relocate into another one.

use crate::NodeId;

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

#[cfg(test)]
mod tests;
