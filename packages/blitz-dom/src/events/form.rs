//! Form default actions cross the event driver only after its DOM borrow ends.
use crate::{BaseDocument, NodeId};

/// A form activation selected by native input. Embedders may dispatch script
/// during this action; the event driver retains its nodes and holds no DOM borrow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormAction {
    Submit {
        form: NodeId,
        submitter: Option<NodeId>,
    },
    Reset {
        form: NodeId,
    },
}

impl FormAction {
    pub(crate) fn nodes(self) -> impl Iterator<Item = NodeId> {
        match self {
            Self::Submit {
                form, submitter, ..
            } => [Some(form), submitter],
            Self::Reset { form } => [Some(form), None],
        }
        .into_iter()
        .flatten()
    }
}

impl BaseDocument {
    /// Resolve current form activation after click listeners. A pointer position
    /// is in viewport CSS pixels; keyboard/programmatic activation selects (0,0).
    pub fn activate_form_control(
        &mut self,
        id: NodeId,
        pointer: Option<(f64, f64)>,
    ) -> Option<FormAction> {
        let node = self.get_node(id)?;
        let e = node.element_data()?;
        if e.name.ns != crate::ns!(html) || node.is_disabled() {
            return None;
        }
        let kind = e.attr(markup5ever::local_name!("type")).unwrap_or("");
        if e.is_submit_button() {
            if &*e.name.local == "input" && kind.eq_ignore_ascii_case("image") {
                let coordinates = pointer
                    .and_then(|(x, y)| {
                        let rect = self.get_client_bounding_rect(id)?;
                        let border = self.nodes[id].final_layout().border;
                        Some((
                            (x - rect.x - border.left as f64).round().max(0.0) as u32,
                            (y - rect.y - border.top as f64).round().max(0.0) as u32,
                        ))
                    })
                    .unwrap_or_default();
                self.nodes[id]
                    .element_data_mut()
                    .expect("image input is an element")
                    .form_state
                    .image_coordinates = coordinates;
            }
            return self.form_owner(id).map(|form| FormAction::Submit {
                form,
                submitter: Some(id),
            });
        }
        if matches!(&*e.name.local, "input" | "button") && kind.eq_ignore_ascii_case("reset") {
            return self.form_owner(id).map(|form| FormAction::Reset { form });
        }
        None
    }

    /// The default button is the first submit button in tree order, including
    /// external associated controls. A disabled default button prevents submission.
    /// https://html.spec.whatwg.org/multipage/form-control-infrastructure.html#implicit-submission
    pub fn implicit_submission(&self, input: NodeId) -> Option<ImplicitSubmission> {
        let form = self.form_owner(input)?;
        let controls = self.form_controls(form);
        if let Some(button) = controls.iter().copied().find(|&id| {
            self.nodes[id]
                .element_data()
                .is_some_and(|e| e.is_submit_button())
        }) {
            return (!self.nodes[button].is_disabled())
                .then_some(ImplicitSubmission::Click(button));
        }
        let blockers = controls
            .into_iter()
            .filter(|&id| {
                let Some(e) = self.nodes[id].element_data() else {
                    return false;
                };
                if &*e.name.local != "input" {
                    return false;
                }
                let kind = e
                    .attr(markup5ever::local_name!("type"))
                    .unwrap_or("text")
                    .to_ascii_lowercase();
                // Unknown input types use the text state.
                !matches!(
                    kind.as_str(),
                    "hidden"
                        | "checkbox"
                        | "radio"
                        | "file"
                        | "submit"
                        | "image"
                        | "reset"
                        | "button"
                        | "range"
                        | "color"
                )
            })
            .take(2)
            .count();
        (blockers <= 1).then_some(ImplicitSubmission::Submit(form))
    }
}

/// The implicit-submission algorithm activates the default button, or submits
/// directly when no button exists and at most one text-like field blocks it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImplicitSubmission {
    Click(NodeId),
    Submit(NodeId),
}
