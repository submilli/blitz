//! Keypress and cancellable editing admission before a native key default.
use super::{EventDriver, EventHandler};
use crate::NodeId;
use blitz_traits::events::{DomEvent, DomEventData, EventState};
impl<Handler: EventHandler> EventDriver<'_, Handler> {
    pub(super) fn prepare_text_input(&mut self, event: &DomEvent) -> bool {
        let DomEventData::KeyDown(key) = &event.data else {
            return false;
        };
        if !printable_key(key) || !self.is_focused_target(event.target) {
            return false;
        }
        let mut press = DomEvent::new(event.target, DomEventData::KeyPress(key.clone()));
        let state = self.run_handler_event(&mut press, EventState::default());
        if state.is_cancelled() || !self.is_focused_target(event.target) {
            return false;
        }
        let editable = self
            .doc
            .inner()
            .get_node(event.target)
            .and_then(crate::Node::element_data)
            .is_some_and(|element| element.text_input_data().is_some());
        !editable || self.handler.before_input(event.target, key)
    }

    fn is_focused_target(&self, target: NodeId) -> bool {
        self.handler.placement(target) == crate::Placement::Here(target)
            && self.doc.inner().get_focussed_node_id() == Some(target)
    }
}

pub(super) fn printable_key(key: &blitz_traits::events::BlitzKeyEvent) -> bool {
    matches!(
        key.key,
        keyboard_types::Key::Character(_) | keyboard_types::Key::Enter
    ) && !key.modifiers.intersects(
        keyboard_types::Modifiers::CONTROL
            | keyboard_types::Modifiers::META
            | keyboard_types::Modifiers::ALT,
    )
}
