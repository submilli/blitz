mod budget;
#[cfg(test)]
mod tests;
use crate::{BaseDocument, ElementData, Node as BlitzDomNode, local_name};
use accesskit::{
    Invalid, Node as AccessKitNode, NodeId, Rect, Role, Toggled, TreeId, TreeInfo, TreeUpdate,
};
use budget::Builder;
pub use budget::{AccessibilityBudget, AccessibilityLimit};
use style::properties::longhands::visibility;

impl BaseDocument {
    /// Bounded default for legacy consumers. Use the fallible form when a
    /// caller must distinguish admission failure from an empty document.
    pub fn build_accessibility_tree(&self) -> TreeUpdate {
        self.try_build_accessibility_tree(&AccessibilityBudget::new(1_000_000, 4 * 1024 * 1024))
            .unwrap_or_else(|_| TreeUpdate {
                tree_id: TreeId::ROOT,
                nodes: Vec::new(),
                tree: None,
                focus: NodeId(u64::MAX),
            })
    }

    pub fn try_build_accessibility_tree(
        &self,
        budget: &AccessibilityBudget,
    ) -> Result<TreeUpdate, AccessibilityLimit> {
        let tree = Builder {
            document: self,
            budget,
        }
        .build();
        if budget.failed() {
            Err(AccessibilityLimit)
        } else {
            Ok(tree)
        }
    }
    /// Labeling sources only, in ARIA precedence order. Unlike a full accessible
    /// name, this never falls back to content, title, or placeholder.
    pub fn try_accessibility_labels(
        &self,
        id: crate::NodeId,
        budget: &AccessibilityBudget,
    ) -> Result<Vec<String>, AccessibilityLimit> {
        let builder = Builder {
            document: self,
            budget,
        };
        let names = self
            .get_node(id)
            .filter(|node| builder.admit_node(node))
            .map(|node| builder.label_names(node))
            .unwrap_or_default();
        if budget.failed() {
            Err(AccessibilityLimit)
        } else {
            Ok(names)
        }
    }
}

impl Builder<'_> {
    fn build(&self) -> TreeUpdate {
        let mut nodes = std::collections::HashMap::new();
        let mut window = AccessKitNode::new(Role::Window);
        let mut hidden_nodes = std::collections::HashSet::new();

        // The flat tree: shadow content where it renders.
        self.visit_flat(|node_id, node| {
            if !self.admit_node(node) {
                return;
            }
            // A collapsed `<select>`'s options are not rendered, but they are
            // its popup list: keep them.
            if (node.is_hidden_from_accessibility_tree() && !node.is_select_option())
                || node
                    .flat_parent_id()
                    .map(|p| hidden_nodes.contains(&p))
                    .unwrap_or(false)
            {
                hidden_nodes.insert(node_id);
                return;
            }
            let parent = node
                .flat_parent_id()
                .and_then(|parent_id| nodes.get_mut(&parent_id))
                .map(|(_, parent)| parent)
                .unwrap_or(&mut window);
            let (id, builder) = self.build_accessibility_node(node, parent);

            nodes.insert(node_id, (id, builder));
        });

        let mut nodes: Vec<_> = nodes
            .into_iter()
            .map(|(_, (id, node))| (id, node))
            .collect();
        nodes.push((NodeId(u64::MAX), window));

        let tree = TreeInfo::new(NodeId(u64::MAX));
        TreeUpdate {
            tree_id: TreeId::ROOT,
            nodes,
            tree: Some(tree),
            focus: NodeId(self.focus_node_id.map(|id| id.as_u64()).unwrap_or(u64::MAX)),
        }
    }

    fn build_accessibility_node(
        &self,
        node: &BlitzDomNode,
        parent: &mut AccessKitNode,
    ) -> (NodeId, AccessKitNode) {
        let id = NodeId(node.id.as_u64());

        let mut builder = AccessKitNode::default();
        if node.parent.is_none() {
            builder.set_role(Role::Window)
        } else if let Some(element_data) = node.element_data() {
            let name = element_data.name.local.to_string();
            let role_attr = element_data.attr(local_name!("role"));

            // TODO: The roles of elements with strong native semantics cannot be overridden; see
            // https://www.w3.org/TR/wai-aria-1.2/#host_general_conflict.
            let role = role_attr
                .and_then(role_from_name)
                .or_else(|| role_from_element_data(element_data))
                .unwrap_or(Role::Unknown);

            builder.set_role(role);
            builder.set_html_tag(name);

            // https://www.w3.org/TR/wai-aria-1.2/#tree_exclusion
            if element_data.attr(local_name!("aria-hidden")) == Some("true") {
                builder.set_hidden();
            }
            self.set_element_properties(node, element_data, role, &mut builder);
        } else if node.is_text_node() {
            builder.set_role(Role::TextRun);
            builder.set_value(self.raw_text(node));
            parent.push_labelled_by(id)
        }

        parent.push_child(id);

        (id, builder)
    }
}

/// Roles whose accessible name comes from their content when nothing else
/// names them (accname 1.2, "name from content").
fn name_from_content(role: Role) -> bool {
    matches!(
        role,
        Role::Button
            | Role::Link
            | Role::Heading
            | Role::Cell
            | Role::ColumnHeader
            | Role::RowHeader
            | Role::ListBoxOption
            | Role::MenuItem
            | Role::MenuItemCheckBox
            | Role::MenuItemRadio
            | Role::Tab
            | Role::TreeItem
            | Role::CheckBox
            | Role::RadioButton
            | Role::Switch
            | Role::Tooltip
            | Role::DisclosureTriangle
            | Role::Caption
            | Role::Label
    )
}

/// Collapse runs of whitespace and trim.
fn normalize(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

impl Builder<'_> {
    /// Name, description, states, value and bounds of an element's node.
    fn set_element_properties(
        &self,
        node: &BlitzDomNode,
        element: &ElementData,
        role: Role,
        builder: &mut AccessKitNode,
    ) {
        let attr = |name: &str| self.attribute(element, name);
        let tag = &*element.name.local;

        if let Some(name) = self.accessible_name(node, element, role) {
            builder.set_label(name);
        }
        let description = attr("aria-describedby")
            .map(|ids| self.text_of_ids(&ids))
            .filter(|d| !d.is_empty());
        if let Some(description) = description {
            builder.set_description(description);
        }

        // States.
        let disabled = node.is_disabled() || attr("aria-disabled").as_deref() == Some("true");
        if disabled {
            builder.set_disabled();
        }
        if element.attr(local_name!("required")).is_some()
            || attr("aria-required").as_deref() == Some("true")
        {
            builder.set_required();
        }
        if element.attr(local_name!("readonly")).is_some()
            || attr("aria-readonly").as_deref() == Some("true")
        {
            builder.set_read_only();
        }
        if attr("aria-invalid").is_some_and(|v| v != "false") {
            builder.set_invalid(Invalid::True);
        }
        match role {
            Role::CheckBox | Role::RadioButton | Role::Switch if tag == "input" => {
                builder.set_toggled(Toggled::from(self.checkedness(node.id)));
            }
            _ => match attr("aria-checked")
                .or_else(|| attr("aria-pressed"))
                .as_deref()
            {
                Some("true") => builder.set_toggled(Toggled::True),
                Some("mixed") => builder.set_toggled(Toggled::Mixed),
                Some("false") => builder.set_toggled(Toggled::False),
                _ => {}
            },
        }
        if tag == "option" {
            let selected = node
                .ancestors_select()
                .and_then(|select| self.selected_option(select))
                .map_or(element.attr(local_name!("selected")).is_some(), |s| {
                    s == node.id
                });
            builder.set_selected(selected);
        } else if let Some(selected) = attr("aria-selected") {
            builder.set_selected(selected == "true");
        }
        if let Some(expanded) = attr("aria-expanded") {
            builder.set_expanded(expanded == "true");
        } else if tag == "summary" {
            let open = node
                .parent
                .and_then(|p| self.nodes[p].element_data())
                .is_some_and(|d| {
                    &*d.name.local == "details" && d.attr(local_name!("open")).is_some()
                });
            builder.set_expanded(open);
        }

        // Values.
        match tag {
            "input" | "textarea"
                if !matches!(role, Role::CheckBox | Role::RadioButton | Role::Button) =>
            {
                if let Some(value) = self.form_value(node) {
                    builder.set_value(value);
                }
                if let Some(placeholder) = attr("placeholder") {
                    builder.set_placeholder(placeholder);
                }
            }
            "select" => {
                if let Some(option) = self.selected_option(node.id) {
                    builder.set_value(normalize(&self.raw_text(&self.nodes[option])));
                }
            }
            "progress" | "meter" => {
                if let Some(value) = attr("value").and_then(|v| v.parse::<f64>().ok()) {
                    builder.set_numeric_value(value);
                }
            }
            _ => {}
        }

        // Heading level, link target.
        let heading_level = match tag {
            "h1" => Some(1),
            "h2" => Some(2),
            "h3" => Some(3),
            "h4" => Some(4),
            "h5" => Some(5),
            "h6" => Some(6),
            _ => None,
        };
        let level = attr("aria-level")
            .and_then(|l| l.parse::<usize>().ok())
            .or(heading_level);
        if let Some(level) = level {
            builder.set_level(level);
        }
        if role == Role::Link
            && let Some(url) = attr("href").and_then(|href| {
                let Some(base) =
                    self.document_base_url_with_limit(node.id, self.budget.remaining().1)
                else {
                    self.budget.refuse();
                    return None;
                };
                if !self
                    .budget
                    .text(base.as_str().len().saturating_add(href.len()))
                {
                    return None;
                }
                base.join(&href).ok()
            })
        {
            builder.set_url(url.to_string());
        }

        // Bounds, from layout, in CSS pixels relative to the viewport.
        if let Some(rect) = self.get_client_bounding_rect(node.id) {
            builder.set_bounds(Rect {
                x0: rect.x,
                y0: rect.y,
                x1: rect.x + rect.width,
                y1: rect.y + rect.height,
            });
        }
    }

    /// The accessible name (a practical subset of accname 1.2): labelled-by,
    /// `aria-label`, the host language's labels, content, then `title`.
    fn accessible_name(
        &self,
        node: &BlitzDomNode,
        element: &ElementData,
        role: Role,
    ) -> Option<String> {
        let attr = |name: &str| self.attribute(element, name);
        let non_empty = |s: String| Some(normalize(&s)).filter(|s| !s.is_empty());
        if let Some(name) = attr("aria-labelledby")
            .map(|ids| self.text_of_ids(&ids))
            .and_then(non_empty)
        {
            return Some(name);
        }
        if let Some(name) = attr("aria-label").and_then(non_empty) {
            return Some(name);
        }
        let tag = &*element.name.local;
        let native = match tag {
            "img" | "area" => attr("alt"),
            "input" => match attr("type").as_deref() {
                Some("image") => attr("alt"),
                Some("submit") => attr("value").or(Some("Submit".into())),
                Some("reset") => attr("value").or(Some("Reset".into())),
                Some("button") => attr("value"),
                _ => None,
            }
            .or_else(|| self.label_text(node)),
            "select" | "textarea" | "meter" | "progress" | "output" => self.label_text(node),
            // Options are not rendered while their select is collapsed; their
            // text is still their name.
            "option" => attr("label").or_else(|| Some(self.raw_text(node))),
            "fieldset" => self.first_child_text(node, "legend"),
            "figure" => self.first_child_text(node, "figcaption"),
            "table" => self.first_child_text(node, "caption"),
            _ => None,
        };
        if let Some(name) = native.and_then(non_empty) {
            return Some(name);
        }
        if name_from_content(role) {
            let mut text = String::new();
            self.text_alternative(node.id, &mut text);
            if let Some(name) = non_empty(text) {
                return Some(name);
            }
        }
        let fallback = attr("title").or_else(|| {
            if tag == "input" || tag == "textarea" {
                attr("placeholder")
            } else {
                None
            }
        });
        fallback.and_then(non_empty)
    }

    /// Text of the elements with these (space-separated) ids.
    fn text_of_ids(&self, ids: &str) -> String {
        let parts: Vec<String> = ids
            .split_whitespace()
            .filter_map(|id| {
                if !self.budget.work(self.node_count()) {
                    return None;
                }
                self.get_element_by_id(id)
            })
            .map(|id| {
                let mut text = String::new();
                self.text_alternative(id, &mut text);
                normalize(&text)
            })
            .collect();
        parts.join(" ")
    }

    fn label_names(&self, node: &BlitzDomNode) -> Vec<String> {
        let Some(element) = node.element_data() else {
            return Vec::new();
        };
        if let Some(ids) = self.attribute(element, "aria-labelledby") {
            let mut valid = false;
            for id in ids.split_whitespace() {
                if !self.budget.work(self.node_count()) {
                    return Vec::new();
                }
                valid |= self.get_element_by_id(id).is_some();
            }
            if valid {
                return vec![normalize(&self.text_of_ids(&ids))];
            }
        }
        if let Some(label) = self
            .attribute(element, "aria-label")
            .map(|s| normalize(&s))
            .filter(|s| !s.is_empty())
        {
            return vec![label];
        }
        self.native_label_names(node)
    }

    fn native_label_names(&self, node: &BlitzDomNode) -> Vec<String> {
        if !self.is_labelable_control(node.id) {
            return Vec::new();
        }
        if !self.budget.work(self.node_count().saturating_mul(2)) {
            return Vec::new();
        }
        // Label association compares IDs and `for` values throughout the DOM.
        // Admit their source bytes before the engine's association scan, too.
        for id in
            crate::traversal::TreeTraverser::new_with_root(self.document, self.tree_root(node.id))
        {
            if !self.admit_node(&self.nodes[id]) {
                return Vec::new();
            }
        }
        self.custom_element_labels(node.id)
            .into_iter()
            .map(|id| {
                let mut text = String::new();
                self.text_alternative_skipping(id, Some(node.id), &mut text);
                normalize(&text)
            })
            .filter(|text| !text.is_empty())
            .collect()
    }

    /// Native label text for the full accessible-name computation.
    fn label_text(&self, node: &BlitzDomNode) -> Option<String> {
        let names = self.native_label_names(node);
        (!names.is_empty()).then(|| names.join(" "))
    }

    fn first_child_text(&self, node: &BlitzDomNode, tag: &str) -> Option<String> {
        let child = node.children.iter().copied().find(|&c| {
            if !self.budget.work(1) {
                return false;
            }
            self.nodes[c]
                .element_data()
                .is_some_and(|e| &*e.name.local == tag)
        })?;
        let mut text = String::new();
        self.text_alternative(child, &mut text);
        Some(text)
    }

    /// Text a subtree contributes to a name: its text, images' `alt`, and
    /// nothing from hidden parts.
    fn text_alternative(&self, id: crate::NodeId, out: &mut String) {
        self.text_alternative_skipping(id, None, out)
    }

    fn text_alternative_skipping(
        &self,
        id: crate::NodeId,
        skip: Option<crate::NodeId>,
        out: &mut String,
    ) {
        self.text_alternative_at_depth(id, skip, out, 0);
    }

    fn text_alternative_at_depth(
        &self,
        id: crate::NodeId,
        skip: Option<crate::NodeId>,
        out: &mut String,
        depth: usize,
    ) {
        if depth >= 128 {
            self.budget.refuse();
            return;
        }
        if Some(id) == skip {
            return;
        }
        let node = &self.nodes[id];
        if !self.admit_node(node) {
            return;
        }
        if node.is_hidden_from_accessibility_tree() {
            return;
        }
        if node.is_text_node() {
            out.push_str(&self.raw_text(node));
            return;
        }
        if let Some(element) = node.element_data() {
            if element.attr(local_name!("aria-hidden")) == Some("true") {
                return;
            }
            if &*element.name.local == "img" {
                if let Some(alt) = self.attribute(element, "alt") {
                    out.push(' ');
                    out.push_str(&alt);
                    out.push(' ');
                }
                return;
            }
            if let Some(label) = self.attribute(element, "aria-label") {
                out.push(' ');
                out.push_str(&label);
                out.push(' ');
                return;
            }
        }
        if !self.budget.text(out.len()) {
            return;
        }
        let before = out.trim_end().len();
        for &child in node.flat_children() {
            if self.budget.failed() {
                break;
            }
            self.text_alternative_at_depth(child, skip, out, depth + 1);
        }
        // An element that contributed no text contributes its tooltip
        // (accname: a descendant's name falls back to `title`).
        if !self.budget.text(out.len()) {
            return;
        }
        if out.trim_end().len() == before
            && let Some(title) = node.element_data().and_then(|e| self.attribute(e, "title"))
        {
            out.push(' ');
            out.push_str(&title);
        }
        // Block boundaries separate words.
        if self.budget.text(1) {
            out.push(' ');
        }
    }
}

impl BlitzDomNode {
    /// An `<option>` or `<optgroup>` inside a `<select>`.
    fn is_select_option(&self) -> bool {
        let Some(element) = self.element_data() else {
            return false;
        };
        match &*element.name.local {
            "option" => self.ancestors_select().is_some(),
            "optgroup" => self.parent.is_some_and(|p| {
                self.tree()[p]
                    .element_data()
                    .is_some_and(|e| &*e.name.local == "select")
            }),
            _ => false,
        }
    }

    /// For an `<option>`: its `<select>` (parent, or grandparent through an
    /// `<optgroup>`).
    fn ancestors_select(&self) -> Option<crate::NodeId> {
        let parent = self.parent?;
        let tree = self.tree();
        let is = |id: crate::NodeId, tag: &str| {
            tree[id]
                .element_data()
                .is_some_and(|e| &*e.name.local == tag)
        };
        if is(parent, "select") {
            return Some(parent);
        }
        tree[parent]
            .parent
            .filter(|&grand| is(parent, "optgroup") && is(grand, "select"))
    }

    // https://www.w3.org/TR/wai-aria-1.2/#tree_exclusion
    fn is_hidden_from_accessibility_tree(&self) -> bool {
        self.try_stylo_element_data()
            .as_ref()
            .and_then(|s| s.get())
            .is_some_and(|s| {
                s.styles.get_primary().is_some_and(|style| {
                    style.get_box().clone_display().is_none()
                        || style.clone_visibility() == visibility::computed_value::T::Hidden
                })
            })
    }
}

fn role_from_name(name: &str) -> Option<Role> {
    match name {
        "alert" => Some(Role::Alert),
        "alertdialog" => Some(Role::AlertDialog),
        "button" => Some(Role::Button),
        "checkbox" => Some(Role::CheckBox),
        "dialog" => Some(Role::Dialog),
        "gridcell" => Some(Role::GridCell),
        "link" => Some(Role::Link),
        "log" => Some(Role::Log),
        "marquee" => Some(Role::Marquee),
        "menuitem" => Some(Role::MenuItem),
        "menuitemcheckbox" => Some(Role::MenuItemCheckBox),
        "menuitemradio" => Some(Role::MenuItemRadio),
        "option" => Some(Role::ListBoxOption),
        "progressbar" => Some(Role::ProgressIndicator),
        "radio" => Some(Role::RadioButton),
        "scrollbar" => Some(Role::ScrollBar),
        "slider" => Some(Role::Slider),
        "spinbutton" => Some(Role::SpinButton),
        "status" => Some(Role::Status),
        "tab" => Some(Role::Tab),
        "tabpanel" => Some(Role::TabPanel),
        "textbox" => Some(Role::TextInput),
        "timer" => Some(Role::Timer),
        "tooltip" => Some(Role::Tooltip),
        "treeitem" => Some(Role::TreeItem),
        "combobox" => Some(Role::ComboBox),
        "grid" => Some(Role::Grid),
        "listbox" => Some(Role::ListBox),
        "menu" => Some(Role::Menu),
        "menubar" => Some(Role::MenuBar),
        "radiogroup" => Some(Role::RadioGroup),
        "tablist" => Some(Role::TabList),
        "tree" => Some(Role::Tree),
        "treegrid" => Some(Role::TreeGrid),
        "article" => Some(Role::Article),
        "columnheader" => Some(Role::ColumnHeader),
        "definition" => Some(Role::Definition),
        "document" => Some(Role::Document),
        "group" => Some(Role::Group),
        "heading" => Some(Role::Heading),
        "img" => Some(Role::Image),
        "list" => Some(Role::List),
        "listitem" => Some(Role::ListItem),
        "math" => Some(Role::Math),
        "note" => Some(Role::Note),
        "region" => Some(Role::Region),
        "row" => Some(Role::Row),
        "rowgroup" => Some(Role::RowGroup),
        "rowheader" => Some(Role::RowHeader),
        "toolbar" => Some(Role::Toolbar),
        "application" => Some(Role::Application),
        "banner" => Some(Role::Banner),
        "complementary" => Some(Role::Complementary),
        "contentinfo" => Some(Role::ContentInfo),
        "form" => Some(Role::Form),
        "main" => Some(Role::Main),
        "navigation" => Some(Role::Navigation),
        "search" => Some(Role::Search),
        _ => None,
    }
}

fn role_from_element_data(element_data: &ElementData) -> Option<Role> {
    // <https://www.w3.org/TR/html-aam-1.0/>
    match &*element_data.name.local {
        // Document structure
        "article" => Some(Role::Article),
        "aside" => Some(Role::Complementary),
        "footer" => Some(Role::Footer),
        "header" => Some(Role::Header),
        "main" => Some(Role::Main),
        "nav" => Some(Role::Navigation),
        "search" => Some(Role::Search),
        "section" => Some(Role::Section),
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => Some(Role::Heading),
        "p" => Some(Role::Paragraph),
        "blockquote" => Some(Role::Blockquote),
        "figure" => Some(Role::Figure),
        "figcaption" | "caption" => Some(Role::Caption),
        "hr" => Some(Role::Splitter),

        // Grouping
        "ul" | "ol" | "menu" => Some(Role::List),
        "li" => Some(Role::ListItem),
        "dl" => Some(Role::DescriptionList),
        "dt" => Some(Role::Term),
        "dd" => Some(Role::Definition),
        "dialog" => Some(Role::Dialog),
        "fieldset" => Some(Role::Group),
        "form" => Some(Role::Form),
        "div" => Some(Role::GenericContainer),

        // Tables
        "table" => Some(Role::Table),
        "thead" | "tbody" | "tfoot" => Some(Role::RowGroup),
        "tr" => Some(Role::Row),
        "td" => Some(Role::Cell),
        "th" => match element_data.attr(local_name!("scope")) {
            Some("row") | Some("rowgroup") => Some(Role::RowHeader),
            _ => Some(Role::ColumnHeader),
        },

        // Interactive
        // An <a> is only a link when it has an href.
        "a" => match element_data.attr(local_name!("href")) {
            Some(_) => Some(Role::Link),
            None => Some(Role::GenericContainer),
        },
        "button" => Some(Role::Button),
        "label" => Some(Role::Label),
        "legend" => Some(Role::Label),
        "select" => match element_data.attr(local_name!("multiple")) {
            Some(_) => Some(Role::ListBox),
            None => Some(Role::ComboBox),
        },
        "option" => Some(Role::ListBoxOption),
        "textarea" => Some(Role::MultilineTextInput),
        "progress" => Some(Role::ProgressIndicator),
        "meter" => Some(Role::Meter),
        "output" => Some(Role::Status),
        "summary" => Some(Role::DisclosureTriangle),

        // Inline semantics
        "code" => Some(Role::Code),
        "em" => Some(Role::Emphasis),
        "strong" => Some(Role::Strong),
        "mark" => Some(Role::Mark),
        "time" => Some(Role::Time),
        "img" => Some(Role::Image),
        "iframe" => Some(Role::Iframe),

        "input" => {
            let ty = element_data.attr(local_name!("type")).unwrap_or("text");
            match ty {
                "button" | "submit" | "reset" => Some(Role::Button),
                "checkbox" => Some(Role::CheckBox),
                "color" => Some(Role::ColorWell),
                "date" => Some(Role::DateInput),
                "datetime-local" => Some(Role::DateTimeInput),
                "email" => Some(Role::EmailInput),
                "number" => Some(Role::NumberInput),
                "password" => Some(Role::PasswordInput),
                "radio" => Some(Role::RadioButton),
                "range" => Some(Role::Slider),
                "search" => Some(Role::SearchInput),
                "tel" => Some(Role::PhoneNumberInput),
                "time" => Some(Role::TimeInput),
                _ => Some(Role::TextInput),
            }
        }
        _ => None,
    }
}
