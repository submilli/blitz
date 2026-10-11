//! Which elements and names enter serialized markup, shared by the plan and
//! the writer so both walk the same nodes and read references alike.

use super::MAX_ATTRIBUTE_LEN;
use crate::node::ElementData;
use crate::{BaseDocument, NodeId};

/// Containers Blink lays out without painting (`LayoutSVGHiddenContainer`):
/// a reference to one draws nothing, and lengths inside never make a
/// reference viewport-dependent.
pub(super) const HIDDEN_CONTAINERS: [&str; 9] = [
    "defs",
    "linearGradient",
    "radialGradient",
    "pattern",
    "clipPath",
    "mask",
    "marker",
    "filter",
    "symbol",
];

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

/// The `href` a referencing element uses: SVG 2's, else XLink's.
pub(super) fn href(element: &ElementData) -> Option<&str> {
    image_href(element).filter(|href| href.len() <= MAX_ATTRIBUTE_LEN)
}

/// An `<image>`'s `href`, as long as it is: the markup bound limits it.
pub(super) fn image_href(element: &ElementData) -> Option<&str> {
    let named = |ns: &markup5ever::Namespace| {
        element
            .attrs()
            .iter()
            .find(|attr| attr.name.ns == *ns && attr.name.local.as_ref() == "href")
    };
    let attr = named(&markup5ever::ns!()).or_else(|| named(&markup5ever::ns!(xlink)))?;
    Some(attr.value.as_str_lossy())
}

/// A `url(#id)` presentation attribute of an element without styles: the ID
/// and what to paint without it (a paint's fallback, else `none`).
pub(super) fn attribute_reference(value: &str) -> Option<(String, String)> {
    if value.len() > MAX_ATTRIBUTE_LEN {
        return None;
    }
    let value = value.trim();
    let rest = value.strip_prefix("url(")?;
    let close = rest.find(')')?;
    let id = fragment(
        rest[..close]
            .trim()
            .trim_matches(|ch| ch == '"' || ch == '\'')
            .strip_prefix('#')?,
    )?;
    let fallback = rest[close + 1..].trim();
    let fallback = if fallback.is_empty() {
        "none"
    } else {
        fallback
    };
    Some((id, fallback.to_owned()))
}

/// An ID from a URL fragment, where IDs are percent-encoded.
pub(super) fn fragment(id: &str) -> Option<String> {
    if id.is_empty() || id.len() > MAX_ATTRIBUTE_LEN {
        return None;
    }
    Some(
        percent_encoding::percent_decode_str(id)
            .decode_utf8_lossy()
            .into_owned(),
    )
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

/// Whether the parser paints `tag` with a fill or stroke paint server.
pub(super) fn painted(tag: &str) -> bool {
    matches!(
        tag,
        "rect" | "circle" | "ellipse" | "line" | "path" | "polyline" | "polygon" | "text"
    )
}

/// Whether Blink draws markers on `tag`.
pub(super) fn markable(tag: &str) -> bool {
    matches!(tag, "line" | "path" | "polyline" | "polygon")
}

/// How many middle vertices a marker may be drawn at on `element`: at most
/// half its path's numbers.
pub(super) fn middle_vertices(tag: &str, element: &crate::node::ElementData) -> usize {
    let numbers = |name: &str| {
        element
            .attr(markup5ever::LocalName::from(name))
            .map_or(0, count_numbers)
    };
    match tag {
        "path" => numbers("d") / 2,
        "polyline" | "polygon" => numbers("points") / 2,
        _ => 0,
    }
}

/// The numbers in path or point data: runs of digits and points.
fn count_numbers(data: &str) -> usize {
    let mut count = 0;
    let mut in_number = false;
    for byte in data.bytes() {
        let digit = byte.is_ascii_digit() || byte == b'.';
        if digit && !in_number {
            count += 1;
        }
        in_number = digit;
    }
    count
}

#[cfg(test)]
mod tests {
    use super::attribute_reference;

    #[test]
    fn attribute_references_keep_their_fallback() {
        let found = |value| attribute_reference(value);
        assert_eq!(found("url(#a) red"), Some(("a".into(), "red".into())));
        assert_eq!(found(" url('#a%20b')"), Some(("a b".into(), "none".into())));
        assert_eq!(found("url(other.svg#a)"), None);
        assert_eq!(found("red"), None);
    }
}
