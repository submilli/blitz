//! The numbers a referenced element gives the parser: bounds on them, and
//! how many curves its arcs become.

use crate::node::ElementData;
use std::str::FromStr;

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
pub(super) const MAX_FONT_SIZE: f64 = 10_000.0;

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
/// be absolute. `outermost` is an outermost `<svg>` referenced itself.
pub(super) fn bounded_numbers(
    tag: &str,
    element: &ElementData,
    unstyled: bool,
    outermost: bool,
) -> bool {
    // An outermost `<svg>` sizes against its own box, bounded with the viewport.
    let viewport_sizing =
        !outermost && matches!(tag, "svg" | "symbol" | "use" | "image" | "foreignObject");
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
pub(super) fn arc_curves(radius: f64) -> usize {
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
pub(super) fn path_curves(d: &str) -> usize {
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
