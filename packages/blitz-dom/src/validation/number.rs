//! Numeric/calendar value sanitization and min/max/step constraints.
use super::{ValidityState, calendar};
use crate::ElementData;
use markup5ever::local_name;

/// The HTML floating-point grammar excludes whitespace, a leading plus,
/// trailing decimal points, non-finite values, and incomplete exponents.
pub(super) fn float(raw: &str) -> Option<f64> {
    let mantissa = raw.strip_prefix('-').unwrap_or(raw);
    let mut parts = mantissa.split(['e', 'E']);
    let mantissa = parts.next()?;
    if let Some(exponent) = parts.next() {
        let exponent = exponent.strip_prefix(['+', '-']).unwrap_or(exponent);
        if exponent.is_empty() || !exponent.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
    }
    if parts.next().is_some() {
        return None;
    }
    let (whole, fraction) = mantissa
        .split_once('.')
        .map_or((mantissa, None), |(w, f)| (w, Some(f)));
    if whole.is_empty() && fraction.is_none()
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || fraction.is_some_and(|f| f.is_empty() || !f.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    let value: f64 = raw.parse().ok()?;
    value.is_finite().then_some(value)
}

/// The number editor accepts a leading plus and an unfinished decimal point
/// once digits are present, while the programmatic setter uses strict syntax.
pub(super) fn user_value(raw: &str) -> Option<String> {
    let raw = if let Some(unsigned) = raw.strip_prefix('+') {
        if unsigned.starts_with('-') {
            return None;
        }
        unsigned
    } else {
        raw
    };
    let (mantissa, exponent) = raw
        .find(['e', 'E'])
        .map_or((raw, ""), |at| raw.split_at(at));
    let mantissa = mantissa.strip_suffix('.').unwrap_or(mantissa);
    let normalized = format!("{mantissa}{exponent}");
    float(&normalized).map(|_| {
        if exponent.is_empty() {
            normalized
        } else {
            raw.to_owned()
        }
    })
}

fn parse(kind: &str, value: &str) -> Option<f64> {
    if matches!(kind, "number" | "range") {
        float(value)
    } else {
        calendar::parse(kind, value)
    }
}

pub(super) fn applies(kind: &str) -> bool {
    matches!(
        kind,
        "number" | "range" | "date" | "month" | "week" | "time" | "datetime-local"
    )
}

struct Limits {
    min: Option<f64>,
    max: Option<f64>,
    step: Option<f64>,
    base: f64,
}

impl Limits {
    fn for_element(e: &ElementData, kind: &str) -> Self {
        let min = e.attr(local_name!("min")).and_then(|v| parse(kind, v));
        let max = e.attr(local_name!("max")).and_then(|v| parse(kind, v));
        let (scale, default) = match kind {
            "date" => (86_400_000.0, 1.0),
            "week" => (604_800_000.0, 1.0),
            "time" | "datetime-local" => (1000.0, 60.0),
            _ => (1.0, 1.0),
        };
        let raw_step = e.attr(local_name!("step")).unwrap_or("");
        let step = if raw_step.eq_ignore_ascii_case("any") {
            None
        } else {
            Some(float(raw_step).filter(|n| *n > 0.0).unwrap_or(default) * scale)
        };
        let base = min
            .or_else(|| e.attr(local_name!("value")).and_then(|v| parse(kind, v)))
            .unwrap_or(if kind == "week" { -259_200_000.0 } else { 0.0 });
        Self {
            min,
            max,
            step,
            base,
        }
    }
}

/// Avoid overflow in value-base, including finite values near opposite extremes.
fn step_remainder(value: f64, base: f64, step: f64) -> f64 {
    let (a, b) = (value.rem_euclid(step), base.rem_euclid(step));
    if a >= b { a - b } else { step - (b - a) }
}

pub(super) fn sanitize(e: &ElementData, kind: &str, value: &str) -> String {
    let number = parse(kind, value);
    if kind == "range" {
        let limits = Limits::for_element(e, kind);
        let min = limits.min.unwrap_or(0.0);
        let max = limits.max.unwrap_or(100.0).max(min);
        let mut number = number.unwrap_or(min / 2.0 + max / 2.0).clamp(min, max);
        if let Some(step) = limits.step.filter(|n| n.is_finite()) {
            let remainder = step_remainder(number, limits.base, step);
            let lower = number - remainder;
            let upper = lower + step;
            number = if lower < min || upper <= max && remainder >= step / 2.0 {
                upper
            } else {
                lower
            };
            number = number.clamp(min, max);
        }
        return if number == 0.0 {
            "0".to_owned()
        } else {
            ryu_js::Buffer::new().format(number).to_owned()
        };
    }
    if number.is_none() {
        return String::new();
    }
    if kind == "datetime-local" {
        calendar::normalize_datetime(value)
    } else {
        value.to_owned()
    }
}

pub(super) fn constraints(
    e: &ElementData,
    kind: &str,
    value: &str,
    raw: &str,
    state: &mut ValidityState,
) {
    if !applies(kind) {
        return;
    }
    state.bad_input = e.form_state.last_change_by_user
        && !raw.is_empty()
        && if kind == "number" {
            user_value(raw).is_none()
        } else {
            parse(kind, raw).is_none()
        };
    if let Some(editor) = e.form_state.calendar.as_ref() {
        state.bad_input = editor.bad_input();
    }
    let parsed = if kind == "number" && e.form_state.last_change_by_user {
        user_value(value).and_then(|v| v.parse::<f64>().ok())
    } else {
        parse(kind, value)
    };
    let Some(value) = parsed else {
        return;
    };
    let limits = Limits::for_element(e, kind);
    state.range_underflow = kind != "range" && limits.min.is_some_and(|min| value < min);
    state.range_overflow = kind != "range" && limits.max.is_some_and(|max| value > max);
    // Time's periodic domain allows an overnight range, excluding only its gap.
    if kind == "time"
        && let (Some(min), Some(max)) = (limits.min, limits.max)
        && max < min
    {
        let outside = value < min && value > max;
        state.range_underflow = outside;
        state.range_overflow = outside;
    }
    if let Some(step) = limits.step {
        if step.is_finite() {
            let remainder = step_remainder(value, limits.base, step);
            let tolerance = step / 16_777_216.0;
            state.step_mismatch = remainder > tolerance && step - remainder > tolerance;
        } else {
            state.step_mismatch = value != limits.base;
        }
    }
}
