//! Chrome's reading of filter primitive attributes where the engine's SVG parser
//! (usvg) applies different defaults. Values are rewritten before parsing, so the
//! parsed graph matches what Chrome renders.

/// How one attribute enters the snapshot markup.
#[derive(Debug, PartialEq)]
pub(crate) enum Attribute<'a> {
    Keep(&'a str),
    Replace(String),
    Omit,
    /// A nonpositive width or height, which usvg rejects: Chrome ignores such a
    /// filter and gives such a primitive an empty subregion.
    EmptySize,
}

/// Region coordinates past this magnitude lie beyond every admitted filter region
/// (1e6); clamping keeps usvg's bounding-box products finite and `x + width`
/// distinct in f32.
const MAX_COORDINATE: f64 = 1e7;

/// A radius that rounds to zero: Chrome passes that axis through unchanged, while
/// usvg replaces zero radii with one.
const ZERO_RADIUS: &str = "0.000001";
/// An order whose kernel can never match a bounded `kernelMatrix`, so the parser
/// produces the transparent result Chrome renders for an invalid kernel.
const INVALID_ORDER: &str = "4096";
/// Radii past this already cover any raster (execution caps them at 256 pixels);
/// the bound keeps bounding-box scaling finite, where usvg would panic on infinity.
const MAX_RADIUS: f32 = 1e6;

pub(crate) fn chrome_attribute<'a>(tag: &str, name: &str, value: &'a str) -> Attribute<'a> {
    match (tag, name) {
        // https://drafts.fxtf.org/filter-effects/#element-attrdef-femorphology-radius
        ("feMorphology", "radius") => {
            let [x, y] = number_optional_number(value).unwrap_or([0.0, 0.0]);
            let radius = |r: f32| {
                if r > 0.0 {
                    r.min(MAX_RADIUS).to_string()
                } else {
                    ZERO_RADIUS.to_owned()
                }
            };
            Attribute::Replace(format!("{} {}", radius(x), radius(y)))
        }
        // Chrome treats a zero divisor as absent and uses the kernel sum; a
        // malformed one is present with its initial value of 1.
        ("feConvolveMatrix", "divisor") => match number(value) {
            Some(0.0) => Attribute::Omit,
            Some(_) => Attribute::Keep(value),
            None => Attribute::Replace("1".to_owned()),
        },
        // An unparsable order keeps Chrome's default of 3; usvg would read the
        // leading numbers instead, and multiply them without overflow checks.
        ("feConvolveMatrix", "order") => match number_optional_number(value) {
            Some([x, y]) if x.trunc() < 1.0 || y.trunc() < 1.0 || x > 4096.0 || y > 4096.0 => {
                Attribute::Replace(INVALID_ORDER.to_owned())
            }
            Some(_) => Attribute::Keep(value),
            None => Attribute::Replace("3".to_owned()),
        },
        // Chrome parses these as integers; a malformed value is the attribute's
        // initial value, where usvg would truncate or round it.
        ("feConvolveMatrix", "targetX" | "targetY") if integer(value).is_none() => {
            Attribute::Replace("0".to_owned())
        }
        ("feTurbulence", "numOctaves") if integer(value).is_none() => {
            Attribute::Replace("1".to_owned())
        }
        // An unparsable frequency, including one beyond the f32 range that usvg
        // would unwrap as infinity, is Chrome's default of zero.
        ("feTurbulence", "baseFrequency") => match number_optional_number(value) {
            Some(_) => Attribute::Keep(value),
            None => Attribute::Replace("0".to_owned()),
        },
        // Chrome clamps the exponent; usvg drops the whole primitive.
        ("feSpecularLighting", "specularExponent") => match number(value) {
            Some(exponent) => Attribute::Replace(exponent.clamp(1.0, 128.0).to_string()),
            None => Attribute::Omit,
        },
        // Result names are strings, even when they look like numbers.
        (_, "in" | "in2" | "result") => Attribute::Keep(value),
        (tag, "x" | "y" | "width" | "height") if !matches!(tag, "fePointLight" | "feSpotLight") => {
            region(name, value)
        }
        _ if overflows(value) => Attribute::Omit,
        _ => Attribute::Keep(value),
    }
}

/// One or two numbers separated by whitespace and/or a comma, each within the
/// f32 range the parser stores.
fn number_optional_number(value: &str) -> Option<[f32; 2]> {
    let mut numbers = svgtypes::NumberListParser::from(value);
    let first = finite(numbers.next()?.ok()?)?;
    let second = match numbers.next() {
        Some(number) => finite(number.ok()?)?,
        None => first,
    };
    numbers.next().is_none().then_some([first, second])
}

/// A list of numbers that Chrome rejects as a parse error because one exceeds the
/// f32 range, so the element's initial value applies. usvg would read infinity;
/// omitting the attribute gives its matching default.
fn overflows(value: &str) -> bool {
    let numbers: Result<Vec<f64>, _> = svgtypes::NumberListParser::from(value).collect();
    numbers.is_ok_and(|numbers| numbers.into_iter().any(|n| finite(n).is_none()))
}

/// A filter or primitive region length. Chrome reads huge values as effectively
/// infinite: a distant finite stand-in keeps the region out of reach without
/// overflowing usvg, whose bounding-box transform unwraps a finite product. The
/// stand-in is unitless: any unit on such a length leaves it out of reach too.
fn region<'a>(name: &str, value: &'a str) -> Attribute<'a> {
    let Ok(length) = value.trim().parse::<svgtypes::Length>() else {
        return Attribute::Keep(value);
    };
    let size = matches!(name, "width" | "height");
    if size && length.number <= 0.0 {
        return Attribute::EmptySize;
    }
    if length.number.abs() > MAX_COORDINATE {
        return Attribute::Replace(MAX_COORDINATE.copysign(length.number).to_string());
    }
    Attribute::Keep(value)
}

/// An SVG number within the f32 range.
fn number(value: &str) -> Option<f32> {
    let number: svgtypes::Number = value.trim().parse().ok()?;
    finite(number.0)
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "Out-of-range values become infinite and are rejected."
)]
fn finite(value: f64) -> Option<f32> {
    Some(value as f32).filter(|n| n.is_finite())
}

/// An SVG integer: an optional sign and decimal digits.
fn integer(value: &str) -> Option<i32> {
    let value = value.trim();
    let digits = value.strip_prefix(['+', '-']).unwrap_or(value);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn morphology_radii_pass_zero_negative_and_invalid_axes_through() {
        let radius = |value| chrome_attribute("feMorphology", "radius", value);
        assert_eq!(radius("2"), Attribute::Replace("2 2".into()));
        assert_eq!(
            radius("0 1.5"),
            Attribute::Replace(format!("{ZERO_RADIUS} 1.5"))
        );
        assert_eq!(
            radius("-1,3"),
            Attribute::Replace(format!("{ZERO_RADIUS} 3"))
        );
        for invalid in ["", "1 2 3", "inf", "a"] {
            assert_eq!(
                radius(invalid),
                Attribute::Replace(format!("{ZERO_RADIUS} {ZERO_RADIUS}")),
                "{invalid}"
            );
        }
    }

    #[test]
    fn convolution_orders_and_divisors_follow_chrome() {
        let convolve = |name, value| chrome_attribute("feConvolveMatrix", name, value);
        assert_eq!(convolve("divisor", " 0 "), Attribute::Omit);
        assert_eq!(convolve("divisor", "2"), Attribute::Keep("2"));
        for malformed in ["abc", "1e39"] {
            assert_eq!(
                convolve("divisor", malformed),
                Attribute::Replace("1".into())
            );
        }
        assert_eq!(convolve("order", "3 2"), Attribute::Keep("3 2"));
        for invalid in ["-3", "0 3", "0.5", "5000 1"] {
            assert_eq!(
                convolve("order", invalid),
                Attribute::Replace(INVALID_ORDER.into()),
                "{invalid}"
            );
        }
    }

    #[test]
    fn values_that_would_overflow_the_parser_are_replaced() {
        assert_eq!(
            chrome_attribute("feMorphology", "radius", "3e38 1"),
            Attribute::Replace("1000000 1".into())
        );
        for invalid in ["1e39", "1e39 0.1", "a", "1 2 3"] {
            assert_eq!(
                chrome_attribute("feTurbulence", "baseFrequency", invalid),
                Attribute::Replace("0".into()),
                "{invalid}"
            );
        }
        assert_eq!(
            chrome_attribute("feTurbulence", "baseFrequency", "0.1 2"),
            Attribute::Keep("0.1 2")
        );
        for invalid in ["a", "3 3 3", "2147483647 2147483647 0"] {
            assert_eq!(
                chrome_attribute("feConvolveMatrix", "order", invalid),
                Attribute::Replace("3".into()),
                "{invalid}"
            );
        }
    }

    #[test]
    fn numbers_beyond_the_f32_range_take_initial_values() {
        for (tag, name, value) in [
            ("feDiffuseLighting", "surfaceScale", "1e39"),
            ("fePointLight", "x", "-1e39"),
            ("feColorMatrix", "values", "1 0 0 0 1e39"),
            ("feSpecularLighting", "specularExponent", "1e39"),
        ] {
            assert_eq!(
                chrome_attribute(tag, name, value),
                Attribute::Omit,
                "{name}"
            );
        }
        assert_eq!(
            chrome_attribute("feFlood", "x", "1e39"),
            Attribute::Replace("10000000".into())
        );
        assert_eq!(
            chrome_attribute("filter", "y", "-1e37px"),
            Attribute::Replace("-10000000".into())
        );
        assert_eq!(
            chrome_attribute("filter", "width", "1e37"),
            Attribute::Replace("10000000".into())
        );
        for size in ["-1e39", "0", "-2%"] {
            assert_eq!(
                chrome_attribute("feFlood", "width", size),
                Attribute::EmptySize,
                "{size}"
            );
        }
        assert_eq!(
            chrome_attribute("feFlood", "x", "-3"),
            Attribute::Keep("-3")
        );
        assert_eq!(
            chrome_attribute("feFlood", "width", "120%"),
            Attribute::Keep("120%")
        );
        assert_eq!(
            chrome_attribute("feFlood", "result", "1e39"),
            Attribute::Keep("1e39")
        );
        for value in ["3e38", "1 2", "SourceGraphic", "10%"] {
            assert_eq!(
                chrome_attribute("feOffset", "dx", value),
                Attribute::Keep(value)
            );
        }
    }

    #[test]
    fn malformed_integers_take_their_initial_values() {
        for (name, value, replaced) in [
            ("targetX", "1.5", Some("0")),
            ("targetY", "1e1", Some("0")),
            ("targetX", " 2 ", None),
            ("targetY", "-1", None),
        ] {
            let expected =
                replaced.map_or(Attribute::Keep(value), |v| Attribute::Replace(v.into()));
            assert_eq!(
                chrome_attribute("feConvolveMatrix", name, value),
                expected,
                "{value}"
            );
        }
        assert_eq!(
            chrome_attribute("feTurbulence", "numOctaves", "2.5"),
            Attribute::Replace("1".into())
        );
        assert_eq!(
            chrome_attribute("feTurbulence", "numOctaves", "3"),
            Attribute::Keep("3")
        );
    }

    #[test]
    fn specular_exponents_clamp_instead_of_dropping_the_primitive() {
        let exponent = |value| chrome_attribute("feSpecularLighting", "specularExponent", value);
        assert_eq!(exponent("0.5"), Attribute::Replace("1".into()));
        assert_eq!(exponent("200"), Attribute::Replace("128".into()));
        // Unparsable exponents take the initial value 1, as usvg's default.
        assert_eq!(exponent("x"), Attribute::Omit);
    }
}
