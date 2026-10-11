//! Caret offsets of a laid-out Text node (Blink's `LayoutText` queries over
//! its formatting context's offset mapping).

use super::context::Context;
use crate::NodeId;

/// The first offset a caret can take: the start of the first rendered
/// character, or 0 when every character collapsed.
pub(super) fn caret_min_offset(cx: &Context, text: NodeId) -> usize {
    let Some(ifc) = cx.ifc_of(text) else {
        return 0;
    };
    match ifc.start_of_next_non_collapsed(text, 0) {
        Some(offset) if offset != cx.tree.text_length(text) => offset,
        _ => 0,
    }
}

/// The last offset a caret can take, or the length when every character
/// collapsed.
pub(super) fn caret_max_offset(cx: &Context, text: NodeId) -> usize {
    let length = cx.tree.text_length(text);
    let Some(ifc) = cx.ifc_of(text) else {
        return length;
    };
    match ifc.end_of_last_non_collapsed(text, length) {
        Some(offset) if offset != 0 => offset,
        _ => length,
    }
}

/// `EditingStrategy::CaretMaxOffset`: a Text node's last caret offset, or
/// a node's last editing offset.
pub(super) fn node_caret_max_offset(cx: &Context, node: NodeId) -> usize {
    if cx.tree.is_text(node) && cx.has_layout(node) {
        return caret_max_offset(cx, node);
    }
    cx.tree.last_offset_for_editing(node)
}

/// `CaretMinOffset` of any node: 0 except for Text.
pub(super) fn node_caret_min_offset(cx: &Context, node: NodeId) -> usize {
    if cx.tree.is_text(node) && cx.has_layout(node) {
        return caret_min_offset(cx, node);
    }
    0
}

pub(super) fn has_non_collapsed_text(cx: &Context, node: NodeId) -> bool {
    cx.ifc_of(node)
        .is_some_and(|ifc| ifc.has_non_collapsed_text(node))
}

/// Whether a caret can sit at the offset: it is before a rendered
/// character, or after one that is not a line feed.
pub(super) fn contains_caret_offset(cx: &Context, text: NodeId, offset: usize) -> bool {
    let Some(ifc) = cx.ifc_of(text) else {
        return false;
    };
    let length = cx.tree.text_length(text);
    if offset > length {
        return false;
    }
    if offset < length && ifc.is_before_non_collapsed(text, offset) {
        return true;
    }
    if offset == 0 || !ifc.is_after_non_collapsed(text, offset) {
        return false;
    }
    ifc.character_before(text, offset)
        .is_some_and(|ch| ch != u16::from(b'\n'))
}

pub(super) fn is_before_non_collapsed_character(cx: &Context, text: NodeId, offset: usize) -> bool {
    if offset >= cx.tree.text_length(text) {
        return false;
    }
    cx.ifc_of(text)
        .is_some_and(|ifc| ifc.is_before_non_collapsed(text, offset))
}

pub(super) fn is_after_non_collapsed_character(cx: &Context, text: NodeId, offset: usize) -> bool {
    if offset == 0 {
        return false;
    }
    cx.ifc_of(text)
        .is_some_and(|ifc| ifc.is_after_non_collapsed(text, offset))
}
