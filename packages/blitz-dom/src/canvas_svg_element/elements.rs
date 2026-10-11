//! Which elements and names enter serialized markup, shared by the plan and
//! the writer so both walk the same nodes and read references alike.

use super::MAX_ATTRIBUTE_LEN;
use crate::node::ElementData;
use crate::{BaseDocument, NodeId};
use std::str::FromStr;

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
    let mut chars = name.chars();
    chars.next().is_some_and(name_start) && chars.all(|ch| name_start(ch) || name_char(ch))
}

/// XML 1.0's `NameStartChar`, without `:`.
fn name_start(ch: char) -> bool {
    matches!(ch,
        'A'..='Z' | '_' | 'a'..='z'
        | '\u{C0}'..='\u{D6}' | '\u{D8}'..='\u{F6}' | '\u{F8}'..='\u{2FF}'
        | '\u{370}'..='\u{37D}' | '\u{37F}'..='\u{1FFF}' | '\u{200C}'..='\u{200D}'
        | '\u{2070}'..='\u{218F}' | '\u{2C00}'..='\u{2FEF}' | '\u{3001}'..='\u{D7FF}'
        | '\u{F900}'..='\u{FDCF}' | '\u{FDF0}'..='\u{FFFD}' | '\u{10000}'..='\u{EFFFF}')
}

/// XML 1.0's `NameChar` beyond `NameStartChar`.
fn name_char(ch: char) -> bool {
    matches!(ch,
        '-' | '.' | '0'..='9' | '\u{B7}' | '\u{300}'..='\u{36F}' | '\u{203F}'..='\u{2040}')
}

/// Shapes the parser draws markers on but Blink does not.
pub(super) fn unmarked_shape(tag: &str) -> bool {
    matches!(tag, "rect" | "circle" | "ellipse")
}

/// Whether the parser paints `tag` with a fill or stroke paint server.
pub(super) fn takes_paint(tag: &str) -> bool {
    matches!(
        tag,
        "rect" | "circle" | "ellipse" | "line" | "path" | "polyline" | "polygon" | "text"
    )
}

/// Whether Blink draws markers on `tag`.
pub(super) fn markable(tag: &str) -> bool {
    matches!(tag, "line" | "path" | "polyline" | "polygon")
}

/// How many vertices a marker may be drawn at on `element`, read with the
/// parser's own path and point grammars: an arc counts as the curves it
/// becomes.
pub(super) fn middle_vertices(tag: &str, element: &ElementData) -> usize {
    let data = |name: &str| {
        element
            .attr(markup5ever::LocalName::from(name))
            .unwrap_or("")
    };
    match tag {
        "path" => svgtypes::PathParser::from(data("d"))
            .map_while(Result::ok)
            .map(|segment| match segment {
                svgtypes::PathSegment::EllipticalArc { rx, ry, .. } => arc_curves(rx.max(ry)),
                _ => 1,
            })
            .fold(0usize, usize::saturating_add),
        "polyline" | "polygon" => svgtypes::PointsParser::from(data("points")).count(),
        _ => 0,
    }
}

/// The largest radius an arc may have, in user units: the parser splits arcs
/// into a number of curves that grows with the radius (kurbo's
/// `Arc::to_cubic_beziers`), so `<circle r="3e38">` alone would build
/// millions.
pub(super) const MAX_RADIUS: f64 = 1e9;

/// How many cubic curves the parser makes of a full arc of `radius`
/// (kurbo's subdivision for an accuracy of 0.1).
fn arc_curves(radius: f64) -> usize {
    let per_quarter = (radius.abs() / 0.1).powf(1.0 / 6.0).ceil();
    // A bounded radius makes a small count; others are refused by
    // `bounded_radii`.
    if per_quarter.is_finite() && per_quarter < 1e6 {
        4 * (per_quarter as usize).max(1)
    } else {
        usize::MAX
    }
}

/// Whether `element`'s arcs and rounded corners have radii the parser can
/// subdivide within bounds. Percentages count against the largest viewport
/// a capture accepts and font units against the largest font size.
pub(super) fn bounded_radii(tag: &str, element: &ElementData) -> bool {
    let attr = |name: &str| element.attr(markup5ever::LocalName::from(name));
    let bounded = |name: &str| {
        attr(name).is_none_or(|value| {
            svgtypes::Length::from_str(value.trim()).map_or(true, |length| {
                use svgtypes::LengthUnit;
                let scale = match length.unit {
                    LengthUnit::None | LengthUnit::Px => 1.0,
                    LengthUnit::Percent => MAX_RADIUS / 100.0,
                    LengthUnit::Em | LengthUnit::Ex => 10_000.0,
                    _ => 100.0,
                };
                (length.number * scale).abs() <= MAX_RADIUS
            })
        })
    };
    match tag {
        "circle" => bounded("r"),
        "ellipse" | "rect" => bounded("rx") && bounded("ry"),
        "path" => svgtypes::PathParser::from(attr("d").unwrap_or(""))
            .map_while(Result::ok)
            .all(|segment| match segment {
                svgtypes::PathSegment::EllipticalArc { rx, ry, .. } => {
                    rx.abs() <= MAX_RADIUS && ry.abs() <= MAX_RADIUS
                }
                _ => true,
            }),
        _ => true,
    }
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
