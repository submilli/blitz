//! The laid-out content of a `<select>`, which layout sizes the control by
//! and paint draws. Chrome builds it as the control's user-agent shadow
//! tree; here it is data on the element.

use super::TextBrush;
use crate::NodeId;

/// What a select shows, laid out.
#[derive(Clone)]
pub struct SelectLayout {
    /// The intrinsic size of the content box, in CSS px.
    pub content_size: taffy::Size<f32>,
    /// The drop-down's line or the list box's rows.
    pub content: SelectContent,
}

/// What a select shows, by how it presents its options.
#[derive(Clone)]
pub enum SelectContent {
    DropDown(DropDownLayout),
    ListBox(ListBoxLayout),
}

/// A drop-down's one line: the selected option's label.
#[derive(Clone)]
pub struct DropDownLayout {
    /// The label on one line, in device pixels; `None` without one.
    pub label: Option<Box<parley::Layout<TextBrush>>>,
    /// The line's height, in CSS px, centered in the content box.
    pub line_height: f32,
    /// The line's baseline, from its top, in CSS px.
    pub baseline: f32,
    /// The label's start from the content box's start edge, in CSS px.
    pub label_inset: f32,
    /// The button part at the content box's end edge, arrow included, in
    /// CSS px.
    pub button_width: f32,
}

/// A list box's rows from the top of its content box.
#[derive(Clone)]
pub struct ListBoxLayout {
    pub rows: Vec<ListBoxRowLayout>,
    /// The scrollbar reserved at the content box's end edge, in CSS px.
    pub scrollbar_width: f32,
}

/// One row: an option, or an option group's label.
#[derive(Clone)]
pub struct ListBoxRowLayout {
    pub element: NodeId,
    /// The label on one line, in device pixels.
    pub label: Box<parley::Layout<TextBrush>>,
    /// The row's top and height, in CSS px.
    pub top: f32,
    pub height: f32,
    /// The label's start from the content box's start edge, in CSS px.
    pub inset: f32,
    /// The label's baseline from the row's top, in CSS px.
    pub baseline: f32,
    pub selected: bool,
}

impl SelectLayout {
    /// The baseline from the content box's top, given its used height: a
    /// drop-down's label baseline. A list box has none.
    pub fn content_baseline(&self, content_height: f32) -> Option<f32> {
        let SelectContent::DropDown(drop_down) = &self.content else {
            return None;
        };
        Some(drop_down.line_top(content_height) + drop_down.baseline)
    }
}

impl DropDownLayout {
    /// The line's top in a content box of this height: centered, as
    /// Chrome centers its menu list's inner element.
    pub fn line_top(&self, content_height: f32) -> f32 {
        (content_height - self.line_height) / 2.0
    }
}

impl std::fmt::Debug for SelectLayout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let kind = match &self.content {
            SelectContent::DropDown(_) => "drop-down",
            SelectContent::ListBox(list) => &format!("list box, {} rows", list.rows.len()),
        };
        write!(f, "SelectLayout({kind}, {:?})", self.content_size)
    }
}
