//! Script-free persisted form state, with admission before snapshot allocation.
//! User state is scoped to a history document by the embedder. Matching uses
//! control signatures and occurrence, never IDs from a retired native arena.
//! <https://html.spec.whatwg.org/multipage/browsing-the-web.html#restore-persisted-state>
use crate::{BaseDocument, DomString, NodeId, custom_internals::CustomFormValue, ns};
use blitz_traits::net::{EntryValue, FormFile};
use std::collections::HashMap;

pub const MAX_CONTROLS: usize = 4096;
pub const MAX_BYTES: usize = 16 * 1024 * 1024;
// Bound ancestry-based native helpers as well as retained control payloads.
const MAX_TREE_DEPTH: usize = 256;
const MAX_TREE_NODES: usize = 65_536;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ControlKey {
    pub local_name: String,
    pub kind: String,
    pub name: DomString,
    pub id: DomString,
    pub form: Option<FormKey>,
    pub scope: Vec<DomString>,
    pub occurrence: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FormKey {
    pub id: DomString,
    pub name: DomString,
    pub occurrence: usize,
}

#[derive(Clone, Copy)]
enum ControlKind {
    Text,
    Checked,
    Selected,
    Files,
    Custom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RestoreError {
    Limit,
    Mismatch,
    File(crate::FormFileError),
}
impl std::fmt::Display for RestoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Limit => "Persisted form state limit exceeded",
            Self::Mismatch => "Persisted control state does not match",
            Self::File(_) => "Persisted file state rejected",
        })
    }
}
impl std::error::Error for RestoreError {}

#[derive(Clone, Debug)]
pub enum ControlState {
    Text(DomString),
    Checked(bool),
    Selected(Vec<(DomString, bool)>),
    Files(Vec<FormFile>),
    Custom(CustomFormValue),
}

#[derive(Clone, Debug)]
pub struct PersistedControl {
    pub key: ControlKey,
    pub state: ControlState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FormStateLimit;
impl std::fmt::Display for FormStateLimit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Persisted form state limit exceeded")
    }
}
impl std::error::Error for FormStateLimit {}

impl BaseDocument {
    /// Capture only connected, restorable controls. No JS getters or I/O run.
    /// Files retain authorized bytes, never ambient filesystem paths.
    pub fn capture_form_state(&self) -> Result<Vec<PersistedControl>, FormStateLimit> {
        let controls = self.persisted_control_keys()?;
        let mut total = 0usize;
        for (id, key) in &controls {
            total = total.saturating_add(key_bytes(key));
            total = total.saturating_add(self.control_state_bytes(*id)?);
            if total > MAX_BYTES {
                return Err(FormStateLimit);
            }
        }
        Ok(controls
            .into_iter()
            .filter_map(|(id, key)| {
                self.capture_control(id)
                    .map(|state| PersistedControl { key, state })
            })
            .collect())
    }

    /// Recompute signatures of registered form controls in tree order.
    /// Call after registration when matching a pending custom-element upgrade.
    // DomString's UTF-16 identity is immutable; its scalar cache is not a key.
    #[allow(clippy::mutable_key_type)]
    pub fn persisted_control_keys(&self) -> Result<Vec<(NodeId, ControlKey)>, FormStateLimit> {
        let nodes = self.persisted_tree_nodes()?;
        let forms = self.persisted_form_keys(&nodes)?;
        // Uniquely identified shadow hosts give persistent tree scopes. An
        // anonymous or duplicate-ID host is deliberately not restored: its
        // positional identity could disclose another shadow tree's state.
        let identities = self.persisted_identity_counts(&nodes)?;
        let mut owners = HashMap::new();
        for &id in &nodes {
            if id == self.root_node().id || self.nodes[id].is_shadow_root() {
                owners.extend(self.form_owners_in_tree(id));
            }
        }
        let mut occurrences = HashMap::new();
        let mut controls = Vec::new();
        let mut bytes = 0usize;
        for id in nodes {
            let owner = owners.get(&id).copied().flatten();
            let Some(mut key) =
                self.persisted_control_key(id, owner, &forms, &identities, bytes)?
            else {
                continue;
            };
            bytes = bytes.saturating_add(key_bytes(&key).saturating_mul(2));
            if bytes > MAX_BYTES {
                return Err(FormStateLimit);
            }
            let mut counter_key = key.clone();
            if key.local_name.contains('-') {
                counter_key.form = None;
            }
            let next = occurrences.entry(counter_key).or_insert(0);
            key.occurrence = *next;
            *next += 1;
            if self.control_kind(id).is_some() {
                if controls.len() >= MAX_CONTROLS {
                    return Err(FormStateLimit);
                }
                controls.push((id, key));
            }
        }
        Ok(controls)
    }

    /// Admit nesting before tree-root ownership routines walk host ancestors.
    fn persisted_tree_nodes(&self) -> Result<Vec<NodeId>, FormStateLimit> {
        let mut stack = vec![(self.root_node().id, 0usize, 0usize)];
        let mut nodes = Vec::new();
        while let Some((id, shadow_depth, depth)) = stack.pop() {
            if shadow_depth > 64 || depth > MAX_TREE_DEPTH || nodes.len() >= MAX_TREE_NODES {
                return Err(FormStateLimit);
            }
            let node = &self.nodes[id];
            if nodes
                .len()
                .saturating_add(stack.len())
                .saturating_add(node.children.len())
                .saturating_add(2)
                > MAX_TREE_NODES
            {
                return Err(FormStateLimit);
            }
            stack.extend(
                node.children
                    .iter()
                    .rev()
                    .map(|&child| (child, shadow_depth, depth + 1)),
            );
            if let Some(root) = node.element_data().and_then(|e| e.shadow_root) {
                stack.push((root, shadow_depth + 1, depth + 1));
            }
            nodes.push(id);
        }
        Ok(nodes)
    }

    // UTF-16 identity stays immutable despite the lazy scalar cache.
    #[allow(clippy::mutable_key_type)]
    fn persisted_control_key(
        &self,
        id: NodeId,
        owner: Option<NodeId>,
        forms: &HashMap<NodeId, FormKey>,
        identities: &HashMap<&DomString, usize>,
        bytes: usize,
    ) -> Result<Option<ControlKey>, FormStateLimit> {
        let Some(element) = self.nodes[id].element_data() else {
            return Ok(None);
        };
        let local = element.name.local.as_ref();
        if element.name.ns != ns!(html) {
            return Ok(None);
        }
        let kind = if local == "input" {
            let value = element.attr_dom(markup5ever::local_name!("type"));
            if value.is_some_and(|v| v.retained_bytes() > 256) {
                return Err(FormStateLimit);
            }
            value
                .map_or("text", DomString::as_str_lossy)
                .to_ascii_lowercase()
        } else if local == "select" {
            if element
                .attr_dom(markup5ever::local_name!("multiple"))
                .is_some()
            {
                "select-multiple".to_owned()
            } else {
                "select-one".to_owned()
            }
        } else {
            String::new()
        };
        if self.control_kind(id).is_none() && !local.contains('-') {
            return Ok(None);
        }
        if matches!(
            kind.as_str(),
            "password" | "hidden" | "submit" | "image" | "reset" | "button"
        ) || self.persistence_disabled(id, owner)
        {
            return Ok(None);
        }
        let Some(scope) = self.persisted_scope(id, identities)? else {
            return Ok(None);
        };
        // Check the borrowed metadata before retaining two copies (the
        // output signature and its occurrence-counter key).
        let id_bytes = element
            .attr_dom(markup5ever::local_name!("id"))
            .map_or(0, DomString::retained_bytes);
        let name_bytes = element
            .attr_dom(markup5ever::local_name!("name"))
            .map_or(0, DomString::retained_bytes);
        if bytes.saturating_add(
            256 + local.len() * 2
                + kind.len() * 2
                + name_bytes.saturating_add(id_bytes).saturating_mul(2),
        ) > MAX_BYTES
        {
            return Err(FormStateLimit);
        }
        let name = element
            .attr_dom(markup5ever::local_name!("name"))
            .cloned()
            .unwrap_or_default();
        let key = ControlKey {
            local_name: local.to_owned(),
            kind,
            name,
            id: element
                .attr_dom(markup5ever::local_name!("id"))
                .cloned()
                .unwrap_or_default(),
            form: owner.and_then(|id| forms.get(&id).cloned()),
            scope,
            occurrence: 0,
        };
        Ok(Some(key))
    }

    fn control_kind(&self, id: NodeId) -> Option<ControlKind> {
        let element = self.nodes[id].element_data()?;
        if self.is_form_associated_custom(id) {
            return Some(ControlKind::Custom);
        }
        if self.is_file_input(id) {
            return Some(ControlKind::Files);
        }
        if self.is_checkable_input(id) {
            return Some(ControlKind::Checked);
        }
        match element.name.local.as_ref() {
            "select" => Some(ControlKind::Selected),
            "input" | "textarea" => Some(ControlKind::Text),
            _ => None,
        }
    }

    fn control_state_bytes(&self, id: NodeId) -> Result<usize, FormStateLimit> {
        let element = self.nodes[id].element_data().ok_or(FormStateLimit)?;
        match self.control_kind(id).ok_or(FormStateLimit)? {
            ControlKind::Custom => element
                .custom_internals
                .as_ref()
                .map_or(Ok(0), |i| custom_bytes(&i.state)),
            ControlKind::Files => files_bytes(&element.form_state.files),
            ControlKind::Checked => Ok(1),
            ControlKind::Selected => {
                let mut bytes = 0usize;
                for option in self.select_options(id) {
                    let raw = self.null_attribute(option, "value").map_or_else(
                        || {
                            crate::traversal::TreeTraverser::new_with_root(self, option)
                                .filter_map(|id| self.nodes[id].text_data())
                                .fold(0usize, |bytes, text| {
                                    bytes.saturating_add(text.content.retained_bytes())
                                })
                        },
                        |attribute| attribute.value.retained_bytes(),
                    );
                    bytes = bytes
                        .saturating_add(size_of::<FormKey>())
                        .saturating_add(raw.saturating_mul(2));
                    if bytes > MAX_BYTES {
                        return Err(FormStateLimit);
                    }
                }
                Ok(bytes)
            }
            ControlKind::Text => {
                // Charge borrowed raw text before cloning or sanitization. UTF-8
                // may expand into UTF-16; reserve that representation as well.
                let raw = if let Some(value) = &element.form_state.value {
                    value.retained_bytes()
                } else if element.name.local.as_ref() == "textarea" {
                    self.nodes[id]
                        .children
                        .iter()
                        .filter_map(|&id| self.nodes[id].text_data())
                        .fold(0usize, |bytes, text| {
                            bytes.saturating_add(text.content.retained_bytes())
                        })
                } else {
                    element
                        .attr_dom(markup5ever::local_name!("value"))
                        .map_or(0, DomString::retained_bytes)
                };
                Ok(raw.saturating_mul(2))
            }
        }
    }

    fn persistence_disabled(&self, id: NodeId, owner: Option<NodeId>) -> bool {
        let policy = self
            .null_attribute(id, "autocomplete")
            .map(|a| &a.value)
            .or_else(|| {
                owner.and_then(|id| self.null_attribute(id, "autocomplete").map(|a| &a.value))
            });
        policy.is_some_and(|value| {
            value.retained_bytes() <= 32 && value.as_str_lossy().eq_ignore_ascii_case("off")
        })
    }

    // DomString code units are immutable; its lazy scalar cache is not identity.
    #[allow(clippy::mutable_key_type)]
    fn persisted_identity_counts<'a>(
        &'a self,
        nodes: &[NodeId],
    ) -> Result<HashMap<&'a DomString, usize>, FormStateLimit> {
        let mut counts = HashMap::new();
        let mut bytes = 0usize;
        for &id in nodes {
            if let Some(attribute) = self.null_attribute(id, "id") {
                bytes = bytes.saturating_add(attribute.value.retained_bytes());
                if bytes > MAX_BYTES {
                    return Err(FormStateLimit);
                }
                *counts.entry(&attribute.value).or_insert(0) += 1;
            }
        }
        Ok(counts)
    }

    // DomString code units are immutable; its lazy scalar cache is not identity.
    #[allow(clippy::mutable_key_type)]
    fn persisted_scope(
        &self,
        id: NodeId,
        identities: &HashMap<&DomString, usize>,
    ) -> Result<Option<Vec<DomString>>, FormStateLimit> {
        let mut root = self.tree_root(id);
        let mut scope = Vec::new();
        let mut bytes = 0usize;
        while let Some(shadow) = &self.nodes[root].shadow_root_data {
            let Some(attribute) = self.null_attribute(shadow.host, "id") else {
                return Ok(None);
            };
            if attribute.value.is_empty() || identities.get(&attribute.value) != Some(&1) {
                return Ok(None);
            }
            bytes = bytes.saturating_add(attribute.value.retained_bytes());
            if bytes > MAX_BYTES || scope.len() >= 64 {
                return Err(FormStateLimit);
            }
            scope.push(attribute.value.clone());
            root = self.tree_root(shadow.host);
        }
        scope.reverse();
        Ok(Some(scope))
    }

    // DomString code units are immutable; its lazy scalar cache is not identity.
    #[allow(clippy::mutable_key_type)]
    fn persisted_form_keys(
        &self,
        nodes: &[NodeId],
    ) -> Result<HashMap<NodeId, FormKey>, FormStateLimit> {
        let mut forms = HashMap::new();
        let mut occurrences = HashMap::new();
        let mut bytes = 0usize;
        for &node in nodes {
            if !self.is_html_form(node) {
                continue;
            }
            let attr = |name| self.null_attribute(node, name).map(|a| &a.value);
            let id = attr("id");
            let name = attr("name");
            bytes = bytes
                .saturating_add(128)
                .saturating_add(id.map_or(0, DomString::retained_bytes).saturating_mul(2))
                .saturating_add(name.map_or(0, DomString::retained_bytes).saturating_mul(2));
            if bytes > MAX_BYTES {
                return Err(FormStateLimit);
            }
            let mut key = FormKey {
                id: id.cloned().unwrap_or_default(),
                name: name.cloned().unwrap_or_default(),
                occurrence: 0,
            };
            let next = occurrences
                .entry((self.tree_root(node), key.clone()))
                .or_insert(0);
            key.occurrence = *next;
            *next += 1;
            forms.insert(node, key);
        }
        Ok(forms)
    }

    fn capture_control(&self, id: NodeId) -> Option<ControlState> {
        let element = self.nodes[id].element_data()?;
        match self.control_kind(id)? {
            ControlKind::Custom => {
                let state = &element.custom_internals.as_ref()?.state;
                (!matches!(state, CustomFormValue::Null))
                    .then(|| ControlState::Custom(state.clone()))
            }
            ControlKind::Files => Some(ControlState::Files(element.form_state.files.to_vec())),
            ControlKind::Checked => Some(ControlState::Checked(self.checkedness(id))),
            ControlKind::Selected => Some(ControlState::Selected(
                self.select_options(id)
                    .into_iter()
                    .map(|id| {
                        (
                            self.form_value_dom(id).unwrap_or_default(),
                            self.option_selectedness(id),
                        )
                    })
                    .collect(),
            )),
            ControlKind::Text => self.form_value_dom(id).map(ControlState::Text),
        }
    }

    /// Apply ordinary control state without input/change dispatch. Custom state
    /// is delivered by the embedder after constructing its realm-local value;
    /// it must not overwrite the independently submitted internals.value.
    pub fn restore_control_state(
        &mut self,
        id: NodeId,
        state: &ControlState,
    ) -> Result<(), RestoreError> {
        if self.get_node(id).is_none() || !self.is_connected(id) {
            return Ok(());
        }
        if self
            .null_attribute(id, "type")
            .is_some_and(|a| a.value.retained_bytes() > 256)
        {
            return Err(RestoreError::Limit);
        }
        if state_bytes(state).map_err(|_| RestoreError::Limit)? > MAX_BYTES {
            return Err(RestoreError::Limit);
        }
        if !self.state_matches_control(id, state) {
            return Err(RestoreError::Mismatch);
        }
        match state {
            ControlState::Text(value) => {
                if self
                    .control_state_bytes(id)
                    .map_err(|_| RestoreError::Limit)?
                    > MAX_BYTES
                {
                    return Err(RestoreError::Limit);
                }
                self.mutate().set_form_value_dom(id, value);
            }
            ControlState::Checked(value) => self.mutate().set_checkedness(id, *value),
            ControlState::Selected(values) => {
                let options = self.select_options(id);
                if self.null_attribute(id, "multiple").is_none()
                    && values.iter().filter(|(_, selected)| *selected).count() > 1
                {
                    return Err(RestoreError::Mismatch);
                }
                if options.len() != values.len() {
                    return Err(RestoreError::Mismatch);
                }
                if self
                    .control_state_bytes(id)
                    .map_err(|_| RestoreError::Limit)?
                    > MAX_BYTES
                {
                    return Err(RestoreError::Limit);
                }
                if options
                    .iter()
                    .zip(values)
                    .any(|(&id, (value, _))| self.form_value_dom(id).as_ref() != Some(value))
                {
                    return Err(RestoreError::Mismatch);
                }
                for (option, (_, selected)) in options.into_iter().zip(values) {
                    self.mutate()
                        .write_option_selectedness(option, *selected, Some(true));
                }
            }
            ControlState::Files(files) => self
                .mutate()
                .set_form_files(id, files.clone())
                .map_err(RestoreError::File)?,
            ControlState::Custom(_) => {}
        }
        Ok(())
    }

    fn state_matches_control(&self, id: NodeId, state: &ControlState) -> bool {
        let Some(element) = self.get_node(id).and_then(|n| n.element_data()) else {
            return false;
        };
        if element.name.ns != ns!(html) {
            return false;
        }
        match state {
            ControlState::Text(_) => {
                matches!(element.name.local.as_ref(), "input" | "textarea")
                    && !self.is_file_input(id)
                    && !self.is_checkable_input(id)
            }
            ControlState::Checked(_) => self.is_checkable_input(id),
            ControlState::Selected(_) => element.name.local.as_ref() == "select",
            ControlState::Files(_) => self.is_file_input(id),
            ControlState::Custom(_) => self.is_form_associated_custom(id),
        }
    }
}

fn key_bytes(key: &ControlKey) -> usize {
    size_of::<PersistedControl>()
        .saturating_add(key.local_name.capacity())
        .saturating_add(key.kind.capacity())
        .saturating_add(key.name.retained_bytes())
        .saturating_add(key.id.retained_bytes())
        .saturating_add(key.scope.capacity() * size_of::<DomString>())
        .saturating_add(key.scope.iter().fold(0usize, |bytes, id| {
            bytes.saturating_add(id.retained_bytes())
        }))
        .saturating_add(key.form.as_ref().map_or(0, |form| {
            form.id
                .retained_bytes()
                .saturating_add(form.name.retained_bytes())
                .saturating_add(64)
        }))
}
fn file_bytes(file: &FormFile) -> usize {
    size_of::<FormFile>()
        .saturating_add(file.name.len())
        .saturating_add(file.content_type.len())
        .saturating_add(file.bytes.len())
}
fn value_bytes(value: &EntryValue) -> Result<usize, FormStateLimit> {
    match value {
        EntryValue::String(s) => Ok(s.len()),
        EntryValue::FileContents(file) => Ok(file_bytes(file)),
        EntryValue::EmptyFile => Ok(64),
        EntryValue::File(_) => Err(FormStateLimit),
    }
}
fn custom_bytes(value: &CustomFormValue) -> Result<usize, FormStateLimit> {
    match value {
        CustomFormValue::Null => Ok(0),
        CustomFormValue::Single(value) => value_bytes(value),
        CustomFormValue::Entries(entries) => {
            if entries.len() > 16_384 {
                return Err(FormStateLimit);
            }
            entries.iter().try_fold(0usize, |bytes, entry| {
                Ok(bytes
                    .saturating_add(size_of::<blitz_traits::net::Entry>() + entry.name.len())
                    .saturating_add(value_bytes(&entry.value)?))
            })
        }
    }
}

fn files_bytes(files: &[FormFile]) -> Result<usize, FormStateLimit> {
    if files.len() > 256 {
        return Err(FormStateLimit);
    }
    Ok(files
        .iter()
        .fold(0usize, |bytes, file| bytes.saturating_add(file_bytes(file))))
}

fn state_bytes(state: &ControlState) -> Result<usize, FormStateLimit> {
    match state {
        ControlState::Text(value) => Ok(value.retained_bytes()),
        ControlState::Checked(_) => Ok(1),
        ControlState::Selected(values) => Ok(values.iter().fold(0usize, |bytes, (value, _)| {
            bytes
                .saturating_add(64)
                .saturating_add(value.retained_bytes())
        })),
        ControlState::Files(files) => files_bytes(files),
        ControlState::Custom(value) => custom_bytes(value),
    }
}

#[cfg(test)]
#[path = "form_history_tests.rs"]
mod tests;

/// Conservative aggregate admission for decoded persisted controls.
pub fn persisted_controls_bytes(controls: &[PersistedControl]) -> Result<usize, FormStateLimit> {
    if controls.len() > MAX_CONTROLS {
        return Err(FormStateLimit);
    }
    let bytes = controls.iter().try_fold(0usize, |bytes, c| {
        Ok(bytes
            .saturating_add(key_bytes(&c.key))
            .saturating_add(state_bytes(&c.state)?))
    })?;
    if bytes > MAX_BYTES {
        return Err(FormStateLimit);
    }
    Ok(bytes)
}
