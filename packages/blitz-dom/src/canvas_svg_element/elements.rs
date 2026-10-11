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
        "path" => path_curves(data("d")),
        "polyline" | "polygon" => svgtypes::PointsParser::from(data("points")).count(),
        _ => 0,
    }
}

/// The largest coordinate, length or radius a referenced element may use, in
/// user units: the parser splits arcs into a number of curves that grows with
/// their radius (kurbo's `Arc::to_cubic_beziers`), and a radius grows with
/// the coordinates an arc spans and the viewports percentages resolve
/// against. `<circle r="3e38">` alone would build millions of curves.
pub(super) const MAX_COORDINATE: f64 = 1e9;
/// The largest percentage a length may use. Viewport sizes may not exceed
/// 100%, so nested viewports never grow past the outermost one.
pub(super) const MAX_PERCENT: f64 = 1000.0;
/// The largest font size, in pixels, that font-relative units resolve with.
const MAX_FONT_SIZE: f64 = 10_000.0;

/// Attributes the parser reads numbers from.
const NUMERIC: [&str; 33] = [
    "d",
    "points",
    "x",
    "y",
    "x1",
    "y1",
    "x2",
    "y2",
    "cx",
    "cy",
    "r",
    "rx",
    "ry",
    "fx",
    "fy",
    "fr",
    "width",
    "height",
    "viewBox",
    "transform",
    "gradientTransform",
    "patternTransform",
    "stroke-width",
    "stroke-dashoffset",
    "dx",
    "dy",
    "refX",
    "refY",
    "markerWidth",
    "markerHeight",
    "startOffset",
    "textLength",
    "offset",
];

/// Whether every number `element` gives the parser stays within the bounds
/// above. Elements without styles also give their own font size, which must
/// be absolute.
pub(super) fn bounded_numbers(tag: &str, element: &ElementData, unstyled: bool) -> bool {
    let viewport_sizing = matches!(tag, "svg" | "symbol" | "use" | "image" | "foreignObject");
    element.attrs().iter().all(|attr| {
        let name = attr.name.local.as_ref();
        if attr.name.ns != markup5ever::ns!() {
            return true;
        }
        let value = attr.value.as_str_lossy();
        let percent_cap = match name {
            "width" | "height" if viewport_sizing => 100.0,
            _ => MAX_PERCENT,
        };
        if name == "font-size" && unstyled {
            return absolute_font_size(value);
        }
        !NUMERIC.contains(&name) || numbers_within(value, percent_cap)
    })
}

/// An absolute font size (a number with a fixed unit) within the bound.
fn absolute_font_size(value: &str) -> bool {
    svgtypes::Length::from_str(value.trim()).is_ok_and(|length| {
        use svgtypes::LengthUnit;
        let pixels = match length.unit {
            LengthUnit::None | LengthUnit::Px => 1.0,
            LengthUnit::In => 96.0,
            LengthUnit::Cm => 96.0 / 2.54,
            LengthUnit::Mm => 96.0 / 25.4,
            LengthUnit::Pt => 4.0 / 3.0,
            LengthUnit::Pc => 16.0,
            LengthUnit::Percent | LengthUnit::Em | LengthUnit::Ex => return false,
        };
        (length.number * pixels).abs() <= MAX_FONT_SIZE
    })
}

/// Whether every number in `value` is within `MAX_COORDINATE`, percentages
/// within `percent_cap` and font-relative lengths within what the largest
/// font size keeps under `MAX_COORDINATE`. Numbers are read as SVG reads
/// them: `.1.1` is two.
pub(super) fn numbers_within(value: &str, percent_cap: f64) -> bool {
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let Some((number, end)) = scan_number(bytes, index) else {
            index += 1;
            continue;
        };
        let unit = &value[end..];
        let limit = if unit.starts_with('%') {
            percent_cap
        } else if unit.starts_with("em") || unit.starts_with("ex") {
            MAX_COORDINATE / MAX_FONT_SIZE
        } else {
            MAX_COORDINATE
        };
        // NaN is never within a limit.
        if number.is_nan() || number.abs() > limit {
            return false;
        }
        index = end;
    }
    true
}

/// The number starting at `start`, if one does, and where it ends.
fn scan_number(bytes: &[u8], start: usize) -> Option<(f64, usize)> {
    let mut end = start;
    if matches!(bytes.get(end), Some(b'+' | b'-')) {
        end += 1;
    }
    let digits = |from: usize| {
        bytes[from..]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count()
    };
    let integer = digits(end);
    end += integer;
    let mut fraction = 0;
    if bytes.get(end) == Some(&b'.') {
        fraction = digits(end + 1);
        if fraction > 0 {
            end += 1 + fraction;
        }
    }
    if integer == 0 && fraction == 0 {
        return None;
    }
    if matches!(bytes.get(end), Some(b'e' | b'E')) {
        let mut exponent = end + 1;
        if matches!(bytes.get(exponent), Some(b'+' | b'-')) {
            exponent += 1;
        }
        let exponent_digits = digits(exponent);
        if exponent_digits > 0 {
            end = exponent + exponent_digits;
        }
    }
    let text = std::str::from_utf8(&bytes[start..end]).ok()?;
    Some((text.parse().ok()?, end))
}

/// How many cubic curves the parser makes of an arc of `radius`: kurbo's
/// count for a full turn at an accuracy of 0.1, at least four.
fn arc_curves(radius: f64) -> usize {
    let count = (1.1163 * radius.abs() / 0.1)
        .powf(1.0 / 6.0)
        .ceil()
        .max(4.0);
    // Radii are bounded by `MAX_COORDINATE`, which keeps this small.
    if count.is_finite() && count < 1e6 {
        count as usize
    } else {
        usize::MAX
    }
}

/// How many curves `d`'s arcs and other segments become: kurbo scales an
/// arc's radii up to half its chord, so the endpoints count too.
fn path_curves(d: &str) -> usize {
    use svgtypes::PathSegment;
    let (mut x, mut y, mut start) = (0.0, 0.0, (0.0, 0.0));
    let mut curves = 0usize;
    for segment in svgtypes::PathParser::from(d).map_while(Result::ok) {
        let (to, count) = match segment {
            PathSegment::EllipticalArc {
                abs,
                rx,
                ry,
                x: ex,
                y: ey,
                ..
            } => {
                let to = if abs { (ex, ey) } else { (x + ex, y + ey) };
                let chord = ((to.0 - x).powi(2) + (to.1 - y).powi(2)).sqrt() / 2.0;
                (Some(to), arc_curves(rx.abs().max(ry.abs()).max(chord)))
            }
            PathSegment::ClosePath { .. } => (Some(start), 1),
            other => (end_point(other, x, y), 1),
        };
        if let PathSegment::MoveTo { .. } = segment {
            start = to.unwrap_or((x, y));
        }
        if let Some((nx, ny)) = to {
            (x, y) = (nx, ny);
        }
        curves = curves.saturating_add(count);
    }
    curves
}

/// Where a segment other than an arc or a close ends.
fn end_point(segment: svgtypes::PathSegment, x: f64, y: f64) -> Option<(f64, f64)> {
    use svgtypes::PathSegment::*;
    let point = |abs: bool, px: f64, py: f64| if abs { (px, py) } else { (x + px, y + py) };
    Some(match segment {
        MoveTo { abs, x: px, y: py }
        | LineTo { abs, x: px, y: py }
        | CurveTo {
            abs, x: px, y: py, ..
        }
        | SmoothCurveTo {
            abs, x: px, y: py, ..
        }
        | Quadratic {
            abs, x: px, y: py, ..
        }
        | SmoothQuadratic { abs, x: px, y: py } => point(abs, px, py),
        HorizontalLineTo { abs, x: px } => (if abs { px } else { x + px }, y),
        VerticalLineTo { abs, y: py } => (x, if abs { py } else { y + py }),
        EllipticalArc { .. } | ClosePath { .. } => return None,
    })
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
