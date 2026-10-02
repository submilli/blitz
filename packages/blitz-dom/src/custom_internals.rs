//! Engine-owned form association, constraint state and submission values.
use crate::custom_elements::CustomElementReaction;
use crate::{BaseDocument, DomString, NodeId, ValidityState};
use blitz_traits::net::{Entry, EntryValue};

#[derive(Clone, Debug, Default)]
pub enum CustomFormValue {
    #[default]
    Null,
    Single(EntryValue),
    Entries(Vec<Entry>),
}
#[derive(Clone, Debug, Default)]
pub struct CustomInternals {
    pub value: CustomFormValue,
    pub state: CustomFormValue,
    pub anchor: Option<NodeId>,
    pub validity: ValidityState,
    pub message: DomString,
    owner: Option<NodeId>,
    disabled: bool,
}
/// A validation message exceeded the native retained-payload budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ValidationMessageLimit;

impl BaseDocument {
    pub fn is_form_associated_custom(&self, id: NodeId) -> bool {
        self.get_node(id)
            .and_then(|n| n.element_data())
            .is_some_and(|e| e.custom_internals.is_some())
    }
    /// Ownership is established after successful construction, not merely by topology.
    pub fn custom_element_form_owner(&self, id: NodeId) -> Option<NodeId> {
        self.get_node(id)
            .and_then(|node| node.element_data())
            .and_then(|element| element.custom_internals.as_ref())
            .and_then(|internals| internals.owner)
    }
    /// Register before invoking the constructor; its internals can query ownership.
    pub fn register_form_associated(&mut self, id: NodeId) {
        self.form_associated_custom.insert(id);
        if let Some(element) = self.get_node_mut(id).and_then(|n| n.element_data_mut()) {
            element
                .custom_internals
                .get_or_insert_with(Default::default);
        }
    }
    pub fn set_internals_form_value(
        &mut self,
        id: NodeId,
        value: CustomFormValue,
        state: CustomFormValue,
    ) {
        if let Some(internals) = self
            .get_node_mut(id)
            .and_then(|n| n.element_data_mut())
            .and_then(|e| e.custom_internals.as_mut())
        {
            internals.value = value;
            internals.state = state;
        }
    }
    pub fn set_internals_validity(
        &mut self,
        id: NodeId,
        validity: ValidityState,
        message: DomString,
        anchor: Option<NodeId>,
    ) -> Result<(), ValidationMessageLimit> {
        if message.retained_bytes() > 256 * 1024 {
            return Err(ValidationMessageLimit);
        }
        if let Some(internals) = self
            .get_node_mut(id)
            .and_then(|n| n.element_data_mut())
            .and_then(|e| e.custom_internals.as_mut())
        {
            internals.validity = validity;
            internals.message = message;
            internals.anchor = anchor;
        }
        Ok(())
    }
    /// Runs after topology and attribute mutation, never invokes page script.
    pub(crate) fn refresh_custom_form_associations(&mut self) {
        self.form_associated_custom
            .retain(|&id| self.nodes.get(id).is_some());
        if self.form_associated_custom.is_empty() {
            return;
        }
        let mut roots = std::collections::BTreeSet::new();
        for &id in &self.form_associated_custom {
            roots.insert(self.tree_root(id));
        }
        let mut changes = Vec::new();
        for root in roots {
            let owners = self.form_owners_in_tree(root);
            for id in crate::traversal::TreeTraverser::new_with_root(self, root) {
                if self.form_associated_custom.contains(&id)
                    && self.custom_element_state(id)
                        == crate::custom_elements::CustomElementState::Custom
                {
                    changes.push((
                        id,
                        owners.get(&id).copied().flatten(),
                        self.nodes[id].is_disabled(),
                    ));
                }
            }
        }
        for (id, owner, disabled) in changes {
            self.update_custom_form_association(id, owner, disabled);
        }
    }
    pub fn refresh_custom_form_association(&mut self, id: NodeId) {
        if self.custom_element_state(id) != crate::custom_elements::CustomElementState::Custom {
            return;
        }
        let owner = self.form_owner(id);
        let disabled = self.get_node(id).is_some_and(|n| n.is_disabled());
        self.update_custom_form_association(id, owner, disabled);
    }
    fn update_custom_form_association(
        &mut self,
        id: NodeId,
        owner: Option<NodeId>,
        disabled: bool,
    ) {
        let Some(internals) = self
            .get_node_mut(id)
            .and_then(|n| n.element_data_mut())
            .and_then(|e| e.custom_internals.as_mut())
        else {
            return;
        };
        let owner_changed = internals.owner != owner;
        let disabled_changed = internals.disabled != disabled;
        internals.owner = owner;
        internals.disabled = disabled;
        if owner_changed {
            self.record_custom_element_reaction(CustomElementReaction::FormAssociated {
                element: id,
                form: owner,
            });
        }
        if disabled_changed {
            self.record_custom_element_reaction(CustomElementReaction::FormDisabled {
                element: id,
                disabled,
            });
        }
    }
    /// Label control resolution is scoped to the element's DOM tree.
    pub fn custom_element_labels(&self, id: NodeId) -> Vec<NodeId> {
        if !self.is_labelable_control(id) {
            return Vec::new();
        }
        use crate::traversal::TreeTraverser;
        let root = self.tree_root(id);
        let nodes: Vec<_> = TreeTraverser::new_with_root(self, root).collect();
        let explicit_id = self.null_attribute(id, "id").filter(|attribute| {
            nodes.iter().find(|&&candidate| {
                self.null_attribute(candidate, "id")
                    .is_some_and(|a| a.value == attribute.value)
            }) == Some(&id)
        });
        // Reverse tree order lets each earlier child replace later candidates.
        let mut first_controls = std::collections::HashMap::new();
        for &candidate in nodes.iter().rev() {
            let first = if self.is_labelable_control(candidate) {
                Some(candidate)
            } else {
                first_controls.get(&candidate).copied()
            };
            if let Some(first) = first
                && let Some(parent) = self.nodes[candidate].parent
            {
                first_controls.insert(parent, first);
            }
        }
        nodes
            .into_iter()
            .filter(|&label| {
                let Some(element) = self.get_node(label).and_then(|n| n.element_data()) else {
                    return false;
                };
                if element.name.ns != crate::ns!(html) || &*element.name.local != "label" {
                    return false;
                }
                if let Some(target) = self.null_attribute(label, "for") {
                    return explicit_id.is_some_and(|attribute| attribute.value == target.value);
                }
                first_controls.get(&label) == Some(&id)
            })
            .collect()
    }
    pub fn is_labelable_control(&self, id: NodeId) -> bool {
        self.get_node(id)
            .and_then(|n| n.element_data())
            .is_some_and(|e| {
                e.name.ns == crate::ns!(html)
                    && (e.is_custom_form_control()
                        || match &*e.name.local {
                            "button" | "meter" | "output" | "progress" | "select" | "textarea" => {
                                true
                            }
                            "input" => !e
                                .attr(markup5ever::local_name!("type"))
                                .is_some_and(|v| v.eq_ignore_ascii_case("hidden")),
                            _ => false,
                        })
            })
    }
}

impl BaseDocument {
    pub fn set_custom_state(&mut self, id: NodeId, name: &str, present: bool) {
        self.snapshot_node(id);
        if let Some(element) = self.get_node_mut(id).and_then(|n| n.element_data_mut()) {
            let name = style::values::AtomIdent::from(name);
            if present {
                if !element.custom_states.contains(&name) {
                    element.custom_states.push(name);
                }
            } else {
                element.custom_states.retain(|v| v != &name);
            }
        }
        self.invalidate_custom_state_selectors();
    }
    pub fn clear_custom_states(&mut self, id: NodeId) {
        self.snapshot_node(id);
        if let Some(element) = self.get_node_mut(id).and_then(|n| n.element_data_mut()) {
            element.custom_states.clear();
        }
        self.invalidate_custom_state_selectors();
    }
    fn invalidate_custom_state_selectors(&mut self) {
        let roots: Vec<_> = self
            .root_node()
            .children
            .iter()
            .copied()
            .filter(|&id| {
                self.get_node(id)
                    .is_some_and(|n| n.element_data().is_some())
            })
            .collect();
        for id in roots {
            self.restyle_subtree_of(id);
        }
    }
}

impl BaseDocument {
    pub fn internals_validation_anchor(&self, id: NodeId) -> NodeId {
        self.get_node(id)
            .and_then(|n| n.element_data())
            .and_then(|e| e.custom_internals.as_ref())
            .and_then(|i| i.anchor)
            .filter(|&id| self.get_node(id).is_some())
            .unwrap_or(id)
    }
}
