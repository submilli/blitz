//! Reset current form state without mutating content attributes.
use crate::{DocumentMutator, NodeId, ns};
use std::collections::HashMap;

impl DocumentMutator<'_> {
    /// Called after a cancelable reset event, with current tree ownership.
    pub fn reset_form_controls(&mut self, form: NodeId) {
        let root = self.doc.tree_root(form);
        let owners = self.doc.form_owners_in_tree(root);
        let controls: Vec<_> = crate::traversal::TreeTraverser::new_with_root(self.doc, root)
            .filter(|id| owners.get(id) == Some(&Some(form)))
            .collect();
        // One winner per named radio group, all in this tree and form. Avoid
        // recomputing ownership or dirtying peers for each individual reset.
        let mut radios = HashMap::new();
        for &id in &controls {
            if self.doc.attribute_by_name(id, "checked").is_some() {
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
                Some("input" | "textarea") => self.reset_text_or_input(id, &radios),
                _ => {}
            }
        }
    }

    fn reset_text_or_input(&mut self, id: NodeId, radios: &HashMap<String, NodeId>) {
        if self.doc.nodes[id]
            .element_data()
            .is_some_and(crate::dom_api::is_text_control)
        {
            let textarea = self.doc.nodes[id]
                .element_data()
                .is_some_and(|el| &*el.name.local == "textarea");
            let value = if textarea {
                self.doc.text_content_of(id).unwrap_or_default()
            } else {
                self.doc
                    .attribute_by_name(id, "value")
                    .map(|a| a.value.clone())
                    .unwrap_or_default()
            };
            self.set_form_value(id, &value);
        }
        if self.doc.nodes[id]
            .element_data()
            .is_some_and(crate::dom_api::is_checkable)
        {
            let checked = self.doc.radio_name(id).map_or_else(
                || self.doc.attribute_by_name(id, "checked").is_some(),
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
