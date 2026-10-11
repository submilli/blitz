//! Which elements and names enter serialized markup, shared by the plan and
//! the writer so both walk the same nodes.

use super::MAX_ATTRIBUTE_LEN;
use crate::{BaseDocument, NodeId};

/// The tag of `id` when it enters serialized markup. Scripts, styles,
/// metadata and the effects Blink leaves out (clips, masks and filters) never
/// enter, nor names the HTML parser accepts but XML does not. Elements without
/// computed styles (below `display: none`) enter with their attributes only,
/// since a `<use>` may still instance them.
pub(super) fn included(doc: &BaseDocument, id: NodeId) -> Option<&str> {
    let element = doc.nodes[id].element_data()?;
    let tag = element.name.local.as_ref();
    let skipped = matches!(
        tag,
        "script"
            | "style"
            | "clipPath"
            | "mask"
            | "filter"
            | "foreignObject"
            | "title"
            | "desc"
            | "metadata"
    );
    if element.name.ns != markup5ever::ns!(svg) || skipped || !xml_name(tag) {
        return None;
    }
    Some(tag)
}

/// The `href` an element uses: SVG 2's, else XLink's.
pub(super) fn href(element: &crate::node::ElementData) -> Option<&str> {
    let named = |ns: &markup5ever::Namespace| {
        element
            .attrs()
            .iter()
            .find(|attr| attr.name.ns == *ns && attr.name.local.as_ref() == "href")
    };
    let attr = named(&markup5ever::ns!()).or_else(|| named(&markup5ever::ns!(xlink)))?;
    (attr.value.utf16_len_bound() <= MAX_ATTRIBUTE_LEN).then(|| attr.value.as_str_lossy())
}

/// Whether `name` is an XML name without a prefix: a page may create elements
/// and attributes whose names would break the serialized markup, or name
/// prefixes it never declares.
pub(super) fn xml_name(name: &str) -> bool {
    let start = |ch: char| ch.is_alphabetic() || ch == '_';
    let mut chars = name.chars();
    chars.next().is_some_and(start)
        && chars.all(|ch| start(ch) || ch.is_alphanumeric() || matches!(ch, '-' | '.'))
}
