//! HTML Unicode-v patterns share one budget across a form and email tokens.
use super::ValidationError;
use crate::{ElementData, ns};
use markup5ever::local_name;
use regress_bounded::{Budget, Pattern, PatternError};

/// Admit the raw input before form_value copies and sanitizes it. A pattern
/// operation includes its normalization prelude, even when syntax is invalid.
pub(super) fn admit_input(element: &ElementData, budget: &Budget) -> Result<(), ValidationError> {
    if &*element.name.local != "input" || element.attr_dom(local_name!("pattern")).is_none() {
        return Ok(());
    }
    let kind = element.attr_dom(local_name!("type"));
    let kind_bound = kind.map_or(0, crate::DomString::utf16_len_bound);
    budget
        .charge(kind_bound, kind_bound.saturating_mul(6))
        .map_err(|_| ValidationError)?;
    let kind = kind.map_or("", crate::DomString::as_str_lossy);
    if !super::text::is_text_type(&kind.to_ascii_lowercase()) {
        return Ok(());
    }
    let raw_bound = element
        .text_input_data()
        .map(|i| i.editor.raw_text().len())
        .or_else(|| {
            element
                .form_state
                .value
                .as_ref()
                .map(|v| v.utf16_len_bound().saturating_mul(3))
        })
        .unwrap_or_else(|| {
            element
                .attr_dom(local_name!("value"))
                .map_or(0, |v| v.utf16_len_bound().saturating_mul(3))
        });
    // Covers two value snapshots (required + constraints), sanitization,
    // UTF-8/UTF-16 projections and linear email/URL checks before matching.
    budget
        .charge(raw_bound.saturating_mul(8), raw_bound.saturating_mul(32))
        .map_err(|_| ValidationError)
}

pub(super) fn mismatch(
    element: &ElementData,
    value: &crate::DomString,
    budget: &Budget,
) -> Result<bool, ValidationError> {
    if value.is_empty() || element.name.ns != ns!(html) || &*element.name.local != "input" {
        return Ok(false);
    }
    let kind = element
        .attr_dom(local_name!("type"))
        .map(crate::DomString::as_str_lossy)
        .unwrap_or("")
        .to_ascii_lowercase();
    if !super::text::is_text_type(&kind) {
        return Ok(false);
    }
    let Some(source) = element.attr_dom(local_name!("pattern")) else {
        return Ok(false);
    };
    let bound = source.utf16_len_bound();
    // Each code unit needs at most three UTF-8 bytes. Reject before conversion
    // when even this representation-independent upper bound cannot fit.
    if bound > 4096 * 3 {
        return Err(ValidationError);
    }
    budget
        .charge(bound, bound.saturating_mul(8))
        .map_err(|_| ValidationError)?;
    let source = source.to_utf16();
    if source.len() > 4096 {
        return Err(ValidationError);
    }
    let pattern = match Pattern::compile(&source, budget) {
        Ok(pattern) => pattern,
        Err(PatternError::Syntax) => return Ok(false),
        Err(PatternError::Resource(_)) => return Err(ValidationError),
    };
    let units = value.to_utf16();
    if kind == "email" && element.attr_dom(local_name!("multiple")).is_some() {
        for token in units.split(|&u| u == 44).filter(|token| !token.is_empty()) {
            if !matches(&pattern, token, budget)? {
                return Ok(true);
            }
        }
        Ok(false)
    } else {
        Ok(!matches(&pattern, &units, budget)?)
    }
}

fn matches(pattern: &Pattern, value: &[u16], budget: &Budget) -> Result<bool, ValidationError> {
    if value.len() > 1_048_576 {
        return Err(ValidationError);
    }
    pattern
        .is_full_match(value, budget)
        .map_err(|_| ValidationError)
}
