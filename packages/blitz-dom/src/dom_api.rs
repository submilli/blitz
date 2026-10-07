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
    Syntax,
    NoModificationAllowed,
    QuotaExceeded,
    IndexSize,
}

impl DomError {
    /// The `DOMException` name.
    pub fn name(self) -> &'static str {
        match self {
            Self::HierarchyRequest => "HierarchyRequestError",
            Self::NotFound => "NotFoundError",
            Self::NotSupported => "NotSupportedError",
            Self::InvalidCharacter => "InvalidCharacterError",
            Self::Syntax => "SyntaxError",
            Self::NoModificationAllowed => "NoModificationAllowedError",
            Self::QuotaExceeded => "QuotaExceededError",
            Self::IndexSize => "IndexSizeError",
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
    /// Evaluate a media query list (`matchMedia`) against the document's
    /// current device (viewport, media type).
    pub fn evaluate_media_query(&mut self, query: &str) -> bool {
        if !crate::css_limits::allowed(query) {
            return false;
        }
        use style::media_queries::MediaList;
        use style::parser::ParserContext;
        use style::stylesheets::{CssRuleType, Origin};
        use style_traits::ParsingMode;
        let url_data = self.style_base_url(self.root_node_id);
        let mut input = cssparser::ParserInput::new(query);
        let mut parser = cssparser::Parser::new(&mut input);
        let mut context = ParserContext::new(
            Origin::Author,
            &url_data,
            Some(CssRuleType::Media),
            ParsingMode::DEFAULT,
            selectors::matching::QuirksMode::NoQuirks,
            Default::default(),
            None,
            None,
            Default::default(),
        );
        let list = MediaList::parse(&mut context, &mut parser);
        let device = self.stylist_device();
        list.evaluate(
            device,
            selectors::matching::QuirksMode::NoQuirks,
            &mut style::stylesheets::CustomMediaEvaluator::none(),
        )
    }

    /// Serialize a media query list as `MediaQueryList.media` does.
    pub fn serialize_media_query(&self, query: &str) -> String {
        if !crate::css_limits::allowed(query) {
            return "not all".into();
        }
        use style::media_queries::MediaList;
        use style::parser::ParserContext;
        use style::stylesheets::{CssRuleType, Origin};
        use style_traits::{ParsingMode, ToCss};
        let url_data = self.style_base_url(self.root_node_id);
        let mut input = cssparser::ParserInput::new(query);
        let mut parser = cssparser::Parser::new(&mut input);
        let mut context = ParserContext::new(
            Origin::Author,
            &url_data,
            Some(CssRuleType::Media),
            ParsingMode::DEFAULT,
            selectors::matching::QuirksMode::NoQuirks,
            Default::default(),
            None,
            None,
            Default::default(),
        );
        MediaList::parse(&mut context, &mut parser).to_css_string()
    }

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
            NodeData::Element(el) | NodeData::AnonymousBlock(el) => {
                html_uppercased_qualified_name(&el.name)
            }
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
        self.character_data_dom(id)
            .map(crate::DomString::as_str_lossy)
    }

    /// Original character data, including lone UTF-16 surrogates.
    pub fn character_data_dom(&self, id: NodeId) -> Option<&crate::DomString> {
        match &self.nodes[id].data {
            NodeData::Text(t) => Some(&t.content),
            NodeData::Comment { contents } => Some(contents),
            NodeData::ProcessingInstruction { contents, .. } => Some(contents),
            _ => None,
        }
    }

    /// `Node.textContent` getter. `None` (JS `null`) for documents and doctypes.
    pub fn text_content_of(&self, id: NodeId) -> Option<String> {
        self.text_content_dom(id)
            .map(|s| s.as_str_lossy().to_owned())
    }

    /// DOMString textContent without scalar-value conversion.
    pub fn text_content_dom(&self, id: NodeId) -> Option<crate::DomString> {
        match &self.nodes[id].data {
            NodeData::Document(_) | NodeData::Doctype { .. } => None,
            NodeData::Text(_)
            | NodeData::Comment { .. }
            | NodeData::ProcessingInstruction { .. } => self.character_data_dom(id).cloned(),
            _ => {
                let mut out = crate::DomString::new();
                self.collect_descendant_text(id, &mut out);
                Some(out)
            }
        }
    }

    /// Append the data of `id`'s descendant Text nodes, in tree order,
    /// walking with an explicit stack so a deep tree cannot overflow the
    /// native stack.
    fn collect_descendant_text(&self, id: NodeId, out: &mut crate::DomString) {
        let mut stack: Vec<NodeId> = self.nodes[id].children.iter().rev().copied().collect();
        while let Some(node) = stack.pop() {
            match &self.nodes[node].data {
                NodeData::Text(t) => out.push_dom(&t.content),
                NodeData::Element(_) | NodeData::AnonymousBlock(_) => {
                    stack.extend(self.nodes[node].children.iter().rev().copied());
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

    /// DOM host-including ancestry prevents insertion cycles across shadow roots.
    pub fn is_host_including_inclusive_ancestor(&self, ancestor: NodeId, node: NodeId) -> bool {
        let mut current = Some(node);
        while let Some(id) = current {
            if id == ancestor {
                return true;
            }
            current = self.nodes[id].parent.or_else(|| self.shadow_host_of(id));
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
    /// Connected through shadow roots too: a shadow tree is connected when
    /// its host is.
    pub fn is_connected(&self, node: NodeId) -> bool {
        let mut root = self.tree_root(node);
        while let Some(host) = self.shadow_host_of(root) {
            root = self.tree_root(host);
        }
        self.node_type(root) == node_type::DOCUMENT
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
        if self.is_host_including_inclusive_ancestor(node, parent) {
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
        let others = || {
            children
                .iter()
                .copied()
                .filter(move |&c| Some(c) != replacing)
        };
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
                let elements = frag_children
                    .iter()
                    .filter(|&&c| self.is_element(c))
                    .count();
                if frag_children
                    .iter()
                    .any(|&c| self.node_type(c) == node_type::TEXT)
                {
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

    /// HTML semantic attributes always belong to the empty namespace. Generic
    /// DOM qualified-name lookup remains available through `attribute_by_name`.
    pub fn null_attribute(&self, id: NodeId, name: &str) -> Option<&Attribute> {
        self.get_node(id)?
            .element_data()?
            .attrs()
            .iter()
            .find(|attr| attr.name.ns == ns!() && attr.name.local.as_ref() == name)
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
pub(crate) fn is_text_control(el: &crate::node::ElementData) -> bool {
    match &*el.name.local {
        "textarea" => true,
        "input" => !matches!(
            el.attr(markup5ever::local_name!("type"))
                .map(|t| t.to_ascii_lowercase())
                .as_deref(),
            Some(
                "checkbox" | "radio" | "file" | "button" | "submit" | "reset" | "image" | "hidden"
            )
        ),
        _ => false,
    }
}

pub(crate) fn is_checkable(el: &crate::node::ElementData) -> bool {
    &*el.name.local == "input"
        && matches!(
            el.attr(markup5ever::local_name!("type"))
                .map(|t| t.to_ascii_lowercase())
                .as_deref(),
            Some("checkbox" | "radio")
        )
}

impl BaseDocument {
    /// The `value` IDL attribute of `input`, `textarea`, `select`, `option`
    /// and `button`: the current value, falling back to the defaults.
    pub fn form_value(&self, id: NodeId) -> Option<String> {
        self.form_value_dom(id).map(|v| v.as_str_lossy().to_owned())
    }

    /// Lossless current value. Rendering must never replace these code units.
    pub fn form_value_dom(&self, id: NodeId) -> Option<crate::DomString> {
        let el = self.nodes[id].element_data()?;
        if el.name.ns != ns!(html) {
            return None;
        }
        if self.is_file_input(id) {
            return Some(
                el.selected_file_name()
                    .map(|name| format!("C:\\fakepath\\{name}"))
                    .unwrap_or_default()
                    .into(),
            );
        }
        match &*el.name.local {
            "input" | "textarea" => {
                let value = if let Some(value) =
                    el.form_state.value.as_ref().filter(|_| is_text_control(el))
                {
                    value.clone()
                } else if &*el.name.local == "textarea" {
                    self.nodes[id].child_text_content()
                } else {
                    el.attr_dom(markup5ever::local_name!("value"))
                        .cloned()
                        .unwrap_or_else(|| if is_checkable(el) { "on" } else { "" }.into())
                };
                Some(crate::validation::text::api_text(el, &value))
            }
            "option" => Some(match el.attr_dom(markup5ever::local_name!("value")) {
                Some(v) => v.clone(),
                None => self.option_text(id),
            }),
            "select" => self.form_value_dom(self.selected_option(id)?),
            "button" | "data" | "li" | "param" | "output" => Some(
                el.attr_dom(markup5ever::local_name!("value"))
                    .cloned()
                    .unwrap_or_default(),
            ),
            _ => None,
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
        el.form_state
            .checked
            .unwrap_or_else(|| el.has_attr(markup5ever::local_name!("checked")))
    }
}

impl DocumentMutator<'_> {
    /// `input.value = ...` / `textarea.value = ...`: sets the current value
    /// and its dirty flag; the `value` attribute is untouched.
    pub fn set_form_value(&mut self, id: NodeId, value: &str) {
        self.set_form_value_dom(id, &value.into());
    }

    /// Assign the current DOMString without a scalar round trip.
    pub fn set_form_value_dom(&mut self, id: NodeId, value: &crate::DomString) {
        let previous = self.doc.form_value_dom(id);
        let previous_selection = self.doc.control_selection(id);
        if self.doc.is_file_input(id) {
            if value.is_empty() {
                if let Some(e) = self.doc.nodes[id].element_data_mut() {
                    e.clear_file_selection();
                }
            }
            return;
        }
        let is_text = self.doc.nodes[id]
            .element_data()
            .is_some_and(is_text_control);
        if !is_text {
            // Other controls reflect `value` to the attribute.
            self.set_attribute(id, QualName::new(None, ns!(), "value".into()), value);
            return;
        }
        let doc = &mut *self.doc;
        let font_ctx = &doc.font_ctx;
        let layout_ctx = &mut doc.layout_ctx;
        let Some(el) = doc.nodes[id].element_data_mut() else {
            return;
        };
        let value = crate::validation::text::sanitize_text(el, value);
        if previous.as_ref() != Some(&value) {
            el.form_state.selection =
                crate::form_selection::ControlSelection::caret(value.to_utf16().len());
        }
        el.form_state.calendar = None;
        el.form_state.value = Some(value.clone());
        el.form_state.value_dirty = true;
        el.form_state.last_change_by_user = false;
        if let Some(input) = el.text_input_data_mut() {
            input.set_text(
                &mut font_ctx.lock().unwrap(),
                layout_ctx,
                value.as_str_lossy(),
            );
        }
        doc.project_control_selection(id);
        if doc.control_selection(id) != previous_selection {
            doc.notify_control_selection(id, doc.focus_node_id == Some(id));
        }
    }

    /// `input.checked = ...`: sets the current checkedness and its dirty
    /// flag. Checking a radio button unchecks the others in its group.
    pub fn set_checkedness(&mut self, id: NodeId, checked: bool) {
        if checked {
            let others = self.doc.radio_group_members(id);
            for other in others {
                if other == id {
                    continue;
                }
                self.write_checkedness(other, false);
            }
        }
        self.write_checkedness(id, checked);
    }

    /// Content attributes update pristine checkedness, preserving group dirtiness.
    pub(crate) fn update_default_checkedness(&mut self, id: NodeId) {
        if !self.doc.is_checkable_input(id)
            || self.doc.nodes[id]
                .element_data()
                .is_some_and(|el| el.form_state.checked_dirty)
        {
            return;
        }
        let checked = self.doc.null_attribute(id, "checked").is_some();
        if checked {
            for peer in self.doc.radio_group_members(id) {
                if peer == id {
                    continue;
                }
                let dirty = self.doc.nodes[peer]
                    .element_data()
                    .is_some_and(|el| el.form_state.checked_dirty);
                self.write_checkedness(peer, false);
                if let Some(el) = self.doc.nodes[peer].element_data_mut() {
                    el.form_state.checked_dirty = dirty;
                }
            }
        }
        self.write_checkedness(id, checked);
        if let Some(el) = self.doc.nodes[id].element_data_mut() {
            el.form_state.checked_dirty = false;
        }
    }

    pub(crate) fn write_checkedness(&mut self, id: NodeId, checked: bool) {
        self.doc
            .snapshot_node_and(id, style_dom::ElementState::CHECKED, |node| {
                if let Some(el) = node.element_data_mut() {
                    el.form_state.checked = Some(checked);
                    el.form_state.checked_dirty = true;
                    if el.checkbox_input_checked().is_some() {
                        el.set_checkbox_input_checked(checked);
                    } else {
                        el.element_state
                            .set(style_dom::ElementState::CHECKED, checked);
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
        value: impl Into<crate::DomString>,
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
        if doc.is_host_including_inclusive_ancestor(node, parent) {
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
        // One mutation record for the whole replacement, as the spec's
        // "replace all" queues (its removals and insertion are silent).
        self.with_one_child_list_record(parent, |m| {
            m.detach_children(parent);
            if let Some(node) = node {
                m.insert(node, parent, None);
            }
        });
    }

    /// Run `f`, which replaces `parent`'s children, recording it as a single
    /// childList record (removed: the old children, added: the new ones).
    pub(crate) fn with_one_child_list_record(&mut self, parent: NodeId, f: impl FnOnce(&mut Self)) {
        let Some(log) = self.doc.mutation_log.take() else {
            return f(self);
        };
        let removed: Vec<NodeId> = self.doc.nodes[parent].children.to_vec();
        f(self);
        let added: Vec<NodeId> = self.doc.nodes[parent].children.to_vec();
        self.doc.mutation_log = Some(log);
        if !removed.is_empty() || !added.is_empty() {
            self.doc
                .record_mutation(crate::mutations::MutationRecord::ChildList {
                    target: parent,
                    added,
                    removed,
                    previous_sibling: None,
                    next_sibling: None,
                });
        }
    }

    /// `Node.textContent` setter.
    pub fn set_text_content(&mut self, id: NodeId, value: impl Into<crate::DomString>) {
        let value = value.into();
        match self.doc.node_type(id) {
            node_type::ELEMENT | node_type::DOCUMENT_FRAGMENT => {
                let text = (!value.is_empty()).then(|| self.create_text_node(&value));
                self.replace_all(id, text);
            }
            node_type::TEXT | node_type::COMMENT | node_type::PROCESSING_INSTRUCTION => {
                self.set_character_data(id, value)
            }
            _ => {}
        }
    }

    /// Replace the data of a Text, Comment or ProcessingInstruction node.
    pub fn set_character_data(&mut self, id: NodeId, value: impl Into<crate::DomString>) {
        let value = value.into();
        if let Some(old) = self.doc.character_data_dom(id) {
            self.doc
                .notify_range_mutation(crate::ranges::RangeMutation::ReplaceData {
                    node: id,
                    offset: 0,
                    removed: old.to_utf16().len(),
                    added: value.to_utf16().len(),
                });
        }
        self.set_character_data_raw(id, value);
    }

    /// Store character data after applying the operation's live-range steps.
    pub(crate) fn set_character_data_raw(&mut self, id: NodeId, value: crate::DomString) {
        if matches!(self.doc.nodes[id].data, NodeData::Text(_)) {
            // Text changes affect layout, which set_node_text takes care of.
            self.set_node_text_raw(id, value);
            return;
        }
        let old = match &self.doc.nodes[id].data {
            NodeData::Comment { contents } | NodeData::ProcessingInstruction { contents, .. } => {
                contents.clone()
            }
            _ => return,
        };
        self.doc
            .record_mutation(crate::mutations::MutationRecord::CharacterData {
                target: id,
                old_value: old,
            });
        if let NodeData::Comment { contents } | NodeData::ProcessingInstruction { contents, .. } =
            &mut self.doc.nodes[id].data
        {
            *contents = value;
        }
        if let Some(parent) = self.doc.nodes[id].parent {
            self.textarea_children_changed(parent);
        }
    }

    /// Replace the children of `id` with the result of parsing `html` as a
    /// fragment in its context (the `innerHTML` setter). Old children are
    /// detached, not dropped. For `<template>`, replaces the template contents.
    pub fn set_inner_html_detaching(&mut self, id: NodeId, html: &str) {
        let _ = self.try_set_inner_html_detaching(id, html);
    }

    /// Parse before replacing children. On quota failure the old subtree and
    /// its identities remain intact, and the temporary parse tree is released.
    pub fn try_set_inner_html_detaching(
        &mut self,
        id: NodeId,
        html: &str,
    ) -> Result<(), crate::NodeBudgetExceeded> {
        self.try_replace_html(id, html, false)
    }

    /// HTML unsafe parsing enables parser-owned declarative roots. Admission
    /// remains transactional and old children remain usable after replacement.
    pub fn try_set_html_unsafe(
        &mut self,
        id: NodeId,
        html: &str,
    ) -> Result<(), crate::NodeBudgetExceeded> {
        self.try_replace_html(id, html, true)
    }

    fn try_replace_html(
        &mut self,
        id: NodeId,
        html: &str,
        declarative: bool,
    ) -> Result<(), crate::NodeBudgetExceeded> {
        let context = self.doc.shadow_host_of(id).unwrap_or(id);
        let Some(element) = self.doc.nodes[context].element_data() else {
            return Ok(());
        };
        let name = element.name.clone();
        let is_template =
            name.ns == ns!(html) && name.local == markup5ever::local_name!("template");
        if html.is_empty() {
            let target = if is_template {
                element.template_contents
            } else {
                Some(id)
            };
            if let Some(target) = target {
                self.replace_all(target, None);
            }
            return Ok(());
        }
        let scratch = self.try_create_element(name, Vec::new())?;
        self.set_node_document(scratch, self.doc.node_document(id));
        let result = self.parse_temporary_fragment_mode(scratch, html, declarative);
        if let Err(error) = result {
            self.remove_and_drop_node(scratch);
            return Err(error);
        }
        let target = if is_template {
            match self.try_ensure_template_contents(id) {
                Ok(contents) => contents,
                Err(error) => {
                    self.remove_and_drop_node(scratch);
                    return Err(error);
                }
            }
        } else {
            id
        };
        let parsed = self.doc.nodes[scratch].children.to_vec();
        self.with_one_child_list_record(target, |m| {
            m.replace_all(target, None);
            m.append_children(target, &parsed);
        });
        self.remove_and_drop_node(scratch);
        Ok(())
    }

    /// `Element.insertAdjacentHTML(position, html)`: parse `html` as a
    /// fragment in the right context element and insert the result.
    pub fn insert_adjacent_html(
        &mut self,
        id: NodeId,
        position: &str,
        html: &str,
    ) -> Result<(), DomError> {
        let parent = self.doc.nodes[id].parent;
        let (context, target_parent, anchor) = match position.to_ascii_lowercase().as_str() {
            "beforebegin" | "afterend" => {
                let parent = parent.ok_or(DomError::NoModificationAllowed)?;
                if self.doc.node_type(parent) == node_type::DOCUMENT {
                    return Err(DomError::NoModificationAllowed);
                }
                let anchor = if position.eq_ignore_ascii_case("beforebegin") {
                    Some(id)
                } else {
                    self.next_sibling_id(id)
                };
                (parent, parent, anchor)
            }
            "afterbegin" => (id, id, self.doc.nodes[id].children.first().copied()),
            "beforeend" => (id, id, None),
            _ => return Err(DomError::Syntax),
        };
        // Parse into a detached element named like the context element.
        // A context that is not an element, or is the `html` element, parses
        // as a `body` would (HTML § 8.5, insertAdjacentHTML step 2).
        let context_name = self.doc.nodes[context]
            .element_data()
            .map(|el| el.name.clone())
            .filter(|name| {
                !(name.ns == ns!(html) && name.local == markup5ever::local_name!("html"))
            })
            .unwrap_or_else(|| QualName::new(None, ns!(html), markup5ever::local_name!("body")));
        let scratch = self
            .try_create_element(context_name, Vec::new())
            .map_err(|_| DomError::QuotaExceeded)?;
        if self.parse_temporary_fragment(scratch, html).is_err() {
            self.remove_and_drop_node(scratch);
            return Err(DomError::QuotaExceeded);
        }
        let parsed = self.doc.nodes[scratch].children.to_vec();
        // Moving out of an unobservable parser context must not expose its
        // soon-to-be-freed ID through a removal record.
        self.detach_children(scratch);
        if !parsed.is_empty() {
            match anchor {
                Some(anchor) => self.insert_nodes_before(anchor, &parsed),
                None => self.append_children(target_parent, &parsed),
            }
        }
        self.remove_and_drop_node(scratch);
        Ok(())
    }

    /// Scratch construction is unobservable. Keep only the eventual mutation
    /// of the real target, including when fragment parsing rolls back.
    fn parse_temporary_fragment(
        &mut self,
        scratch: NodeId,
        html: &str,
    ) -> Result<(), crate::NodeBudgetExceeded> {
        self.parse_temporary_fragment_mode(scratch, html, false)
    }

    fn parse_temporary_fragment_mode(
        &mut self,
        scratch: NodeId,
        html: &str,
        declarative: bool,
    ) -> Result<(), crate::NodeBudgetExceeded> {
        let log = self.doc.mutation_log.take();
        let selection_scratch = self.selection_scratch.replace(scratch);
        let provider = self.doc.html_parser_provider.clone();
        let result = if declarative {
            provider.try_parse_html_unsafe(self, scratch, html)
        } else {
            provider.try_parse_inner_html(self, scratch, html)
        };
        self.doc.mutation_log = log;
        self.selection_scratch = selection_scratch;
        result
    }

    /// `Node.cloneNode(deep)`: the clone is detached. A deep clone copies
    /// the descendants (and template contents) with an explicit stack, so
    /// a deep tree cannot overflow the native stack.
    pub fn clone_node(&mut self, id: NodeId, deep: bool) -> NodeId {
        // Cloning the document itself is not supported.
        if matches!(self.doc.nodes[id].data, NodeData::Document(_)) {
            return id;
        }
        let copy = self.clone_one(id);
        self.set_node_document(copy, self.doc.node_document(id));
        let mut pending = vec![(id, copy, deep)];
        while let Some((source, copy, deep)) = pending.pop() {
            if let Some(root) = self.doc.shadow_root_of(source)
                && let Some(data) = self.doc.nodes[root].shadow_root_data.as_deref().cloned()
                && data.clonable
            {
                let clone_root = self.create_document_fragment();
                let mut metadata = data;
                metadata.host = copy;
                self.doc.nodes[clone_root].shadow_root_data = Some(Box::new(metadata));
                self.doc.nodes[copy]
                    .element_data_mut()
                    .expect("cloned shadow host")
                    .shadow_root = Some(clone_root);
                self.doc.shadow_hosts.insert(copy);
                self.set_node_document(clone_root, self.doc.node_document(copy));
                self.clone_children(root, clone_root, &mut pending);
            }
            if !deep {
                continue;
            }
            let contents = self.doc.nodes[source]
                .element_data()
                .and_then(|el| el.template_contents);
            if let Some(contents) = contents {
                let copy_contents = self.template_contents(copy);
                self.clone_children(contents, copy_contents, &mut pending);
            }
            self.clone_children(source, copy, &mut pending);
        }
        copy
    }

    /// Append copies of `source`'s children to `copy`, and queue each pair
    /// so its own children are copied next.
    fn clone_children(
        &mut self,
        source: NodeId,
        copy: NodeId,
        pending: &mut Vec<(NodeId, NodeId, bool)>,
    ) {
        let children: Vec<NodeId> = self.doc.nodes[source].children.to_vec();
        for child in children {
            let child_copy = self.clone_one(child);
            self.append_children(copy, &[child_copy]);
            pending.push((child, child_copy, true));
        }
    }

    /// A detached copy of `id` alone, without children.
    fn clone_one(&mut self, id: NodeId) -> NodeId {
        match self.doc.nodes[id].data.clone() {
            NodeData::Element(el) | NodeData::AnonymousBlock(el) => {
                let attrs: Vec<Attribute> = el.attrs().to_vec();
                let copy = self.create_element(el.name.clone(), attrs);
                self.doc.nodes[copy]
                    .element_data_mut()
                    .expect("cloned element")
                    .custom_element_is = el.custom_element_is.clone();
                let custom_state = if el.custom_element_is.is_some() {
                    crate::custom_elements::CustomElementState::Undefined
                } else {
                    crate::custom_elements::CustomElementState::initial(&el.name)
                };
                self.doc.set_custom_element_state(copy, custom_state);
                if el.name.ns == ns!(html) && matches!(&*el.name.local, "input" | "textarea") {
                    let original = self.doc.nodes[id].element_data().expect("source element");
                    let state = original.form_state.clone();
                    let value = state.value.clone().or_else(|| self.doc.form_value_dom(id));
                    let target = self.doc.nodes[copy]
                        .element_data_mut()
                        .expect("cloned element");
                    target.form_state.value = is_text_control(&el).then_some(value).flatten();
                    target.form_state.value_dirty = state.value_dirty;
                    target.form_state.last_change_by_user = state.last_change_by_user;
                    if &*el.name.local == "input" {
                        target.form_state.files = state.files;
                        target.form_state.checked = state.checked;
                        target.form_state.checked_dirty = state.checked_dirty;
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
            // Unreachable: `clone_node` returns early for a document, and a
            // document is never a child.
            NodeData::Document(_) => self.create_document_node(),
        }
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
    !name
        .chars()
        .any(|c| c.is_ascii_whitespace() || matches!(c, '>' | '/' | '<' | '\0'))
}

/// Whether `name` can be used as an attribute name with `setAttribute`: not
/// empty and free of whitespace, NUL, `/`, `=` and `>`.
pub fn is_valid_attribute_name(name: &str) -> bool {
    !name.is_empty()
        && !name
            .chars()
            .any(|c| c.is_ascii_whitespace() || matches!(c, '\0' | '/' | '=' | '>'))
}
