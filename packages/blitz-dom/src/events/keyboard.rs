use crate::events::focus::queue_focus;
use crate::{BaseDocument, node::GeneratedTextInputEvent, util::ACTION_MOD};
use blitz_traits::node_id::NodeId;
use blitz_traits::{
    SmolStr,
    events::{BlitzInputEvent, BlitzKeyEvent, DomEvent, DomEventData},
};
use keyboard_types::{Key, Modifiers};

pub(super) enum KeyboardOrTextInputEvent {
    KeyPress(BlitzKeyEvent),
    AppleStandardKeyBinding(SmolStr),
}

pub(crate) fn handle_key_or_input_event<F: FnMut(super::GeneratedEvent)>(
    doc: &mut BaseDocument,
    target: NodeId,
    event: KeyboardOrTextInputEvent,
    mut dispatch_event: F,
) -> Option<super::FormAction> {
    if let KeyboardOrTextInputEvent::KeyPress(event) = &event {
        if event.key == Key::Tab {
            doc.flush_style_and_layout(0.0);
            let backwards = event.modifiers.contains(Modifiers::SHIFT);
            if let Some(start) = doc.get_focussed_node_id()
                && let Some(target) = doc.sequential_focus_target(start, backwards)
            {
                queue_focus(doc, Some(target), &mut dispatch_event);
            }
            return None;
        }

        // Handle copy (Ctrl+C/Cmd+C) for text selection when no text input is focused
        if event.state.is_pressed() {
            let action_mod = event.modifiers.contains(ACTION_MOD);
            if action_mod {
                if let Key::Character(c) = &event.key {
                    if c.to_lowercase() == "c" {
                        // Check if we have a text selection (and no focused text input)
                        let has_focused_text_input = doc.focus_node_id.is_some_and(|id| {
                            doc.get_node(id)
                                .and_then(|n| n.element_data())
                                .is_some_and(|e| e.text_input_data().is_some())
                        });

                        if !has_focused_text_input {
                            if let Some(text) = doc.get_selected_text() {
                                let _ = doc.shell_provider.set_clipboard_text(text);
                                return None;
                            }
                        }
                    }
                }
            }
        }
    }

    if let Some(node_id) = doc.focus_node_id {
        if target != node_id {
            return None;
        }

        let node = &mut doc.nodes[node_id];
        let element_data = node.element_data_mut()?;

        if let Some(input_data) = element_data.text_input_data_mut() {
            let generated_event = match event {
                KeyboardOrTextInputEvent::KeyPress(blitz_key_event) => input_data
                    .apply_keypress_event(
                        &mut doc.font_ctx.lock().unwrap(),
                        &mut doc.layout_ctx,
                        &*doc.shell_provider,
                        blitz_key_event,
                    ),
                KeyboardOrTextInputEvent::AppleStandardKeyBinding(command) => input_data
                    .apply_apple_standard_keybinding(
                        &mut doc.font_ctx.lock().unwrap(),
                        &mut doc.layout_ctx,
                        &*doc.shell_provider,
                        &command,
                    ),
            };

            if let Some(generated_event) = generated_event {
                return doc.apply_generated_text_input_event(
                    node_id,
                    generated_event,
                    dispatch_event,
                );
            }
        }
    }
    None
}

impl BaseDocument {
    pub(crate) fn apply_generated_text_input_event<F: FnMut(super::GeneratedEvent)>(
        &mut self,
        node_id: NodeId,
        event: GeneratedTextInputEvent,
        mut dispatch_event: F,
    ) -> Option<super::FormAction> {
        let node = &mut self.nodes[node_id];
        let element_data = node
            .element_data_mut()
            .expect("apply_generated_text_input_event called on a node that is not an element");
        let input_data = element_data
            .text_input_data_mut()
            .expect("apply_generated_text_input_event called on a node that is not a text input");

        let edits = std::mem::take(&mut input_data.edits);
        let changed = !edits.is_empty();
        if changed {
            let value = element_data
                .form_state
                .value
                .get_or_insert_with(Default::default);
            for edit in edits {
                edit.apply(value);
            }
            element_data.form_state.value_dirty = true;
            element_data.form_state.last_change_by_user = true;
            let value = element_data.form_state.value.clone().unwrap_or_default();
            if let Some(input) = element_data.text_input_data_mut() {
                input.reproject(
                    &value,
                    &mut self.font_ctx.lock().unwrap(),
                    &mut self.layout_ctx,
                );
            }
        }
        self.capture_control_selection(node_id);
        let element_data = self.nodes[node_id]
            .element_data()
            .expect("text input is an element");
        match event {
            GeneratedTextInputEvent::Input if changed => {
                let value = element_data
                    .form_state
                    .value
                    .as_ref()
                    .map(|v| v.as_str_lossy().to_owned())
                    .unwrap_or_default();
                dispatch_event(
                    (DomEvent::new(node_id, DomEventData::Input(BlitzInputEvent { value }))).into(),
                );
                self.shell_provider.request_redraw();
            }
            GeneratedTextInputEvent::Input
            | GeneratedTextInputEvent::Select
            | GeneratedTextInputEvent::PreEditChange => {
                self.shell_provider.request_redraw();
            }
            GeneratedTextInputEvent::Submit => match self.implicit_submission(node_id) {
                Some(super::ImplicitSubmission::Click(button)) => {
                    let event = self.nodes[button].synthetic_click_event(Modifiers::empty());
                    dispatch_event((DomEvent::new(button, event)).into());
                }
                Some(super::ImplicitSubmission::Submit(form)) => {
                    return Some(super::FormAction::Submit {
                        form,
                        submitter: None,
                    });
                }
                None => {}
            },
        }
        None
    }
}
