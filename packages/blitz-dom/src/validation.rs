//! Native candidate selection and live constraint state.
mod calendar;
mod number;
mod pattern;
pub(crate) mod text;

use crate::{BaseDocument, DocumentMutator, DomString, NodeId, ns};
use markup5ever::local_name;

/// A validation operation exceeded its shared native resource allowance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ValidationError;
impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("constraint validation resource budget exceeded")
    }
}
impl std::error::Error for ValidationError {}

/// Current constraint flags, independent of whether validation is barred.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ValidityState {
    pub value_missing: bool,
    pub type_mismatch: bool,
    pub pattern_mismatch: bool,
    pub too_long: bool,
    pub too_short: bool,
    pub range_underflow: bool,
    pub range_overflow: bool,
    pub step_mismatch: bool,
    pub bad_input: bool,
    pub custom_error: bool,
}

impl ValidityState {
    /// Whether none of the constraints fail.
    pub fn valid(self) -> bool {
        self == Self::default()
    }
}

impl BaseDocument {
    /// Drain a native default-action resource failure. Script embeddings return
    /// their validation errors directly or through their existing event diagnostics.
    pub fn take_validation_error(&self) -> Option<ValidationError> {
        self.validation_error.take()
    }

    /// Whether a control participates in constraint validation.
    pub fn will_validate(&self, id: NodeId) -> bool {
        let Some(node) = self.get_node(id) else {
            return false;
        };
        let Some(e) = node.element_data().filter(|e| e.name.ns == ns!(html)) else {
            return false;
        };
        if node.is_disabled()
            || crate::traversal::AncestorTraverser::new(self, id).any(|p| {
                self.nodes[p]
                    .element_data()
                    .is_some_and(|e| e.name.ns == ns!(html) && &*e.name.local == "datalist")
            })
        {
            return false;
        }
        Self::validation_type_candidate(e)
    }

    fn validation_type_candidate(e: &crate::ElementData) -> bool {
        let kind = e
            .attr_dom(local_name!("type"))
            .map(crate::DomString::as_str_lossy)
            .unwrap_or("")
            .to_ascii_lowercase();
        match &*e.name.local {
            "input" => {
                !matches!(kind.as_str(), "hidden" | "reset" | "button" | "image")
                    && !(e.has_attr(local_name!("readonly"))
                        && !matches!(
                            kind.as_str(),
                            "checkbox" | "radio" | "file" | "range" | "color" | "submit"
                        ))
            }
            "textarea" => !e.has_attr(local_name!("readonly")),
            "select" => true,
            "button" => !matches!(kind.as_str(), "button" | "reset"),
            _ => false,
        }
    }

    /// Snapshot live flags without dispatching events or checking candidacy.
    pub fn control_validity(&self, id: NodeId) -> Result<ValidityState, ValidationError> {
        self.control_validity_with_radio(id, None, false, &regress_bounded::Budget::default())
    }

    fn control_validity_with_radio(
        &self,
        id: NodeId,
        radio: Option<bool>,
        known_candidate: bool,
        budget: &regress_bounded::Budget,
    ) -> Result<ValidityState, ValidationError> {
        let Some(e) = self.get_node(id).and_then(|n| n.element_data()) else {
            return Ok(ValidityState::default());
        };
        if e.name.ns != ns!(html) {
            return Ok(ValidityState::default());
        }
        pattern::admit_input(e, budget)?;
        let mut state = ValidityState {
            value_missing: self.required_value_missing(id, radio, known_candidate),
            custom_error: !e.form_state.custom_validity.is_empty(),
            ..ValidityState::default()
        };
        if matches!(&*e.name.local, "input" | "textarea") {
            let value = self.form_value_dom(id).unwrap_or_default();
            text::text_constraints(e, value.as_str_lossy(), &mut state);
            state.pattern_mismatch = pattern::mismatch(e, &value, budget)?;
            if &*e.name.local == "input" {
                let kind = e
                    .attr_dom(local_name!("type"))
                    .map(crate::DomString::as_str_lossy)
                    .unwrap_or("")
                    .to_ascii_lowercase();
                let raw = e
                    .text_input_data()
                    .map(|i| i.editor.raw_text())
                    .or_else(|| e.form_state.value.as_ref().map(DomString::as_str_lossy))
                    .unwrap_or("");
                number::constraints(e, &kind, value.as_str_lossy(), raw, &mut state);
            }
        }
        Ok(state)
    }

    /// Current constraint failure, without dispatching invalid events.
    pub fn control_validation_message(
        &self,
        id: NodeId,
    ) -> Result<Option<DomString>, ValidationError> {
        if !self.will_validate(id) {
            return Ok(None);
        }
        let Some(e) = self.get_node(id).and_then(|node| node.element_data()) else {
            return Ok(None);
        };
        if !e.form_state.custom_validity.is_empty() {
            return Ok(Some(e.form_state.custom_validity.clone()));
        }
        let state = self.control_validity(id)?;
        Ok(if state.valid() {
            None
        } else if state.value_missing {
            Some("Please fill out this field.".into())
        } else {
            Some("Please enter a valid value.".into())
        })
    }

    /// Snapshot invalid controls in tree order. Radio requirements are grouped
    /// once for the form, including disabled peers in group checkedness.
    // DomString only caches its scalar projection; shared access cannot change
    // the authoritative UTF-16 units used by both Hash and Eq.
    #[allow(clippy::mutable_key_type)]
    pub fn invalid_form_controls(&self, form: NodeId) -> Result<Vec<NodeId>, ValidationError> {
        let budget = regress_bounded::Budget::default();
        let controls = self.form_controls(form);
        let candidates = self.validation_candidates_in_tree(self.tree_root(form));
        let mut radios = std::collections::HashMap::new();
        for &id in &controls {
            if let Some(name) = self.radio_name(id) {
                let state = radios.entry(name).or_insert((false, false));
                state.0 |= self.null_attribute(id, "required").is_some();
                state.1 |= self.checkedness(id);
            }
        }
        let mut invalid = Vec::new();
        for id in controls {
            if !candidates.contains(&id) {
                continue;
            }
            let radio_missing = self
                .radio_name(id)
                .and_then(|name| radios.get(name))
                .map(|&(required, checked)| required && !checked);
            if !self
                .control_validity_with_radio(id, radio_missing, true, &budget)?
                .valid()
            {
                invalid.push(id);
            }
        }
        Ok(invalid)
    }

    /// Carry ancestor eligibility in tree order once, so a form with many
    /// deeply nested controls does not repeatedly scan the same ancestor chain.
    fn validation_candidates_in_tree(&self, root: NodeId) -> std::collections::HashSet<NodeId> {
        let mut contexts = std::collections::HashMap::<NodeId, (bool, bool)>::new();
        let mut candidates = std::collections::HashSet::new();
        for id in crate::traversal::TreeTraverser::new_with_root(self, root) {
            let node = &self.nodes[id];
            let (mut disabled, mut datalist) = node
                .parent
                .and_then(|p| contexts.get(&p).copied())
                .unwrap_or_default();
            if let Some(parent) = node.parent.and_then(|p| self.get_node(p))
                && parent.fieldset_disables(node)
            {
                disabled = true;
            }
            if let Some(e) = node.element_data().filter(|e| e.name.ns == ns!(html)) {
                datalist |= &*e.name.local == "datalist";
                if !disabled
                    && !datalist
                    && !e.has_attr(local_name!("disabled"))
                    && Self::validation_type_candidate(e)
                {
                    candidates.insert(id);
                }
            }
            contexts.insert(id, (disabled, datalist));
        }
        candidates
    }

    fn required_value_missing(
        &self,
        id: NodeId,
        radio_missing: Option<bool>,
        known_candidate: bool,
    ) -> bool {
        let e = self.nodes[id]
            .element_data()
            .expect("validation candidate is an element");
        let kind = e
            .attr_dom(local_name!("type"))
            .map(crate::DomString::as_str_lossy)
            .unwrap_or("")
            .to_ascii_lowercase();
        if !known_candidate
            && matches!(&*e.name.local, "input" | "textarea")
            && (self.nodes[id].is_disabled() || (!Self::validation_type_candidate(e)))
        {
            return false;
        }
        if &*e.name.local == "input" && kind == "radio" {
            return radio_missing.unwrap_or_else(|| self.radio_group_value_missing(id));
        }
        if !e.has_attr(local_name!("required")) {
            return false;
        }
        match &*e.name.local {
            "input" if kind == "checkbox" => !self.checkedness(id),
            "input" if kind == "file" => {
                if !e.form_state.files.is_empty() {
                    return false;
                }
                #[cfg(feature = "file-input")]
                {
                    e.file_data().is_none_or(|files| files.is_empty())
                }
                #[cfg(not(feature = "file-input"))]
                {
                    true
                }
            }
            "input" if matches!(kind.as_str(), "submit" | "image" | "range" | "color") => false,
            "input" | "textarea" => self.form_value(id).is_none_or(|v| v.is_empty()),
            "select" => self.select_value_missing(id),
            _ => false,
        }
    }

    fn select_value_missing(&self, id: NodeId) -> bool {
        let selected = self.selected_options(id);
        if selected.is_empty() {
            return true;
        }
        let e = self.nodes[id]
            .element_data()
            .expect("select candidate is an element");
        if e.attr_dom(local_name!("multiple")).is_some()
            || e.attr(local_name!("size"))
                .and_then(|v| style::servo::attr::parse_unsigned_integer(v.chars()).ok())
                .is_some_and(|size| size > 1)
        {
            return false;
        }
        let first = self.select_options(id).first().copied();
        selected.len() == 1
            && first == selected.first().copied()
            && self.nodes[selected[0]].parent == Some(id)
            && self.form_value(selected[0]).is_none_or(|v| v.is_empty())
    }

    fn radio_group_value_missing(&self, id: NodeId) -> bool {
        let Some(name) = self.radio_name(id) else {
            return self.null_attribute(id, "required").is_some() && !self.checkedness(id);
        };
        let root = self.tree_root(id);
        let owners = self.form_owners_in_tree(root);
        let owner = owners.get(&id);
        let mut required = false;
        for peer in crate::traversal::TreeTraverser::new_with_root(self, root) {
            if self.radio_name(peer) == Some(name) && owners.get(&peer) == owner {
                if self.checkedness(peer) {
                    return false;
                }
                required |= self.null_attribute(peer, "required").is_some();
            }
        }
        required
    }
}

impl DocumentMutator<'_> {
    /// Custom validity is live state, independent of default values and reset.
    pub fn set_custom_validity(&mut self, id: NodeId, message: DomString) {
        if let Some(e) = self
            .doc
            .nodes
            .get_mut(id)
            .and_then(|n| n.element_data_mut())
        {
            e.form_state.custom_validity = message;
        }
    }
}
