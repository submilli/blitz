//! HTML text-field selection in authoritative UTF-16 coordinates.
//! <https://html.spec.whatwg.org/multipage/form-control-infrastructure.html#textFieldSelection>

use crate::{BaseDocument, DocumentMutator, ElementData, NodeId};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SelectionDirection {
    #[default]
    None,
    Forward,
    Backward,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ControlSelection {
    pub start: usize,
    pub end: usize,
    pub direction: SelectionDirection,
}

impl ControlSelection {
    pub(crate) fn caret(offset: usize) -> Self {
        Self {
            start: offset,
            end: offset,
            direction: SelectionDirection::None,
        }
    }

    /// Parley uses scalar UTF-8 positions. Keep surrogate-interior positions in
    /// this DOM state; their paint projection uses the following scalar edge.
    pub(crate) fn project(self, text: &str) -> (usize, usize) {
        let byte = |offset| {
            let mut units = 0;
            for (index, ch) in text.char_indices() {
                if units >= offset {
                    return index;
                }
                units += ch.len_utf16();
            }
            text.len()
        };
        let start = byte(self.start);
        let end = byte(self.end);
        if self.direction == SelectionDirection::Backward {
            (end, start)
        } else {
            (start, end)
        }
    }
}

fn supports_selection(element: &ElementData) -> bool {
    if element.name.ns != crate::ns!(html) {
        return false;
    }
    if element.name.local.as_ref() == "textarea" {
        return true;
    }
    if element.name.local.as_ref() != "input" {
        return false;
    }
    let kind = element
        .attr(markup5ever::local_name!("type"))
        .unwrap_or("")
        .to_ascii_lowercase();
    !matches!(
        kind.as_str(),
        "email"
            | "hidden"
            | "date"
            | "month"
            | "week"
            | "time"
            | "datetime-local"
            | "number"
            | "range"
            | "color"
            | "checkbox"
            | "radio"
            | "file"
            | "submit"
            | "image"
            | "reset"
            | "button"
    )
}

impl BaseDocument {
    pub(crate) fn notify_control_selection(&self, id: NodeId, focused: bool) {
        if let Some(observer) = &self.tree_observer {
            observer.control_selection(self, id, focused);
        }
    }

    /// Text-field stringification uses the editor's scalar projection while
    /// retaining exact UTF-16 selection coordinates for script getters.
    pub fn control_selection_text(&self, id: NodeId) -> String {
        let Some(selection) = self.control_selection(id) else {
            return String::new();
        };
        let value = self.form_value_dom(id).unwrap_or_default();
        let text = value.as_str_lossy();
        let (a, b) = selection.project(text);
        text[a.min(b)..a.max(b)].to_owned()
    }
    /// Unsupported input types expose no text-field selection.
    pub fn control_selection(&self, id: NodeId) -> Option<ControlSelection> {
        let element = self.get_node(id)?.element_data()?;
        supports_selection(element).then_some(element.form_state.selection)
    }

    /// Project the DOM selection after editor creation or a script change.
    pub(crate) fn project_control_selection(&mut self, id: NodeId) {
        let Some(element) = self.nodes[id].element_data_mut() else {
            return;
        };
        let selection = element.form_state.selection;
        let Some(input) = element.text_input_data_mut() else {
            return;
        };
        let (anchor, focus) = selection.project(input.editor.raw_text());
        input
            .editor
            .driver(&mut self.font_ctx.lock().unwrap(), &mut self.layout_ctx)
            .select_byte_range(anchor, focus);
    }

    /// Native editing updates the DOM coordinates only after an editor action,
    /// never merely because layout rounded a surrogate-interior projection.
    pub(crate) fn capture_control_selection(&mut self, id: NodeId) {
        let Some(element) = self.nodes[id].element_data_mut() else {
            return;
        };
        let previous = element.form_state.selection;
        let Some(input) = element.text_input_data() else {
            return;
        };
        let selection = input.editor.raw_selection();
        let text = input.editor.raw_text();
        let units = |index| text.get(..index).map_or(0, |s| s.encode_utf16().count());
        let anchor = units(selection.anchor().index());
        let focus = units(selection.focus().index());
        element.form_state.selection = ControlSelection {
            start: anchor.min(focus),
            end: anchor.max(focus),
            direction: if anchor > focus {
                SelectionDirection::Backward
            } else if anchor < focus {
                SelectionDirection::Forward
            } else {
                SelectionDirection::None
            },
        };
        if self.control_selection(id) != Some(previous) {
            self.notify_control_selection(id, self.focus_node_id == Some(id));
        }
    }
}

impl DocumentMutator<'_> {
    /// Clamp and normalize before changing state. A false result means that
    /// this input type does not support the text selection API.
    pub fn set_control_selection(&mut self, id: NodeId, mut selection: ControlSelection) -> bool {
        let Some(previous) = self.doc.control_selection(id) else {
            return false;
        };
        let length = self
            .doc
            .form_value_dom(id)
            .map_or(0, |v| v.to_utf16().len());
        selection.end = selection.end.min(length);
        selection.start = selection.start.min(selection.end);
        if previous == selection {
            return true;
        }
        self.doc.nodes[id]
            .element_data_mut()
            .expect("selection belongs to a control")
            .form_state
            .selection = selection;
        self.doc.project_control_selection(id);
        self.doc
            .notify_control_selection(id, self.doc.focus_node_id == Some(id));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_survives_equal_values_and_clamps_changed_values() {
        let mut doc = BaseDocument::new(crate::DocumentConfig::default());
        let input = doc.mutate().create_element(
            crate::QualName::new(None, crate::ns!(html), "input".into()),
            vec![],
        );
        assert_eq!(
            doc.control_selection(input),
            Some(ControlSelection::default())
        );
        doc.mutate().set_form_value(input, "a😀z");
        let selected = ControlSelection {
            start: 2,
            end: 3,
            direction: SelectionDirection::Backward,
        };
        assert!(doc.mutate().set_control_selection(input, selected));
        doc.mutate().set_form_value(input, "a😀z");
        assert_eq!(doc.control_selection(input), Some(selected));
        assert_eq!(selected.project("a😀z"), (5, 5));
        doc.mutate().set_form_value(input, "x");
        assert_eq!(
            doc.control_selection(input),
            Some(ControlSelection::caret(1))
        );
        assert!(doc.mutate().set_control_selection(
            input,
            ControlSelection {
                start: usize::MAX,
                end: 0,
                direction: SelectionDirection::Forward,
            }
        ));
        assert_eq!(doc.control_selection(input).unwrap().start, 0);
    }

    #[test]
    fn unsupported_controls_do_not_accept_selection_state() {
        let mut doc = BaseDocument::new(crate::DocumentConfig::default());
        let input = doc.mutate().create_element(
            crate::QualName::new(None, crate::ns!(html), "input".into()),
            vec![],
        );
        for kind in ["email", "number", "checkbox", "date", "hidden", "file"] {
            doc.mutate().set_attribute(
                input,
                crate::QualName::new(None, crate::ns!(), "type".into()),
                kind,
            );
            assert_eq!(doc.control_selection(input), None, "{kind}");
            assert!(
                !doc.mutate()
                    .set_control_selection(input, ControlSelection::caret(0))
            );
        }
    }
}
