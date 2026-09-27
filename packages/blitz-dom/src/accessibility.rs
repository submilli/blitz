use crate::{BaseDocument, ElementData, Node as BlitzDomNode, local_name};
use accesskit::{Invalid, Node as AccessKitNode, NodeId, Rect, Role, Toggled, TreeId, TreeInfo, TreeUpdate};
use markup5ever::ns;
use style_dom::ElementState;
use style::properties::longhands::visibility;

impl BaseDocument {
    pub fn build_accessibility_tree(&self) -> TreeUpdate {
        let mut nodes = std::collections::HashMap::new();
        let mut window = AccessKitNode::new(Role::Window);
        let mut hidden_nodes = std::collections::HashSet::new();

        // The flat tree: shadow content where it renders.
        self.visit_flat(|node_id, node| {
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
            builder.set_value(node.text_content());
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

impl BaseDocument {
    /// Name, description, states, value and bounds of an element's node.
    fn set_element_properties(
        &self,
        node: &BlitzDomNode,
        element: &ElementData,
        role: Role,
        builder: &mut AccessKitNode,
    ) {
        let attr = |name: &str| element.attrs().iter().find(|a| &*a.name.local == name).map(|a| a.value.to_string());
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
        let disabled = element.element_state.contains(ElementState::DISABLED)
            || attr("aria-disabled").as_deref() == Some("true");
        if disabled {
            builder.set_disabled();
        }
        if element.attr(local_name!("required")).is_some() || attr("aria-required").as_deref() == Some("true") {
            builder.set_required();
        }
        if element.attr(local_name!("readonly")).is_some() || attr("aria-readonly").as_deref() == Some("true") {
            builder.set_read_only();
        }
        if attr("aria-invalid").is_some_and(|v| v != "false") {
            builder.set_invalid(Invalid::True);
        }
        match role {
            Role::CheckBox | Role::RadioButton | Role::Switch if tag == "input" => {
                builder.set_toggled(Toggled::from(self.checkedness(node.id)));
            }
            _ => match attr("aria-checked").or_else(|| attr("aria-pressed")).as_deref() {
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
                .map_or(element.attr(local_name!("selected")).is_some(), |s| s == node.id);
            builder.set_selected(selected);
        } else if let Some(selected) = attr("aria-selected") {
            builder.set_selected(selected == "true");
        }
        if let Some(expanded) = attr("aria-expanded") {
            builder.set_expanded(expanded == "true");
        } else if tag == "summary" {
            let open = node.parent.and_then(|p| self.nodes[p].element_data()).is_some_and(|d| {
                &*d.name.local == "details" && d.attr(local_name!("open")).is_some()
            });
            builder.set_expanded(open);
        }

        // Values.
        match tag {
            "input" | "textarea" if !matches!(role, Role::CheckBox | Role::RadioButton | Role::Button) => {
                if let Some(value) = self.form_value(node.id) {
                    builder.set_value(value);
                }
                if let Some(placeholder) = attr("placeholder") {
                    builder.set_placeholder(placeholder);
                }
            }
            "select" => {
                if let Some(option) = self.selected_option(node.id) {
                    builder.set_value(normalize(&self.nodes[option].text_content()));
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
        let level = attr("aria-level").and_then(|l| l.parse::<usize>().ok()).or(heading_level);
        if let Some(level) = level {
            builder.set_level(level);
        }
        if role == Role::Link
            && let Some(url) = attr("href").and_then(|href| self.resolve_url(&href))
        {
            builder.set_url(url.to_string());
        }

        // Bounds, from layout, in CSS pixels relative to the viewport.
        if let Some(rect) = self.get_client_bounding_rect(node.id) {
            builder.set_bounds(Rect { x0: rect.x, y0: rect.y, x1: rect.x + rect.width, y1: rect.y + rect.height });
        }
    }

    /// The accessible name (a practical subset of accname 1.2): labelled-by,
    /// `aria-label`, the host language's labels, content, then `title`.
    fn accessible_name(&self, node: &BlitzDomNode, element: &ElementData, role: Role) -> Option<String> {
        let attr = |name: &str| element.attrs().iter().find(|a| &*a.name.local == name).map(|a| a.value.to_string());
        let non_empty = |s: String| Some(normalize(&s)).filter(|s| !s.is_empty());
        if let Some(name) = attr("aria-labelledby").map(|ids| self.text_of_ids(&ids)).and_then(non_empty) {
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
            "option" => attr("label").or_else(|| Some(node.text_content())),
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
        let fallback = attr("title").or_else(|| if tag == "input" || tag == "textarea" { attr("placeholder") } else { None });
        fallback.and_then(non_empty)
    }

    /// Text of the elements with these (space-separated) ids.
    fn text_of_ids(&self, ids: &str) -> String {
        let parts: Vec<String> = ids
            .split_whitespace()
            .filter_map(|id| self.get_element_by_id(id))
            .map(|id| {
                let mut text = String::new();
                self.text_alternative(id, &mut text);
                normalize(&text)
            })
            .collect();
        parts.join(" ")
    }

    /// Text of a form control's labels: `<label for=id>`, and an ancestor
    /// `<label>`.
    fn label_text(&self, node: &BlitzDomNode) -> Option<String> {
        let mut labels = Vec::new();
        if let Some(id) = node.element_data().and_then(|e| e.attr(local_name!("id"))) {
            self.visit(|label_id, label| {
                if label.element_data().is_some_and(|l| {
                    l.name.ns == ns!(html) && &*l.name.local == "label" && l.attr(local_name!("for")) == Some(id)
                }) {
                    labels.push(label_id);
                }
            });
        }
        let mut ancestor = node.parent;
        while let Some(id) = ancestor {
            if self.nodes[id].element_data().is_some_and(|e| &*e.name.local == "label") {
                labels.push(id);
                break;
            }
            ancestor = self.nodes[id].parent;
        }
        let text: Vec<String> = labels
            .into_iter()
            .map(|id| {
                let mut text = String::new();
                self.text_alternative_skipping(id, Some(node.id), &mut text);
                normalize(&text)
            })
            .filter(|t| !t.is_empty())
            .collect();
        (!text.is_empty()).then(|| text.join(" "))
    }

    fn first_child_text(&self, node: &BlitzDomNode, tag: &str) -> Option<String> {
        let child = node
            .children
            .iter()
            .copied()
            .find(|&c| self.nodes[c].element_data().is_some_and(|e| &*e.name.local == tag))?;
        let mut text = String::new();
        self.text_alternative(child, &mut text);
        Some(text)
    }

    /// Text a subtree contributes to a name: its text, images' `alt`, and
    /// nothing from hidden parts.
    fn text_alternative(&self, id: crate::NodeId, out: &mut String) {
        self.text_alternative_skipping(id, None, out)
    }

    fn text_alternative_skipping(&self, id: crate::NodeId, skip: Option<crate::NodeId>, out: &mut String) {
        if Some(id) == skip {
            return;
        }
        let node = &self.nodes[id];
        if node.is_hidden_from_accessibility_tree() {
            return;
        }
        if node.is_text_node() {
            out.push_str(&node.text_content());
            return;
        }
        if let Some(element) = node.element_data() {
            if element.attr(local_name!("aria-hidden")) == Some("true") {
                return;
            }
            if &*element.name.local == "img" {
                if let Some(alt) = element.attr(local_name!("alt")) {
                    out.push(' ');
                    out.push_str(alt);
                    out.push(' ');
                }
                return;
            }
            if let Some(label) = element.attr(local_name!("aria-label")) {
                out.push(' ');
                out.push_str(label);
                out.push(' ');
                return;
            }
        }
        let before = out.trim_end().len();
        for &child in node.flat_children() {
            self.text_alternative_skipping(child, skip, out);
        }
        // An element that contributed no text contributes its tooltip
        // (accname: a descendant's name falls back to `title`).
        if out.trim_end().len() == before
            && let Some(title) = node.element_data().and_then(|e| e.attr(local_name!("title")))
        {
            out.push(' ');
            out.push_str(title);
        }
        // Block boundaries separate words.
        out.push(' ');
    }
}

impl BlitzDomNode {
    /// An `<option>` or `<optgroup>` inside a `<select>`.
    fn is_select_option(&self) -> bool {
        let Some(element) = self.element_data() else { return false };
        match &*element.name.local {
            "option" => self.ancestors_select().is_some(),
            "optgroup" => self
                .parent
                .is_some_and(|p| self.tree()[p].element_data().is_some_and(|e| &*e.name.local == "select")),
            _ => false,
        }
    }

    /// For an `<option>`: its `<select>` (parent, or grandparent through an
    /// `<optgroup>`).
    fn ancestors_select(&self) -> Option<crate::NodeId> {
        let parent = self.parent?;
        let tree = self.tree();
        let is = |id: crate::NodeId, tag: &str| tree[id].element_data().is_some_and(|e| &*e.name.local == tag);
        if is(parent, "select") {
            return Some(parent);
        }
        tree[parent].parent.filter(|&grand| is(parent, "optgroup") && is(grand, "select"))
    }

    // https://www.w3.org/TR/wai-aria-1.2/#tree_exclusion
    fn is_hidden_from_accessibility_tree(&self) -> bool {
        self.try_stylo_element_data()
            .as_ref()
            .and_then(|s| s.get())
            .map(|s| {
                s.styles.is_display_none()
                    || s.styles.primary().clone_visibility()
                        == visibility::computed_value::T::Hidden
            })
            .unwrap_or(false)
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
