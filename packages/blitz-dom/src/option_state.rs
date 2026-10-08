//! HTML option selectedness. Tree and attribute mutation call this owner;
//! readers never infer a selection or change defaults.
//! https://html.spec.whatwg.org/multipage/form-elements.html#the-option-element

use crate::{BaseDocument, DocumentMutator, NodeId};
use markup5ever::{QualName, local_name, ns};
use style_dom::ElementState;

impl BaseDocument {
    fn option_tag(&self, id: NodeId) -> Option<&str> {
        let el = self.get_node(id)?.element_data()?;
        (el.name.ns == ns!(html)).then_some(&*el.name.local)
    }

    /// Option text excludes script descendants and collapses only HTML space.
    pub fn option_text(&self, option: NodeId) -> crate::DomString {
        let mut pending = self.nodes[option]
            .children
            .iter()
            .rev()
            .copied()
            .collect::<Vec<_>>();
        let mut units = Vec::new();
        let mut space = false;
        while let Some(id) = pending.pop() {
            if self.option_tag(id) == Some("script") {
                continue;
            }
            if let Some(text) = self.nodes[id].text_data() {
                for unit in text.content.to_utf16().iter().copied() {
                    if matches!(unit, 9 | 10 | 12 | 13 | 32) {
                        space = !units.is_empty();
                    } else {
                        if space {
                            units.push(32);
                        }
                        space = false;
                        units.push(unit);
                    }
                }
            }
            pending.extend(self.nodes[id].children.iter().rev().copied());
        }
        crate::DomString::from_utf16(units)
    }

    /// Owning select, excluding options nested inside other options/datalists.
    pub fn option_select(&self, id: NodeId) -> Option<NodeId> {
        self.get_node(id)?.select_owner
    }

    /// Iterative tree-order enumeration, bounded by the document node quota.
    pub fn select_options(&self, select: NodeId) -> Vec<NodeId> {
        self.select_option_entries(select)
            .into_iter()
            .map(|(id, _)| id)
            .collect()
    }

    // Carry optgroup disabledness once through the tree, rather than walking
    // the same deep ancestry separately for each option.
    fn select_option_entries(&self, select: NodeId) -> Vec<(NodeId, bool)> {
        if self.option_tag(select) != Some("select") {
            return Vec::new();
        }
        let mut pending: Vec<_> = self.nodes[select]
            .children
            .iter()
            .rev()
            .map(|&id| (id, false))
            .collect();
        let mut options = Vec::new();
        while let Some((id, mut disabled)) = pending.pop() {
            match self.option_tag(id) {
                Some("option") => options.push((
                    id,
                    disabled || self.null_attribute(id, "disabled").is_some(),
                )),
                Some("select" | "datalist" | "hr") => {}
                tag => {
                    if tag == Some("optgroup") {
                        disabled = self.null_attribute(id, "disabled").is_some();
                    }
                    pending.extend(
                        self.nodes[id]
                            .children
                            .iter()
                            .rev()
                            .map(|&id| (id, disabled)),
                    );
                }
            }
        }
        options
    }

    /// Successful selected options for form entries, excluding disabled groups.
    pub fn selected_enabled_options(&self, select: NodeId) -> Vec<NodeId> {
        self.select_option_entries(select)
            .into_iter()
            .filter(|&(id, disabled)| !disabled && self.option_selectedness(id))
            .map(|(id, _)| id)
            .collect()
    }

    /// Current selectedness; a never-assigned option starts from its attribute.
    pub fn option_selectedness(&self, id: NodeId) -> bool {
        self.get_node(id)
            .and_then(|n| n.element_data())
            .is_some_and(|el| {
                el.form_state
                    .selected
                    .unwrap_or_else(|| el.has_attr(local_name!("selected")))
            })
    }

    /// All selected options in tree order, without inventing a default.
    pub fn selected_options(&self, select: NodeId) -> Vec<NodeId> {
        self.select_options(select)
            .into_iter()
            .filter(|&id| self.option_selectedness(id))
            .collect()
    }

    /// First selected option, or None for an explicit no-selection state.
    pub fn selected_option(&self, select: NodeId) -> Option<NodeId> {
        self.select_options(select)
            .into_iter()
            .find(|&id| self.option_selectedness(id))
    }
}

impl DocumentMutator<'_> {
    /// Script selection dirties only the explicitly assigned option.
    pub fn set_option_selectedness(&mut self, option: NodeId, selected: bool) {
        let changed = self.doc.option_selectedness(option) != selected;
        self.write_option_selectedness(option, selected, Some(true));
        if let Some(select) = self.doc.option_select(option) {
            if selected {
                self.deselect_other_options(select, option);
            }
            if changed {
                self.reconcile_select(select);
            }
        }
    }

    /// The legacy Option factory overrides selectedness without dirtying it.
    pub fn initialize_option_selectedness(&mut self, option: NodeId, selected: bool) {
        self.write_option_selectedness(option, selected, Some(false));
    }

    /// Select only this index (or nothing when out of range), dirtying the match.
    pub fn set_select_index(&mut self, select: NodeId, index: i32) {
        let options = self.doc.select_options(select);
        let wanted = usize::try_from(index)
            .ok()
            .and_then(|i| options.get(i).copied());
        self.select_only(&options, wanted);
    }

    /// Select only the first matching value, even in a multiple select.
    /// No match clears selectedness; only a matching option becomes dirty.
    pub fn set_select_value(&mut self, select: NodeId, value: &str) {
        let options = self.doc.select_options(select);
        let wanted = options
            .iter()
            .copied()
            .find(|&id| self.doc.form_value(id).as_deref() == Some(value));
        self.select_only(&options, wanted);
    }

    fn select_only(&mut self, options: &[NodeId], wanted: Option<NodeId>) {
        for &option in options {
            let selected = wanted == Some(option);
            self.write_option_selectedness(option, selected, selected.then_some(true));
        }
    }

    /// Form reset restores defaults and clears every option's dirtiness.
    pub fn reset_select(&mut self, select: NodeId) {
        for option in self.doc.select_options(select) {
            let selected = self.doc.null_attribute(option, "selected").is_some();
            self.write_option_selectedness(option, selected, Some(false));
        }
        self.reconcile_select(select);
    }

    pub(crate) fn write_option_selectedness(
        &mut self,
        option: NodeId,
        selected: bool,
        dirty: Option<bool>,
    ) {
        self.doc
            .snapshot_node_and(option, ElementState::CHECKED, |node| {
                if let Some(el) = node.element_data_mut() {
                    el.form_state.selected = Some(selected);
                    if let Some(dirty) = dirty {
                        el.form_state.selected_dirty = dirty;
                    }
                    el.element_state.set(ElementState::CHECKED, selected);
                }
                node.mark_ancestors_dirty();
            });
    }

    fn deselect_other_options(&mut self, select: NodeId, selected: NodeId) {
        if self.selection_scratch == Some(select) {
            return;
        }
        if self.doc.null_attribute(select, "multiple").is_some() {
            return;
        }
        for option in self.doc.select_options(select) {
            if option != selected && self.doc.option_selectedness(option) {
                self.write_option_selectedness(option, false, None);
            }
        }
    }

    pub(crate) fn reconcile_select(&mut self, select: NodeId) {
        if self.selection_scratch == Some(select) {
            return;
        }
        if self.doc.option_tag(select) != Some("select")
            || self.doc.null_attribute(select, "multiple").is_some()
        {
            return;
        }
        let options = self.doc.select_option_entries(select);
        let last_selected = options
            .iter()
            .map(|&(id, _)| id)
            .rfind(|&id| self.doc.option_selectedness(id));
        if let Some(selected) = last_selected {
            self.deselect_other_options(select, selected);
        } else if self
            .doc
            .null_attribute(select, "size")
            .map_or(0, |a| display_size(a.value.as_str_lossy()))
            <= 1
        {
            if let Some((option, _)) = options.into_iter().find(|&(_, disabled)| !disabled) {
                self.write_option_selectedness(option, true, None);
            }
        }
    }

    pub(crate) fn selection_attribute_changed(&mut self, id: NodeId, name: &QualName) {
        if name.ns != ns!() {
            return;
        }
        match (self.doc.option_tag(id), &*name.local) {
            (Some("select"), "multiple") => {
                if self.doc.null_attribute(id, "multiple").is_none() {
                    if let Some(first) = self.doc.selected_option(id) {
                        self.deselect_other_options(id, first);
                    }
                }
                self.reconcile_select(id);
            }
            (Some("select"), "size") => self.reconcile_select(id),
            (Some("option"), "selected") => {
                let dirty = self.doc.nodes[id]
                    .element_data()
                    .is_some_and(|el| el.form_state.selected_dirty);
                if !dirty {
                    let selected = self.doc.null_attribute(id, "selected").is_some();
                    self.write_option_selectedness(id, selected, None);
                    if selected {
                        if let Some(select) = self.doc.option_select(id) {
                            self.deselect_other_options(select, id);
                        }
                    }
                }
                if let Some(select) = self.doc.option_select(id) {
                    self.reconcile_select(select);
                }
            }
            _ => {}
        }
    }

    pub(crate) fn selection_inserted(&mut self, children: &[NodeId]) {
        // Inserted selected options take precedence even before the old selection.
        let mut pending = children.iter().rev().copied().collect::<Vec<_>>();
        let mut selected = std::collections::HashMap::new();
        while let Some(id) = pending.pop() {
            let owner =
                self.doc.nodes[id]
                    .parent
                    .and_then(|parent| match self.doc.option_tag(parent) {
                        Some("select") => Some(parent),
                        Some("option" | "datalist" | "hr") => None,
                        _ => self.doc.nodes[parent].select_owner,
                    });
            let changed = self.doc.nodes[id].select_owner != owner;
            self.doc.nodes[id].select_owner = owner;
            if self.doc.option_tag(id) == Some("option") && self.doc.option_selectedness(id) {
                if let Some(select) = owner {
                    selected.insert(select, id);
                }
            }
            // Descendant ownership is unchanged when the inherited context is
            // unchanged. This keeps unrelated deep-tree construction linear.
            if changed {
                pending.extend(self.doc.nodes[id].children.iter().rev().copied());
            }
        }
        for (select, option) in selected {
            self.deselect_other_options(select, option);
        }
    }

    pub(crate) fn selection_children_changed(&mut self, parent: NodeId) {
        if matches!(
            self.doc.option_tag(parent),
            Some("option" | "datalist" | "hr")
        ) {
            return;
        }
        let select = if self.doc.option_tag(parent) == Some("select") {
            Some(parent)
        } else {
            self.doc.option_select(parent)
        };
        if let Some(select) = select {
            self.reconcile_select(select);
        }
    }
}

// HTML nonnegative integer parsing accepts a numeric prefix, not just Rust literals.
fn display_size(value: &str) -> u32 {
    let value = value.trim_start_matches(['\t', '\n', '\u{c}', '\r', ' ']);
    let value = value.strip_prefix('+').unwrap_or(value);
    let end = value.bytes().take_while(u8::is_ascii_digit).count();
    value[..end].parse().unwrap_or(0)
}
