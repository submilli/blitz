//! Native input dispatch and scalar rendering projection; no page callbacks.
use super::{CalendarEditor, Kind};
use crate::{BaseDocument, ElementData, NodeId};
use blitz_traits::events::{BlitzInputEvent, BlitzKeyEvent, DomEvent, DomEventData};
use keyboard_types::{Key, Modifiers};
use markup5ever::local_name;

pub(crate) fn kind(element: &ElementData) -> Option<Kind> {
    (element.name.ns == markup5ever::ns!(html) && &*element.name.local == "input")
        .then(|| element.attr(local_name!("type")).and_then(Kind::parse))
        .flatten()
}
impl BaseDocument {
    /// Supply the UTC presentation instant used to initialize an empty year.
    /// This bounded conversion never reads an ambient clock.
    pub fn set_calendar_reference_time(&mut self, unix_ms: f64) {
        let days = if unix_ms.is_finite() {
            (unix_ms / 86_400_000.0)
                .floor()
                .clamp(-719162.0, 100_000_000.0) as i64
        } else {
            0
        };
        let z = days + 719468;
        let era = z.div_euclid(146097);
        let doe = z - era * 146097;
        let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let month = (5 * doy + 2) / 153;
        self.calendar_reference_year =
            (yoe + era * 400 + i64::from(month >= 10)).clamp(1, 275760) as u32;
    }
    pub(crate) fn ensure_calendar_editor(&mut self, id: NodeId) -> bool {
        let Some(element) = self.nodes[id].element_data() else {
            return false;
        };
        let Some(kind) = kind(element) else {
            return false;
        };
        let value = self.form_value(id).unwrap_or_default();
        let Some(element) = self.nodes[id].element_data_mut() else {
            return false;
        };
        let mut editor = element
            .form_state
            .calendar
            .take()
            .filter(|c| c.kind == kind)
            .unwrap_or_else(|| {
                CalendarEditor::new(kind, &value, element.attr(local_name!("step")))
            });
        editor.update_precision(&value, element.attr(local_name!("step")));
        editor.update_limits(
            element.attr(local_name!("min")),
            element.attr(local_name!("max")),
        );
        editor.update_step_base(
            element.attr(local_name!("min")),
            element.attr(local_name!("value")),
        );
        element.form_state.calendar = Some(editor);
        true
    }
    pub(crate) fn calendar_key<F: FnMut(crate::events::GeneratedEvent)>(
        &mut self,
        id: NodeId,
        event: &BlitzKeyEvent,
        emit: &mut F,
    ) -> bool {
        if !self.ensure_calendar_editor(id) {
            return false;
        }
        if !event.state.is_pressed()
            || event.is_composing
            || event
                .modifiers
                .intersects(Modifiers::CONTROL | Modifiers::META | Modifiers::ALT)
        {
            return true;
        }
        if event.key == Key::Enter {
            return false;
        }
        if self.nodes[id].is_disabled() || self.nodes[id].attr(local_name!("readonly")).is_some() {
            return event.key != Key::Tab;
        }
        let Some(element) = self.nodes[id].element_data_mut() else {
            return true;
        };
        let Some(editor) = element.form_state.calendar.as_mut() else {
            return true;
        };
        let before = editor.values;
        let navigation = matches!(event.key, Key::Tab | Key::ArrowLeft | Key::ArrowRight);
        let handled = if event.key == Key::Tab {
            editor.move_field(event.modifiers.contains(Modifiers::SHIFT))
        } else {
            editor.key(&event.key, self.calendar_reference_year)
        };
        let edited = (handled && !navigation) || before != editor.values;
        if let Some(value) = self.apply_calendar_value(id, edited) {
            emit_change(id, value, emit);
        }
        handled || event.key != Key::Tab
    }
    fn apply_calendar_value(&mut self, id: NodeId, edited: bool) -> Option<String> {
        let old = self.form_value(id).unwrap_or_default();
        let element = self.nodes[id].element_data_mut()?;
        let value = element.form_state.calendar.as_ref()?.value();
        if edited {
            element.form_state.value = Some(value.clone().into());
            if value != old {
                element.form_state.value_dirty = true;
                element.form_state.last_change_by_user = true;
            }
        }
        self.project_calendar_editor(id);
        self.shell_provider.request_redraw();
        (edited && value != old).then_some(value)
    }
    pub(crate) fn commit_calendar_field(&mut self, id: NodeId) -> Option<String> {
        let element = self.nodes[id].element_data_mut()?;
        let editor = element.form_state.calendar.as_mut()?;
        let before = editor.values;
        editor.commit_field();
        let edited = before != editor.values;
        self.apply_calendar_value(id, edited)
    }
    pub(crate) fn select_calendar_field<F: FnMut(crate::events::GeneratedEvent)>(
        &mut self,
        id: NodeId,
        emit: &mut F,
    ) {
        let Some(element) = self.nodes[id].element_data_mut() else {
            return;
        };
        let Some(byte) = element
            .text_input_data()
            .map(|i| i.editor.raw_selection().focus().index())
        else {
            return;
        };
        let Some(editor) = element.form_state.calendar.as_mut() else {
            return;
        };
        let before = editor.values;
        editor.select_at(byte);
        let edited = before != editor.values;
        if let Some(value) = self.apply_calendar_value(id, edited) {
            emit_change(id, value, emit);
        }
    }
    pub(crate) fn project_calendar_editor(&mut self, id: NodeId) {
        let Some(element) = self.nodes[id].element_data_mut() else {
            return;
        };
        let Some(editor) = element.form_state.calendar.as_ref() else {
            return;
        };
        let (text, selected) = editor.presentation();
        let Some(input) = element.text_input_data_mut() else {
            return;
        };
        input.set_text(
            &mut self.font_ctx.lock().unwrap(),
            &mut self.layout_ctx,
            &text,
        );
        input
            .editor
            .driver(&mut self.font_ctx.lock().unwrap(), &mut self.layout_ctx)
            .select_byte_range(selected.start, selected.end);
    }
}
fn emit_change(id: NodeId, value: String, emit: &mut impl FnMut(crate::events::GeneratedEvent)) {
    emit(
        DomEvent::new(
            id,
            DomEventData::Input(BlitzInputEvent {
                value: value.clone(),
            }),
        )
        .into(),
    );
    emit(DomEvent::new(id, DomEventData::Change(BlitzInputEvent { value })).into());
}
