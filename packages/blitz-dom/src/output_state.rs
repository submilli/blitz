//! The output element's default/live text mode, independent of JS wrappers.
//! https://html.spec.whatwg.org/multipage/form-elements.html#the-output-element
use crate::{BaseDocument, DocumentMutator, DomString, NodeBudgetExceeded, NodeId, ns};

impl BaseDocument {
    fn is_output(&self, id: NodeId) -> bool {
        self.get_node(id)
            .and_then(|n| n.element_data())
            .is_some_and(|el| el.name.ns == ns!(html) && &*el.name.local == "output")
    }

    /// Both default and live output values are text, preserving UTF-16 code units.
    pub fn output_value(&self, id: NodeId) -> Option<DomString> {
        self.is_output(id)
            .then(|| self.text_content_dom(id).unwrap_or_default())
    }

    /// Default mode follows text edits; live mode keeps its saved reset value.
    pub fn output_default_value(&self, id: NodeId) -> Option<DomString> {
        if !self.is_output(id) {
            return None;
        }
        self.nodes[id]
            .element_data()?
            .form_state
            .output_default
            .clone()
            .or_else(|| self.output_value(id))
    }
}

impl DocumentMutator<'_> {
    /// Enter live mode only after the replacement text passes node admission.
    pub fn set_output_value(
        &mut self,
        id: NodeId,
        value: DomString,
    ) -> Result<(), NodeBudgetExceeded> {
        let Some(default) = self.doc.output_default_value(id) else {
            return Ok(());
        };
        self.try_set_text_content(id, value)?;
        if let Some(el) = self.doc.nodes[id].element_data_mut() {
            el.form_state.output_default = Some(default);
        }
        Ok(())
    }

    /// Setting the default changes text only while the output is in default mode.
    pub fn set_output_default_value(
        &mut self,
        id: NodeId,
        value: DomString,
    ) -> Result<(), NodeBudgetExceeded> {
        if !self.doc.is_output(id) {
            return Ok(());
        }
        let live = self.doc.nodes[id]
            .element_data()
            .is_some_and(|el| el.form_state.output_default.is_some());
        if live {
            if let Some(el) = self.doc.nodes[id].element_data_mut() {
                el.form_state.output_default = Some(value);
            }
        } else {
            self.try_set_text_content(id, value)?;
        }
        Ok(())
    }

    pub(crate) fn reset_output(&mut self, id: NodeId) -> Result<(), NodeBudgetExceeded> {
        let default = self
            .doc
            .get_node(id)
            .and_then(|n| n.element_data())
            .and_then(|el| el.form_state.output_default.clone());
        if let Some(default) = default {
            self.try_set_text_content(id, default)?;
            if let Some(el) = self.doc.nodes[id].element_data_mut() {
                el.form_state.output_default = None;
            }
        }
        Ok(())
    }
}
