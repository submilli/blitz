//! A native event whose listeners have run, with the steps that follow them.
//! They act on the event's target where it lives when the listeners return
//! (see [`crate::Placement`]).

use crate::{BaseDocument, CheckableActivation, NodeId};
use blitz_traits::events::{DomEvent, DomEventData};

/// The rest of a native event once its listeners have returned: its
/// checkable activation, and its default action unless a listener canceled.
pub struct ListenedEvent {
    pub(super) event: DomEvent,
    pub(super) activation: Option<CheckableActivation>,
    pub(super) canceled: bool,
}

impl ListenedEvent {
    /// The node the event was dispatched to, by its ID in the dispatching
    /// document.
    pub fn target(&self) -> NodeId {
        self.event.target
    }

    /// This event in `doc`. `map` gives a node's ID in `doc`, or `None` when
    /// the node is not there; `None` when the target is not there.
    pub fn relocate(
        self,
        doc: &BaseDocument,
        mut map: impl FnMut(NodeId) -> Option<NodeId>,
    ) -> Option<Self> {
        let mut event = self.event;
        event.target = map(event.target)?;
        match &mut event.data {
            DomEventData::Focus(data)
            | DomEventData::Blur(data)
            | DomEventData::FocusIn(data)
            | DomEventData::FocusOut(data) => {
                data.related_target = data.related_target.and_then(&mut map);
            }
            _ => {}
        }
        let activation = self
            .activation
            .and_then(|activation| activation.relocate(doc, &mut map));
        Some(Self {
            event,
            activation,
            canceled: self.canceled,
        })
    }
}
