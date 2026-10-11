//! The size of a `<select>` control.

use crate::BaseDocument;
use markup5ever::local_name;

// A select's metrics, in ems of its font: an average label character, the
// drop-down arrow with its padding, and the line height without a resolved
// `line-height`.
const CHAR_WIDTH_EM: f32 = 0.6;
const ARROW_WIDTH_EM: f32 = 1.5;
const LINE_HEIGHT_EM: f32 = 1.2;
/// The UA default font size, for a select without computed font styles.
const DEFAULT_FONT_SIZE: f32 = 16.0;

/// A select is a control as wide as its longest option label and its arrow,
/// one line tall. The labels are not shaped, so this approximates Chrome's
/// metrics, and list boxes (`multiple`, `size`) are sized as drop-downs
/// (submilli SUB-1575).
pub(super) fn content_size(
    label_length: usize,
    font_size: Option<f32>,
    line_height: Option<f32>,
) -> taffy::Size<f32> {
    let font_size = font_size.unwrap_or(DEFAULT_FONT_SIZE);
    taffy::Size {
        width: label_length as f32 * font_size * CHAR_WIDTH_EM + font_size * ARROW_WIDTH_EM,
        height: line_height.unwrap_or(font_size * LINE_HEIGHT_EM),
    }
}

impl BaseDocument {
    /// For a `<select>`, the length in characters of its longest option
    /// label, which sizes the control.
    pub(super) fn select_label_length(&self, id: blitz_traits::node_id::NodeId) -> Option<usize> {
        let node = self.get_node(id)?;
        if !node.data.is_element_with_tag_name(&local_name!("select")) {
            return None;
        }
        let mut longest = 0;
        let mut pending: Vec<_> = node.children.iter().copied().collect();
        // Options sit in the select or in its option groups.
        while let Some(child) = pending.pop() {
            let Some(child) = self.get_node(child) else {
                continue;
            };
            if child
                .data
                .is_element_with_tag_name(&local_name!("optgroup"))
            {
                pending.extend(child.children.iter().copied());
            } else if child.data.is_element_with_tag_name(&local_name!("option")) {
                longest = longest.max(child.text_content().trim().chars().count());
            }
        }
        Some(longest)
    }
}
