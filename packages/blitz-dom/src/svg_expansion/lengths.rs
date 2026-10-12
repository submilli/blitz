//! Upper bounds on the lengths usvg resolves: font-relative ones against
//! the largest font size an element may have, percentages against the
//! largest viewport (`units::convert_length` and `resolve_font_size`).

use std::str::FromStr;
use svgtypes::{Length, LengthUnit as Unit};

/// usvg's default font size (`Options::font_size`), which the root inherits.
pub(super) const DEFAULT_FONT_SIZE: f64 = 12.0;
/// The viewport a root without a size resolves percentages against.
pub(super) const DEFAULT_VIEWPORT: f64 = 100.0;
/// Pixels per inch (`Options::dpi`).
const DPI: f64 = 96.0;

/// The pixels an absolute unit is worth; `None` for relative units.
fn absolute(unit: Unit) -> Option<f64> {
    Some(match unit {
        Unit::None | Unit::Px => 1.0,
        Unit::In => DPI,
        Unit::Cm => DPI / 2.54,
        Unit::Mm => DPI / 25.4,
        Unit::Pt => DPI / 72.0,
        Unit::Pc => DPI / 6.0,
        Unit::Em | Unit::Ex | Unit::Percent => return None,
    })
}

/// The largest font size `value` may give, below a `parent` size, which
/// relative sizes scale.
pub(super) fn font_size(value: &str, parent: f64) -> f64 {
    // usvg's named sizes scale by at most 1.2 cubed.
    length(value, parent, parent).unwrap_or(parent * 1.2f64.powi(3))
}

/// The largest user length `value` may resolve to for an element of font
/// size `font` in a viewport no larger than `viewport` on either side;
/// `None` when it is not a length.
pub(super) fn length(value: &str, font: f64, viewport: f64) -> Option<f64> {
    let length = Length::from_str(value).ok()?;
    let number = length.number.abs();
    Some(match length.unit {
        Unit::Em => number * font,
        Unit::Ex => number * font / 2.0,
        // Against a side, or the viewport's diagonal over √2.
        Unit::Percent => number * viewport / 100.0,
        unit => number * absolute(unit).unwrap_or(1.0),
    })
}

/// The largest side `view_box` declares, if it declares any.
pub(super) fn view_box_side(view_box: &str) -> Option<f64> {
    let mut numbers = svgtypes::NumberListParser::from(view_box).map_while(Result::ok);
    let _ = (numbers.next()?, numbers.next()?);
    let (width, height) = (numbers.next()?, numbers.next()?);
    Some(width.abs().max(height.abs()))
}

/// A radius as usvg keeps it: a shape whose radius is not a finite single
/// precision number is skipped, so a larger bound is no larger than that.
pub(super) fn radius(bound: f64) -> f64 {
    if bound.is_nan() {
        0.0
    } else {
        bound.min(f64::from(f32::MAX))
    }
}
