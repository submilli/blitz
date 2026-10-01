//! Exceptional nonancestor owners established by the HTML parser.
use std::collections::{HashMap, HashSet};

use crate::{DocumentMutator, NodeId, ns};

/// Reverse links make removal proportional to affected associations, rather
/// than searching all document controls for every removed form.
#[derive(Default)]
pub(crate) struct ParserForms {
    owners: HashMap<NodeId, NodeId>,
    controls: HashMap<NodeId, HashSet<NodeId>>,
}

impl ParserForms {
    pub(crate) fn owner(&self, control: NodeId) -> Option<NodeId> {
        self.owners.get(&control).copied()
    }

    fn associate(&mut self, control: NodeId, form: NodeId) {
        self.clear_control(control);
        self.owners.insert(control, form);
        self.controls.entry(form).or_default().insert(control);
    }

    pub(crate) fn clear_control(&mut self, control: NodeId) {
        if let Some(form) = self.owners.remove(&control)
            && let Some(controls) = self.controls.get_mut(&form)
        {
            controls.remove(&control);
            if controls.is_empty() {
                self.controls.remove(&form);
            }
        }
    }

    pub(crate) fn remove(&mut self, node: NodeId) {
        self.clear_control(node);
        if let Some(controls) = self.controls.remove(&node) {
            for control in controls {
                self.owners.remove(&control);
            }
        }
    }
}

impl DocumentMutator<'_> {
    /// Establish the parser's owner after its initial insertion. The parser
    /// calls this before returning control to script. Later ancestor changes
    /// and form-attribute writes discard the exceptional association.
    pub fn associate_parser_form(&mut self, control: NodeId, form: NodeId) {
        let eligible = self
            .doc
            .get_node(control)
            .and_then(|n| n.element_data())
            .is_some_and(|e| {
                e.name.ns == ns!(html)
                    && matches!(
                        &*e.name.local,
                        "button"
                            | "fieldset"
                            | "input"
                            | "object"
                            | "output"
                            | "select"
                            | "textarea"
                            | "img"
                    )
                    && (&*e.name.local == "img"
                        || self.doc.null_attribute(control, "form").is_none())
            });
        let is_form = self
            .doc
            .get_node(form)
            .and_then(|n| n.element_data())
            .is_some_and(|e| e.name.ns == ns!(html) && &*e.name.local == "form");
        if eligible && is_form && self.doc.tree_root(control) == self.doc.tree_root(form) {
            self.doc.parser_forms.associate(control, form);
        }
    }

    pub(crate) fn reset_parser_form_attribute(&mut self, control: NodeId) {
        let image = self
            .doc
            .get_node(control)
            .and_then(|n| n.element_data())
            .is_some_and(|e| e.name.ns == ns!(html) && &*e.name.local == "img");
        if !image {
            self.doc.parser_forms.clear_control(control);
        }
    }

    pub(crate) fn reset_parser_forms_in_subtree(&mut self, root: NodeId) {
        if self.doc.parser_forms.owners.is_empty() {
            return;
        }
        // Removing a shared ancestor preserves parser associations inside the
        // removed tree. Only links crossing its boundary lose their owner.
        let mut moved = HashSet::new();
        self.doc.iter_shadow_including_subtree_mut(root, |id, _| {
            moved.insert(id);
        });
        let associations = &mut self.doc.parser_forms;
        let mut reset = Vec::new();
        for &id in &moved {
            if associations
                .owner(id)
                .is_some_and(|owner| !moved.contains(&owner))
            {
                reset.push(id);
            }
            if let Some(controls) = associations.controls.get(&id) {
                reset.extend(
                    controls
                        .iter()
                        .copied()
                        .filter(|control| !moved.contains(control)),
                );
            }
        }
        for control in reset {
            associations.clear_control(control);
        }
    }
}
