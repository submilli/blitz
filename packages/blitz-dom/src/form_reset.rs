//! Reset current form state without mutating content attributes.
use crate::{DocumentMutator, NodeId, ns};
use std::collections::HashMap;

impl DocumentMutator<'_> {
    /// Called after a cancelable reset event, with current tree ownership.
    // DomString only caches its scalar projection; shared access cannot change
    // the authoritative UTF-16 units used by both Hash and Eq.
    #[allow(clippy::mutable_key_type)]
    pub fn reset_form_controls(&mut self, form: NodeId) -> Result<(), crate::NodeBudgetExceeded> {
        let controls = self.doc.form_controls(form);
        // One winner per named radio group, all in this tree and form. Avoid
        // recomputing ownership or dirtying peers for each individual reset.
        let mut radios = HashMap::new();
        for &id in &controls {
            if self.doc.null_attribute(id, "checked").is_some() {
                if let Some(name) = self.doc.radio_name(id) {
                    radios.insert(name.to_owned(), id);
                }
            }
        }
        for id in controls {
            let tag = self.doc.nodes[id]
                .element_data()
                .filter(|el| el.name.ns == ns!(html))
                .map(|el| el.name.local.clone());
            match tag.as_deref() {
                Some("select") => self.reset_select(id),
                Some("output") => self.reset_output(id)?,
                Some("input" | "textarea") => self.reset_text_or_input(id, &radios),
                _ => {}
            }
        }
        Ok(())
    }

    /// HTML textarea children-changed steps run after topology has settled.
    pub(crate) fn textarea_children_changed(&mut self, parent: NodeId) {
        let node = &self.doc.nodes[parent];
        if !node.element_data().is_some_and(|e| {
            e.name.ns == ns!(html) && &*e.name.local == "textarea" && !e.form_state.value_dirty
        }) {
            return;
        }
        let value = node.child_text_content();
        self.set_form_value_dom(parent, &value);
        if let Some(e) = self.doc.nodes[parent].element_data_mut() {
            e.form_state.value_dirty = false;
        }
    }

    // DomString only caches its scalar projection; shared access cannot change
    // the authoritative UTF-16 units used by both Hash and Eq.
    #[allow(clippy::mutable_key_type)]
    fn reset_text_or_input(&mut self, id: NodeId, radios: &HashMap<crate::DomString, NodeId>) {
        if self.doc.nodes[id]
            .element_data()
            .is_some_and(crate::dom_api::is_text_control)
        {
            let textarea = self.doc.nodes[id]
                .element_data()
                .is_some_and(|el| &*el.name.local == "textarea");
            let value = if textarea {
                self.doc.nodes[id].child_text_content()
            } else {
                self.doc
                    .null_attribute(id, "value")
                    .map(|a| a.value.clone())
                    .unwrap_or_default()
            };
            self.set_form_value_dom(id, &value);
        }
        if self.doc.nodes[id]
            .element_data()
            .is_some_and(crate::dom_api::is_checkable)
        {
            let checked = self.doc.radio_name(id).map_or_else(
                || self.doc.null_attribute(id, "checked").is_some(),
                |name| radios.get(name) == Some(&id),
            );
            self.write_checkedness(id, checked);
        }
        if let Some(el) = self.doc.nodes[id].element_data_mut() {
            el.form_state.value_dirty = false;
            el.form_state.checked_dirty = false;
        }
    }
}
