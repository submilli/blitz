//! Chrome's reading of filter primitive attributes where the engine's SVG parser
//! (usvg) applies different defaults. Values are rewritten before parsing, so the
//! parsed graph matches what Chrome renders.

/// How one attribute enters the snapshot markup.
#[derive(Debug, PartialEq)]
pub(crate) enum Attribute<'a> {
    Keep(&'a str),
    Replace(String),
    Omit,
}

/// A radius that rounds to zero: Chrome passes that axis through unchanged, while
/// usvg replaces zero radii with one.
const ZERO_RADIUS: &str = "0.000001";
/// An order whose kernel can never match a bounded `kernelMatrix`, so the parser
/// produces the transparent result Chrome renders for an invalid kernel.
const INVALID_ORDER: &str = "4096";

pub(crate) fn chrome_attribute<'a>(tag: &str, name: &str, value: &'a str) -> Attribute<'a> {
    match (tag, name) {
        // https://drafts.fxtf.org/filter-effects/#element-attrdef-femorphology-radius
        ("feMorphology", "radius") => {
            let [x, y] = number_optional_number(value).unwrap_or([0.0, 0.0]);
            let radius = |r: f32| {
                if r > 0.0 {
                    r.to_string()
                } else {
                    ZERO_RADIUS.to_owned()
                }
            };
            Attribute::Replace(format!("{} {}", radius(x), radius(y)))
        }
        // Chrome treats a zero divisor as absent and uses the kernel sum.
        ("feConvolveMatrix", "divisor") if number(value.trim()) == Some(0.0) => Attribute::Omit,
        ("feConvolveMatrix", "order") => match number_optional_number(value) {
            Some([x, y]) if x.trunc() < 1.0 || y.trunc() < 1.0 || x > 4096.0 || y > 4096.0 => {
                Attribute::Replace(INVALID_ORDER.to_owned())
            }
            _ => Attribute::Keep(value),
        },
        // Chrome clamps the exponent; usvg drops the whole primitive.
        ("feSpecularLighting", "specularExponent") => match number(value.trim()) {
            Some(exponent) => Attribute::Replace(exponent.clamp(1.0, 128.0).to_string()),
            None => Attribute::Keep(value),
        },
        _ => Attribute::Keep(value),
    }
}

/// One or two numbers separated by whitespace and/or a comma.
fn number_optional_number(value: &str) -> Option<[f32; 2]> {
    let mut numbers = value
        .split(|c: char| c == ',' || c.is_ascii_whitespace())
        .filter(|token| !token.is_empty());
    let first = number(numbers.next()?)?;
    let second = numbers.next().map_or(Some(first), number)?;
    numbers.next().is_none().then_some([first, second])
}

/// An SVG number: sign, digits, fraction and exponent, but no named values.
fn number(token: &str) -> Option<f32> {
    if token.is_empty()
        || !token
            .bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b'+' | b'-' | b'.' | b'e' | b'E'))
    {
        return None;
    }
    token.parse::<f32>().ok().filter(|n| n.is_finite())
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
    fn specular_exponents_clamp_instead_of_dropping_the_primitive() {
        let exponent = |value| chrome_attribute("feSpecularLighting", "specularExponent", value);
        assert_eq!(exponent("0.5"), Attribute::Replace("1".into()));
        assert_eq!(exponent("200"), Attribute::Replace("128".into()));
        assert_eq!(exponent("x"), Attribute::Keep("x"));
    }
}
