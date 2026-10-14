//! Inline content in the order layout construction pushes it to Parley (see
//! `layout/construct.rs`), shared by the caret builders that align DOM
//! characters with laid-out text.

use crate::layout::construct::{InlineElement, inline_element};
use crate::layout::list::is_symbol_marker;
use crate::node::{ListItemLayoutPosition, Marker, NodeData};
use crate::{BaseDocument, NodeId};
use style::values::computed::Display;
use style::values::specified::box_::{DisplayInside, DisplayOutside};

/// A node to visit; content of a generated (pseudo-element) subtree takes
/// no DOM positions.
#[derive(Clone, Copy, Debug)]
pub(super) struct Visit {
    pub(super) id: NodeId,
    pub(super) generated: bool,
}

/// What layout construction makes of a node in inline content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Content {
    Text,
    /// Nothing: not rendered, or not an element.
    Skip,
    /// `display: contents`: its children stand in its place.
    Contents,
    Atom,
    Break,
    /// An inline box whose content continues the inline content.
    Span,
    /// Any other box: out of flow, or an inline-level box with its own
    /// formatting context.
    Box(InlineElement),
}

pub(super) fn classify(doc: &BaseDocument, id: NodeId) -> Content {
    let node = &doc.nodes[id];
    let element = match &node.data {
        NodeData::Text(_) => return Content::Text,
        NodeData::Element(element) | NodeData::AnonymousBlock(element) => element,
        _ => return Content::Skip,
    };
    let kind = inline_element(element);
    if kind == InlineElement::Hidden {
        return Content::Skip;
    }
    let display = node.display_style().unwrap_or(Display::inline());
    match (display.outside(), display.inside()) {
        (DisplayOutside::None, DisplayInside::None) => Content::Skip,
        (DisplayOutside::None, DisplayInside::Contents) => Content::Contents,
        (DisplayOutside::Inline, DisplayInside::Flow) => match kind {
            InlineElement::Atom => Content::Atom,
            InlineElement::Break => Content::Break,
            InlineElement::Span | InlineElement::Hidden => Content::Span,
        },
        _ => Content::Box(kind),
    }
}

/// A node's children in construction order, reversed for a stack walk.
/// `pseudos` includes its `::before` and `::after`.
pub(super) fn children_for_stack(
    doc: &BaseDocument,
    id: NodeId,
    generated: bool,
    pseudos: bool,
) -> impl Iterator<Item = Visit> {
    let node = &doc.nodes[id];
    let pseudo = |id: Option<NodeId>| {
        id.filter(|_| pseudos).map(|id| Visit {
            id,
            generated: true,
        })
    };
    let children = node.flat_children().iter().rev();
    pseudo(node.after())
        .into_iter()
        .chain(children.map(move |&id| Visit { id, generated }))
        .chain(pseudo(node.before()))
}

/// An inside list marker, which precedes the root's content in its layout.
pub(super) struct InsideMarker {
    /// The marker's laid-out text.
    pub(super) text: String,
    /// The text is a symbol and a space standing for the end margin of
    /// Chrome's symbol marker (see `layout::list::inside_char_marker_style`),
    /// which is not marker content.
    pub(super) margin: bool,
}

pub(super) fn inside_marker(doc: &BaseDocument, root: NodeId) -> Option<InsideMarker> {
    let node = &doc.nodes[root];
    let item = node.element_data()?.list_item_data.as_deref()?;
    if !matches!(item.position, ListItemLayoutPosition::Inside) {
        return None;
    }
    Some(match &item.marker {
        Marker::Char(ch) => InsideMarker {
            text: format!("{ch} "),
            margin: node
                .primary_styles()
                .is_some_and(|style| is_symbol_marker(&style)),
        },
        Marker::String(text) => InsideMarker {
            text: text.clone(),
            margin: false,
        },
    })
}

/// Whether laid-out text is a forced break. Chrome steps over a CR and an
/// LF separately, though they form one grapheme cluster.
pub(super) fn is_forced_break(text: &str) -> bool {
    matches!(text, "\n" | "\r")
}
