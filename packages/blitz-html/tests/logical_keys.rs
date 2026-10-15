//! Automation supplies logical keys without the desktop shell's text callbacks.
mod common;
use blitz_dom::{Document, EventDriver, EventHandler, NodeId};
use blitz_traits::events::{BlitzKeyEvent, DomEvent, DomEventData, EventState, KeyState};
use common::{laid_out, q};
use keyboard_types::{Code, Key, Location, Modifiers};

struct Cancel(bool);
impl EventHandler for Cancel {
    fn handle_event(
        &mut self,
        _: &[NodeId],
        event: &mut DomEvent,
        _: &mut dyn Document,
        state: &mut EventState,
    ) {
        if self.0 && matches!(event.data, DomEventData::KeyDown(_)) {
            state.prevent_default();
        }
    }
}

#[test]
fn logical_backspace_edits_after_listeners_and_respects_cancellation() {
    for cancelled in [false, true] {
        let mut doc = laid_out("<input id=i value=abc>");
        doc.resolve(0.0);
        let input = q(&doc, "#i");
        doc.set_focus_to(input);
        doc.mutate().set_control_selection(
            input,
            blitz_dom::form_selection::ControlSelection {
                start: 3,
                end: 3,
                ..Default::default()
            },
        );
        EventDriver::new(&mut doc, Cancel(cancelled)).handle_logical_key(BlitzKeyEvent {
            key: Key::Backspace,
            code: Code::Backspace,
            modifiers: Modifiers::empty(),
            location: Location::Standard,
            is_auto_repeating: false,
            is_composing: false,
            state: KeyState::Pressed,
            text: None,
        });
        assert_eq!(
            doc.form_value(input).as_deref(),
            Some(if cancelled { "abc" } else { "ab" })
        );
    }
}
