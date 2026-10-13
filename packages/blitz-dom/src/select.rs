//! What a `<select>` shows: a drop-down shows its selected option's label,
//! and a list box shows a row for each option and option group.
//! https://html.spec.whatwg.org/multipage/form-elements.html#the-select-element
//!
//! Layout sizes and paints these from the document, and caret movement
//! steps through a list box's rows.

use crate::{BaseDocument, NodeId};
use markup5ever::{local_name, ns};

/// The rows a list box shows when `size` names none.
const DEFAULT_LIST_BOX_SIZE: u32 = 4;

/// How a select presents its options.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SelectDisplay {
    /// One line showing the selected option, with a button part.
    DropDown,
    /// `size` rows of options (`multiple`, or a display size above one).
    ListBox { size: u32 },
}

/// A row of a list box: an option, or the label of an option group.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ListBoxRow {
    pub(crate) element: NodeId,
    pub(crate) kind: RowKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RowKind {
    /// An `<option>`.
    Option,
    /// An `<optgroup>`'s `label`.
    GroupLabel,
}

impl BaseDocument {
    /// How a `<select>` presents its options; `None` for any other node.
    /// Chrome's `UsesMenuList`: a list box when `multiple` or when the
    /// display size is above one.
    pub(crate) fn select_display(&self, select: NodeId) -> Option<SelectDisplay> {
        let element = self.get_node(select)?.element_data()?;
        if element.name.ns != ns!(html) || element.name.local != local_name!("select") {
            return None;
        }
        let size = element.attr(local_name!("size")).map_or(0, display_size);
        if element.attr(local_name!("multiple")).is_none() && size <= 1 {
            return Some(SelectDisplay::DropDown);
        }
        let size = if size == 0 {
            DEFAULT_LIST_BOX_SIZE
        } else {
            size
        };
        Some(SelectDisplay::ListBox { size })
    }

    /// The label an option shows: its `label` attribute when that is not
    /// empty, otherwise its text, with HTML white space stripped and
    /// collapsed (Chrome's `HTMLOptionElement::DisplayLabel`).
    pub(crate) fn option_display_label(&self, option: NodeId) -> String {
        let label = self.group_or_option_label_attribute(option);
        if !label.is_empty() {
            return label;
        }
        self.option_text(option).as_str_lossy().to_owned()
    }

    /// The label an `<optgroup>` shows (Chrome's `GroupLabelText`).
    pub(crate) fn optgroup_display_label(&self, group: NodeId) -> String {
        self.group_or_option_label_attribute(group)
    }

    fn group_or_option_label_attribute(&self, element: NodeId) -> String {
        self.null_attribute(element, "label")
            .map(|attr| collapse_html_space(attr.value.as_str_lossy()))
            .unwrap_or_default()
    }

    /// Whether an option is a direct child of an `<optgroup>`, which
    /// indents it.
    pub(crate) fn option_is_grouped(&self, option: NodeId) -> bool {
        self.get_node(option)
            .and_then(|node| node.parent)
            .is_some_and(|parent| self.option_tag(parent) == Some("optgroup"))
    }

    /// A list box's rows in tree order: each option group's label before its
    /// options, and each option without `hidden`.
    pub(crate) fn list_box_rows(&self, select: NodeId) -> Vec<ListBoxRow> {
        self.select_items(select)
            .into_iter()
            .filter_map(|item| {
                let kind = if item.is_group {
                    RowKind::GroupLabel
                } else if self.null_attribute(item.id, "hidden").is_none() {
                    RowKind::Option
                } else {
                    return None;
                };
                Some(ListBoxRow {
                    element: item.id,
                    kind,
                })
            })
            .collect()
    }

    /// The label a list box row shows.
    pub(crate) fn list_box_row_label(&self, row: ListBoxRow) -> String {
        match row.kind {
            RowKind::Option => self.option_display_label(row.element),
            RowKind::GroupLabel => self.optgroup_display_label(row.element),
        }
    }
}

/// A select's display size, by the rules for parsing non-negative integers;
/// an invalid value is 0.
/// https://html.spec.whatwg.org/multipage/common-microsyntaxes.html#rules-for-parsing-non-negative-integers
fn display_size(value: &str) -> u32 {
    let value = value.trim_start_matches(['\t', '\n', '\u{c}', '\r', ' ']);
    let value = value.strip_prefix('+').unwrap_or(value);
    let end = value.bytes().take_while(u8::is_ascii_digit).count();
    value[..end].parse().unwrap_or(0)
}

/// Strip leading and trailing HTML white space and collapse the rest to
/// single spaces.
fn collapse_html_space(value: &str) -> String {
    let is_space = |c: char| matches!(c, '\t' | '\n' | '\u{c}' | '\r' | ' ');
    value
        .split(is_space)
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}
