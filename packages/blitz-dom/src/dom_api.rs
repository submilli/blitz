//! DOM Standard node operations, independent of any scripting engine.
//!
//! These implement the algorithms from <https://dom.spec.whatwg.org/> on top
//! of Blitz's tree (node types, text content, pre-insertion validity,
//! insertion, removal, replacement and cloning), so embedders exposing the DOM
//! to a script engine only translate arguments and results.
//!
//! Removal here *detaches* nodes; it never drops them. A detached node stays
//! valid (and re-insertable) for as long as the embedder holds its id.

use markup5ever::{QualName, ns};

use crate::node::{Attribute, NodeData};
use crate::{BaseDocument, DocumentMutator, NodeId};

/// Errors from DOM operations, named after the `DOMException` they map to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DomError {
    HierarchyRequest,
    NotFound,
    NotSupported,
    InvalidCharacter,
}

impl DomError {
    /// The `DOMException` name.
    pub fn name(self) -> &'static str {
        match self {
            Self::HierarchyRequest => "HierarchyRequestError",
            Self::NotFound => "NotFoundError",
            Self::NotSupported => "NotSupportedError",
            Self::InvalidCharacter => "InvalidCharacterError",
        }
    }
}

/// `Node.nodeType` values.
pub mod node_type {
    pub const ELEMENT: u16 = 1;
    pub const TEXT: u16 = 3;
    pub const PROCESSING_INSTRUCTION: u16 = 7;
    pub const COMMENT: u16 = 8;
    pub const DOCUMENT: u16 = 9;
    pub const DOCUMENT_TYPE: u16 = 10;
    pub const DOCUMENT_FRAGMENT: u16 = 11;
}

impl BaseDocument {

    /// `Node.nodeType`
    pub fn node_type(&self, id: NodeId) -> u16 {
        match &self.nodes[id].data {
            NodeData::Element(_) | NodeData::AnonymousBlock(_) => node_type::ELEMENT,
            NodeData::Text(_) => node_type::TEXT,
            NodeData::ProcessingInstruction { .. } => node_type::PROCESSING_INSTRUCTION,
            NodeData::Comment { .. } => node_type::COMMENT,
            NodeData::Document(_) => node_type::DOCUMENT,
            NodeData::Doctype { .. } => node_type::DOCUMENT_TYPE,
            NodeData::DocumentFragment => node_type::DOCUMENT_FRAGMENT,
        }
    }

    /// `Node.nodeName`
    pub fn node_name(&self, id: NodeId) -> String {
        match &self.nodes[id].data {
            NodeData::Element(el) | NodeData::AnonymousBlock(el) => html_uppercased_qualified_name(&el.name),
            NodeData::Text(_) => "#text".into(),
            NodeData::ProcessingInstruction { target, .. } => target.clone(),
            NodeData::Comment { .. } => "#comment".into(),
            NodeData::Document(_) => "#document".into(),
            NodeData::Doctype { name, .. } => name.clone(),
            NodeData::DocumentFragment => "#document-fragment".into(),
        }
    }

    /// The data of a `CharacterData` node (Text, Comment, ProcessingInstruction).
    pub fn character_data(&self, id: NodeId) -> Option<&str> {
        match &self.nodes[id].data {
            NodeData::Text(t) => Some(&t.content),
            NodeData::Comment { contents } => Some(contents),
            NodeData::ProcessingInstruction { contents, .. } => Some(contents),
            _ => None,
        }
    }

    /// `Node.textContent` getter. `None` (JS `null`) for documents and doctypes.
    pub fn text_content_of(&self, id: NodeId) -> Option<String> {
        match &self.nodes[id].data {
            NodeData::Document(_) | NodeData::Doctype { .. } => None,
            NodeData::Text(_) | NodeData::Comment { .. } | NodeData::ProcessingInstruction { .. } => {
                self.character_data(id).map(str::to_string)
            }
            _ => {
                let mut out = String::new();
                self.collect_descendant_text(id, &mut out);
                Some(out)
            }
        }
    }

    fn collect_descendant_text(&self, id: NodeId, out: &mut String) {
        for &child in &self.nodes[id].children {
            match &self.nodes[child].data {
                NodeData::Text(t) => out.push_str(&t.content),
                NodeData::Element(_) | NodeData::AnonymousBlock(_) => {
                    self.collect_descendant_text(child, out)
                }
                _ => {}
            }
        }
    }

    /// Whether `ancestor` is an inclusive ancestor of `node`.
    pub fn is_inclusive_ancestor(&self, ancestor: NodeId, node: NodeId) -> bool {
        let mut current = Some(node);
        while let Some(id) = current {
            if id == ancestor {
                return true;
            }
            current = self.nodes[id].parent;
        }
        false
    }

    /// The root of `node`'s tree (the document, or the top of a detached subtree).
    pub fn tree_root(&self, node: NodeId) -> NodeId {
        let mut id = node;
        while let Some(parent) = self.nodes[id].parent {
            id = parent;
        }
        id
    }

    /// `Node.isConnected`
    pub fn is_connected(&self, node: NodeId) -> bool {
        self.tree_root(node) == self.root_node().id
    }

    fn is_element(&self, id: NodeId) -> bool {
        matches!(self.nodes[id].data, NodeData::Element(_))
    }

    /// "Ensure pre-insertion validity" of `node` into `parent` before `child`.
    pub fn check_pre_insert(
        &self,
        node: NodeId,
        parent: NodeId,
        child: Option<NodeId>,
    ) -> Result<(), DomError> {
        let parent_kind = self.node_type(parent);
        if !matches!(
            parent_kind,
            node_type::DOCUMENT | node_type::DOCUMENT_FRAGMENT | node_type::ELEMENT
        ) {
            return Err(DomError::HierarchyRequest);
        }
        if self.is_inclusive_ancestor(node, parent) {
            return Err(DomError::HierarchyRequest);
        }
        if let Some(child) = child {
            if self.nodes[child].parent != Some(parent) {
                return Err(DomError::NotFound);
            }
        }
        let node_kind = self.node_type(node);
        if matches!(node_kind, node_type::DOCUMENT) {
            return Err(DomError::HierarchyRequest);
        }
        if (node_kind == node_type::TEXT && parent_kind == node_type::DOCUMENT)
            || (node_kind == node_type::DOCUMENT_TYPE && parent_kind != node_type::DOCUMENT)
        {
            return Err(DomError::HierarchyRequest);
        }
        if parent_kind == node_type::DOCUMENT {
            self.check_document_child_constraints(node, parent, child, None)?;
        }
        Ok(())
    }

    /// The document-specific rules of pre-insertion (and replacement, where
    /// `replacing` is the child being replaced).
    fn check_document_child_constraints(
        &self,
        node: NodeId,
        document: NodeId,
        child: Option<NodeId>,
        replacing: Option<NodeId>,
    ) -> Result<(), DomError> {
        let children = &self.nodes[document].children;
        let others = || children.iter().copied().filter(move |&c| Some(c) != replacing);
        let has_element_child = others().any(|c| self.is_element(c));
        let has_doctype_child = others().any(|c| self.node_type(c) == node_type::DOCUMENT_TYPE);
        let doctype_after_child = |child: NodeId| {
            children
                .iter()
                .skip_while(|&&c| c != child)
                .skip(1)
                .any(|&c| self.node_type(c) == node_type::DOCUMENT_TYPE)
        };
        let element_before_child = |child: NodeId| {
            children
                .iter()
                .take_while(|&&c| c != child)
                .any(|&c| Some(c) != replacing && self.is_element(c))
        };
        match self.node_type(node) {
            node_type::DOCUMENT_FRAGMENT => {
                let frag_children = &self.nodes[node].children;
                let elements = frag_children.iter().filter(|&&c| self.is_element(c)).count();
                if frag_children.iter().any(|&c| self.node_type(c) == node_type::TEXT) {
                    return Err(DomError::HierarchyRequest);
                }
                if elements > 1
                    || (elements == 1
                        && (has_element_child || child.is_some_and(doctype_after_child)))
                {
                    return Err(DomError::HierarchyRequest);
                }
            }
            node_type::ELEMENT => {
                if has_element_child || child.is_some_and(doctype_after_child) {
                    return Err(DomError::HierarchyRequest);
                }
            }
            node_type::DOCUMENT_TYPE
                if has_doctype_child
                    || child.is_some_and(element_before_child)
                    || (child.is_none() && has_element_child) =>
            {
                return Err(DomError::HierarchyRequest);
            }
            _ => {}
        }
        Ok(())
    }
}

/// An attribute's qualified name (`prefix:local` or `local`).
pub fn attribute_qualified_name(attr: &Attribute) -> String {
    match &attr.name.prefix {
        Some(prefix) => format!("{prefix}:{}", attr.name.local),
        None => attr.name.local.to_string(),
    }
}

impl BaseDocument {
    fn is_html_element(&self, id: NodeId) -> bool {
        self.nodes[id]
            .element_data()
            .is_some_and(|el| el.name.ns == ns!(html))
    }

    /// The name used for lookups by qualified name: lowercased for HTML
    /// elements, as the DOM requires in HTML documents.
    fn normalize_attribute_name(&self, id: NodeId, name: &str) -> String {
        if self.is_html_element(id) {
            name.to_ascii_lowercase()
        } else {
            name.to_string()
        }
    }

    /// "Get an attribute by name": the first attribute whose qualified name
    /// matches.
    pub fn attribute_by_name(&self, id: NodeId, name: &str) -> Option<&Attribute> {
        let name = self.normalize_attribute_name(id, name);
        self.nodes[id]
            .element_data()?
            .attrs()
            .iter()
            .find(|attr| attribute_qualified_name(attr) == name)
    }

    /// `Element.getAttributeNames()`
    pub fn attribute_names(&self, id: NodeId) -> Vec<String> {
        self.nodes[id]
            .element_data()
            .map(|el| el.attrs().iter().map(attribute_qualified_name).collect())
            .unwrap_or_default()
    }
}

/// Form controls whose value is edited as text.
fn is_text_control(el: &crate::node::ElementData) -> bool {
    match &*el.name.local {
        "textarea" => true,
        "input" => !matches!(
            el.attr(markup5ever::local_name!("type")).map(|t| t.to_ascii_lowercase()).as_deref(),
            Some("checkbox" | "radio" | "file" | "button" | "submit" | "reset" | "image" | "hidden")
        ),
        _ => false,
    }
}

fn is_checkable(el: &crate::node::ElementData) -> bool {
    &*el.name.local == "input"
        && matches!(
            el.attr(markup5ever::local_name!("type")).map(|t| t.to_ascii_lowercase()).as_deref(),
            Some("checkbox" | "radio")
        )
}

impl BaseDocument {
    /// The `value` IDL attribute of `input`, `textarea`, `select`, `option`
    /// and `button`: the current value, falling back to the defaults.
    pub fn form_value(&self, id: NodeId) -> Option<String> {
        let el = self.nodes[id].element_data()?;
        if el.name.ns != ns!(html) {
            return None;
        }
        match &*el.name.local {
            "input" | "textarea" => {
                if let Some(editor) = el.text_input_data() {
                    return Some(editor.editor.raw_text().to_string());
                }
                if el.form_state.value_dirty {
                    return Some(el.form_state.value.clone().unwrap_or_default());
                }
                if &*el.name.local == "textarea" {
                    return self.text_content_of(id);
                }
                // Checkboxes and radios default to "on".
                let default = if is_checkable(el) { "on" } else { "" };
                Some(el.attr(markup5ever::local_name!("value")).unwrap_or(default).to_string())
            }
            "option" => Some(match el.attr(markup5ever::local_name!("value")) {
                Some(v) => v.to_string(),
                None => self
                    .text_content_of(id)
                    .unwrap_or_default()
                    .split_ascii_whitespace()
                    .collect::<Vec<_>>()
                    .join(" "),
            }),
            "select" => {
                let option = self.selected_option(id)?;
                self.form_value(option)
            }
            "button" | "data" | "li" | "param" | "output" => {
                Some(el.attr(markup5ever::local_name!("value")).unwrap_or("").to_string())
            }
            _ => None,
        }
    }

    /// The first selected `<option>` of a `<select>` (the first option when
    /// none has the `selected` attribute).
    pub fn selected_option(&self, select: NodeId) -> Option<NodeId> {
        let mut options = Vec::new();
        self.collect_options(select, &mut options);
        options
            .iter()
            .copied()
            .find(|&o| self.nodes[o].element_data().is_some_and(|e| e.has_attr(markup5ever::local_name!("selected"))))
            .or_else(|| options.first().copied())
    }

    fn collect_options(&self, id: NodeId, out: &mut Vec<NodeId>) {
        for &child in &self.nodes[id].children {
            if let Some(el) = self.nodes[child].element_data() {
                match &*el.name.local {
                    "option" => out.push(child),
                    "optgroup" => self.collect_options(child, out),
                    _ => {}
                }
            }
        }
    }

    /// The `checked` IDL attribute (current checkedness).
    pub fn checkedness(&self, id: NodeId) -> bool {
        let Some(el) = self.nodes[id].element_data() else {
            return false;
        };
        if let Some(checked) = el.checkbox_input_checked() {
            return checked;
        }
        if el.form_state.checked_dirty {
            return el.form_state.checked.unwrap_or(false);
        }
        el.has_attr(markup5ever::local_name!("checked"))
    }
}

impl DocumentMutator<'_> {
    /// `input.value = ...` / `textarea.value = ...`: sets the current value
    /// and its dirty flag; the `value` attribute is untouched.
    pub fn set_form_value(&mut self, id: NodeId, value: &str) {
        let is_text = self.doc.nodes[id].element_data().is_some_and(is_text_control);
        if !is_text {
            // Other controls reflect `value` to the attribute.
            let _ = self.set_attribute_by_name(id, "value", value);
            return;
        }
        let doc = &mut *self.doc;
        let font_ctx = &doc.font_ctx;
        let layout_ctx = &mut doc.layout_ctx;
        let Some(el) = doc.nodes[id].element_data_mut() else {
            return;
        };
        el.form_state.value = Some(value.to_string());
        el.form_state.value_dirty = true;
        if let Some(input) = el.text_input_data_mut() {
            input.set_text(&mut font_ctx.lock().unwrap(), layout_ctx, value);
        }
    }

    /// `input.checked = ...`: sets the current checkedness and its dirty
    /// flag. Checking a radio button unchecks the others in its group.
    pub fn set_checkedness(&mut self, id: NodeId, checked: bool) {
        let radio_group = self.doc.nodes[id].element_data().and_then(|el| {
            (el.attr(markup5ever::local_name!("type")) == Some("radio"))
                .then(|| el.attr(markup5ever::local_name!("name")).map(str::to_string))
                .flatten()
        });
        if let (true, Some(group)) = (checked, radio_group) {
            let others: Vec<NodeId> = self
                .doc
                .nodes
                .iter()
                .filter(|(i, node)| {
                    *i != id
                        && node.data.downcast_element().is_some_and(|el| {
                            el.attr(markup5ever::local_name!("type")) == Some("radio")
                                && el.attr(markup5ever::local_name!("name")) == Some(group.as_str())
                        })
                })
                .map(|(i, _)| i)
                .collect();
            for other in others {
                self.write_checkedness(other, false);
            }
        }
        self.write_checkedness(id, checked);
    }

    fn write_checkedness(&mut self, id: NodeId, checked: bool) {
        self.doc
            .snapshot_node_and(id, style_dom::ElementState::CHECKED, |node| {
                if let Some(el) = node.element_data_mut() {
                    el.form_state.checked = Some(checked);
                    el.form_state.checked_dirty = true;
                    if el.checkbox_input_checked().is_some() {
                        el.set_checkbox_input_checked(checked);
                    } else {
                        el.element_state.set(style_dom::ElementState::CHECKED, checked);
                    }
                }
                node.mark_ancestors_dirty();
            });
    }

    /// `Element.setAttribute(name, value)`
    pub fn set_attribute_by_name(
        &mut self,
        id: NodeId,
        name: &str,
        value: &str,
    ) -> Result<(), DomError> {
        if !is_valid_attribute_name(name) {
            return Err(DomError::InvalidCharacter);
        }
        let qual = match self.doc.attribute_by_name(id, name) {
            Some(existing) => existing.name.clone(),
            None => QualName::new(
                None,
                ns!(),
                self.doc.normalize_attribute_name(id, name).into(),
            ),
        };
        self.set_attribute(id, qual, value);
        Ok(())
    }

    /// `Element.removeAttribute(name)`
    pub fn remove_attribute_by_name(&mut self, id: NodeId, name: &str) {
        if let Some(qual) = self.doc.attribute_by_name(id, name).map(|a| a.name.clone()) {
            self.clear_attribute(id, qual);
        }
    }

    /// `Element.toggleAttribute(name, force)`. Returns whether the attribute
    /// is present afterwards.
    pub fn toggle_attribute(
        &mut self,
        id: NodeId,
        name: &str,
        force: Option<bool>,
    ) -> Result<bool, DomError> {
        if !is_valid_attribute_name(name) {
            return Err(DomError::InvalidCharacter);
        }
        let present = self.doc.attribute_by_name(id, name).is_some();
        match (present, force) {
            (false, None | Some(true)) => {
                self.set_attribute_by_name(id, name, "")?;
                Ok(true)
            }
            (false, Some(false)) => Ok(false),
            (true, None | Some(false)) => {
                self.remove_attribute_by_name(id, name);
                Ok(false)
            }
            (true, Some(true)) => Ok(true),
        }
    }

    /// The nodes actually inserted for `node`: a fragment's children, or the node.
    fn insertion_nodes(&self, node: NodeId) -> Vec<NodeId> {
        if self.doc.node_type(node) == node_type::DOCUMENT_FRAGMENT {
            self.doc.nodes[node].children.to_vec()
        } else {
            vec![node]
        }
    }

    /// `parent.insertBefore(node, child)` (and `appendChild` with `child = None`).
    pub fn pre_insert(
        &mut self,
        node: NodeId,
        parent: NodeId,
        child: Option<NodeId>,
    ) -> Result<NodeId, DomError> {
        self.doc.check_pre_insert(node, parent, child)?;
        // "If child is node, set child to node's next sibling."
        let child = match child {
            Some(c) if c == node => self.next_sibling_id(node),
            other => other,
        };
        self.insert(node, parent, child);
        Ok(node)
    }

    fn insert(&mut self, node: NodeId, parent: NodeId, child: Option<NodeId>) {
        let nodes = self.insertion_nodes(node);
        if nodes.is_empty() {
            return;
        }
        match child {
            Some(anchor) => self.insert_nodes_before(anchor, &nodes),
            None => self.append_children(parent, &nodes),
        }
    }

    /// `parent.removeChild(child)`
    pub fn remove_child(&mut self, parent: NodeId, child: NodeId) -> Result<NodeId, DomError> {
        if self.doc.nodes[child].parent != Some(parent) {
            return Err(DomError::NotFound);
        }
        self.remove_node(child);
        Ok(child)
    }

    /// `parent.replaceChild(node, child)`
    pub fn replace_child(
        &mut self,
        parent: NodeId,
        node: NodeId,
        child: NodeId,
    ) -> Result<NodeId, DomError> {
        let doc = &*self.doc;
        let parent_kind = doc.node_type(parent);
        if !matches!(
            parent_kind,
            node_type::DOCUMENT | node_type::DOCUMENT_FRAGMENT | node_type::ELEMENT
        ) {
            return Err(DomError::HierarchyRequest);
        }
        if doc.is_inclusive_ancestor(node, parent) {
            return Err(DomError::HierarchyRequest);
        }
        if doc.nodes[child].parent != Some(parent) {
            return Err(DomError::NotFound);
        }
        let node_kind = doc.node_type(node);
        if node_kind == node_type::DOCUMENT
            || (node_kind == node_type::TEXT && parent_kind == node_type::DOCUMENT)
            || (node_kind == node_type::DOCUMENT_TYPE && parent_kind != node_type::DOCUMENT)
        {
            return Err(DomError::HierarchyRequest);
        }
        if parent_kind == node_type::DOCUMENT {
            doc.check_document_child_constraints(node, parent, Some(child), Some(child))?;
        }
        if child == node {
            return Ok(child);
        }
        let mut reference = self.next_sibling_id(child);
        if reference == Some(node) {
            reference = self.next_sibling_id(node);
        }
        self.remove_node(child);
        self.insert(node, parent, reference);
        Ok(child)
    }

    /// Replace all children of `parent` with `node` (or nothing), as used by
    /// the `textContent` and `innerHTML` setters.
    pub fn replace_all(&mut self, parent: NodeId, node: Option<NodeId>) {
        // Copy the list: removing children mutates it.
        let children: Vec<NodeId> = self.doc.nodes[parent].children.iter().copied().collect();
        for child in children {
            self.remove_node(child);
        }
        if let Some(node) = node {
            self.insert(node, parent, None);
        }
    }

    /// `Node.textContent` setter.
    pub fn set_text_content(&mut self, id: NodeId, value: &str) {
        match self.doc.node_type(id) {
            node_type::ELEMENT | node_type::DOCUMENT_FRAGMENT => {
                let text = (!value.is_empty()).then(|| self.create_text_node(value));
                self.replace_all(id, text);
            }
            node_type::TEXT | node_type::COMMENT | node_type::PROCESSING_INSTRUCTION => {
                self.set_character_data(id, value)
            }
            _ => {}
        }
    }

    /// Replace the data of a Text, Comment or ProcessingInstruction node.
    pub fn set_character_data(&mut self, id: NodeId, value: &str) {
        if matches!(self.doc.nodes[id].data, NodeData::Text(_)) {
            // Text changes affect layout, which set_node_text takes care of.
            self.set_node_text(id, value);
            return;
        }
        if let NodeData::Comment { contents } | NodeData::ProcessingInstruction { contents, .. } =
            &mut self.doc.nodes[id].data
        {
            *contents = value.to_string();
        }
    }

    /// Replace the children of `id` with the result of parsing `html` as a
    /// fragment in its context (the `innerHTML` setter). Old children are
    /// detached, not dropped. For `<template>`, replaces the template contents.
    pub fn set_inner_html_detaching(&mut self, id: NodeId, html: &str) {
        let target = self.try_template_contents(id).unwrap_or(id);
        self.replace_all(target, None);
        self.doc
            .html_parser_provider
            .clone()
            .parse_inner_html(self, target, html);
    }

    /// `Node.cloneNode(deep)`: the clone is detached.
    pub fn clone_node(&mut self, id: NodeId, deep: bool) -> NodeId {
        let copy = match self.doc.nodes[id].data.clone() {
            NodeData::Element(el) | NodeData::AnonymousBlock(el) => {
                let attrs: Vec<Attribute> = el.attrs().to_vec();
                let copy = self.create_element(el.name.clone(), attrs);
                // Template contents are cloned along with a deep clone.
                if deep {
                    if let Some(contents) = el.template_contents {
                        let copy_contents = self.template_contents(copy);
                        let children: Vec<NodeId> = self.doc.nodes[contents].children.iter().copied().collect();
                        for child in children {
                            let child_copy = self.clone_node(child, true);
                            self.append_children(copy_contents, &[child_copy]);
                        }
                    }
                }
                copy
            }
            NodeData::Text(t) => self.create_text_node(&t.content),
            NodeData::Comment { contents } => self.create_comment_node(&contents),
            NodeData::ProcessingInstruction { target, contents } => {
                self.create_processing_instruction(&target, &contents)
            }
            NodeData::Doctype {
                name,
                public_id,
                system_id,
            } => self.create_doctype(&name, &public_id, &system_id),
            NodeData::DocumentFragment => self.create_document_fragment(),
            // Cloning the document itself is not supported.
            NodeData::Document(_) => return id,
        };
        if deep {
            let children: Vec<NodeId> = self.doc.nodes[id].children.iter().copied().collect();
            for child in children {
                let child_copy = self.clone_node(child, true);
                self.append_children(copy, &[child_copy]);
            }
        }
        copy
    }
}

/// Element `nodeName`/`tagName`: the qualified name, ASCII-uppercased for
/// elements in the HTML namespace.
pub fn html_uppercased_qualified_name(name: &QualName) -> String {
    let qualified = match &name.prefix {
        Some(prefix) => format!("{prefix}:{}", name.local),
        None => name.local.to_string(),
    };
    if name.ns == ns!(html) {
        qualified.to_ascii_uppercase()
    } else {
        qualified
    }
}

/// Whether `name` is a valid XML `Name`-ish token for `createElement`. This
/// follows the DOM "valid element local name" rules loosely: no whitespace,
/// no `>`/`/`, must not be empty, and must start with an ASCII letter or a
/// non-ASCII character.
pub fn is_valid_element_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_' || first == ':' || !first.is_ascii()) {
        return false;
    }
    !name.chars().any(|c| c.is_ascii_whitespace() || matches!(c, '>' | '/' | '<' | '\0'))
}

/// Whether `name` can be used as an attribute name with `setAttribute`: not
/// empty and free of whitespace, NUL, `/`, `=` and `>`.
pub fn is_valid_attribute_name(name: &str) -> bool {
    !name.is_empty()
        && !name
            .chars()
            .any(|c| c.is_ascii_whitespace() || matches!(c, '\0' | '/' | '=' | '>'))
}
