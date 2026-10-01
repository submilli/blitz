//! Text-type sanitization and linear-time email/URL constraints.
use super::ValidityState;
use crate::{DomString, ElementData};
use markup5ever::local_name;

/// User-entered URL whitespace remains visible until the UA commits the edit.
pub(crate) fn api_text(element: &ElementData, value: &DomString) -> DomString {
    if element.form_state.last_change_by_user
        && &*element.name.local == "input"
        && element
            .attr_dom(local_name!("type"))
            .map(crate::DomString::as_str_lossy)
            .is_some_and(|t| t.eq_ignore_ascii_case("number"))
    {
        return super::number::user_value(value.as_str_lossy())
            .unwrap_or_default()
            .into();
    }
    if element.form_state.last_change_by_user
        && &*element.name.local == "input"
        && element
            .attr_dom(local_name!("type"))
            .map(crate::DomString::as_str_lossy)
            .is_some_and(|t| t.eq_ignore_ascii_case("url"))
    {
        return strip_newlines(value);
    }
    sanitize_text(element, value)
}

pub(crate) fn sanitize_text(element: &ElementData, value: &DomString) -> DomString {
    if &*element.name.local == "textarea" {
        let units = value.to_utf16();
        let mut out = Vec::with_capacity(units.len());
        let mut previous_cr = false;
        for &unit in units.iter() {
            if unit != 10 || !previous_cr {
                out.push(if unit == 13 { 10 } else { unit });
            }
            previous_cr = unit == 13;
        }
        return DomString::from_utf16(out);
    }
    let kind = element
        .attr_dom(local_name!("type"))
        .map(DomString::as_str_lossy)
        .unwrap_or("")
        .to_ascii_lowercase();
    if super::number::applies(&kind) {
        return super::number::sanitize(element, &kind, value.as_str_lossy()).into();
    }
    if !is_text_type(&kind) {
        return value.clone();
    }
    let value = strip_newlines(value);
    if matches!(kind.as_str(), "email" | "url") {
        let units = value.to_utf16();
        let trimmed = |token: &[u16]| {
            let start = token
                .iter()
                .position(|u| !matches!(u, 9 | 10 | 12 | 13 | 32))
                .unwrap_or(token.len());
            let end = token
                .iter()
                .rposition(|u| !matches!(u, 9 | 10 | 12 | 13 | 32))
                .map_or(start, |n| n + 1);
            start..end
        };
        if kind == "email" && element.attr_dom(local_name!("multiple")).is_some() {
            let mut normalized = Vec::with_capacity(units.len());
            for (index, token) in units.split(|&u| u == 44).enumerate() {
                if index != 0 {
                    normalized.push(44);
                }
                normalized.extend_from_slice(&token[trimmed(token)]);
            }
            return DomString::from_utf16(normalized);
        }
        return DomString::from_utf16(units[trimmed(&units)].to_vec());
    }
    value
}

fn strip_newlines(value: &DomString) -> DomString {
    DomString::from_utf16(
        value
            .to_utf16()
            .iter()
            .copied()
            .filter(|u| !matches!(u, 10 | 13))
            .collect(),
    )
}

fn trim_spaces(value: &str) -> &str {
    value.trim_matches(['\t', '\n', '\x0c', '\r', ' '])
}

pub(super) fn is_text_type(kind: &str) -> bool {
    !matches!(
        kind,
        "hidden"
            | "date"
            | "month"
            | "week"
            | "time"
            | "datetime-local"
            | "number"
            | "range"
            | "color"
            | "checkbox"
            | "radio"
            | "file"
            | "submit"
            | "image"
            | "reset"
            | "button"
    )
}

pub(super) fn text_constraints(element: &ElementData, value: &str, state: &mut ValidityState) {
    let kind = element
        .attr_dom(local_name!("type"))
        .map(crate::DomString::as_str_lossy)
        .unwrap_or("")
        .to_ascii_lowercase();
    let input = &*element.name.local == "input";
    if input && !value.is_empty() {
        state.type_mismatch = match kind.as_str() {
            "email" if element.attr_dom(local_name!("multiple")).is_some() => {
                value.split(',').any(|part| !valid_email(trim_spaces(part)))
            }
            "email" => !valid_email(value),
            "url" => url::Url::parse(value).is_err(),
            _ => false,
        };
    }
    if !element.form_state.last_change_by_user
        || value.is_empty()
        || !(input && is_text_type(&kind) || &*element.name.local == "textarea")
    {
        return;
    }
    let length = value.encode_utf16().count() as u64;
    let limit = |name| {
        element
            .attr(name)
            .and_then(|raw| style::servo::attr::parse_unsigned_integer(raw.chars()).ok())
    };
    state.too_long = limit(local_name!("maxlength")).is_some_and(|n| length > u64::from(n));
    state.too_short = limit(local_name!("minlength")).is_some_and(|n| length < u64::from(n));
}

/// The HTML email grammar intentionally differs from RFC mailbox syntax.
fn valid_email(value: &str) -> bool {
    let Some((local, domain)) = value.split_once('@') else {
        return false;
    };
    if local.is_empty()
        || !local
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".!#$%&'*+-/=?^_`{|}~".contains(&b))
    {
        return false;
    }
    domain.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && label.as_bytes()[0].is_ascii_alphanumeric()
            && label.as_bytes()[label.len() - 1].is_ascii_alphanumeric()
            && label
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    })
}

fn effective_type(value: &str) -> &str {
    match value {
        "hidden" | "text" | "search" | "tel" | "url" | "email" | "password" | "date" | "month"
        | "week" | "time" | "datetime-local" | "number" | "range" | "color" | "checkbox"
        | "radio" | "file" | "submit" | "image" | "reset" | "button" => value,
        _ => "text",
    }
}

impl crate::DocumentMutator<'_> {
    /// Setting the same effective type must retain incomplete user editor state.
    pub(crate) fn input_sanitization_needed(
        &self,
        id: crate::NodeId,
        name: &markup5ever::QualName,
        value: Option<&str>,
    ) -> bool {
        if name.ns != markup5ever::ns!() || name.local != local_name!("type") {
            return true;
        }
        let old = self
            .doc
            .null_attribute(id, "type")
            .map(|a| a.value.as_str_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        let new = value.unwrap_or("").to_ascii_lowercase();
        effective_type(&old) != effective_type(&new)
    }

    /// Capture the previous type's effective value before changing its sanitizer.
    pub(crate) fn value_before_sanitizer_change(
        &self,
        id: crate::NodeId,
        name: &markup5ever::QualName,
    ) -> Option<(DomString, bool)> {
        (name.ns == markup5ever::ns!()
            && matches!(&*name.local, "type" | "multiple" | "min" | "max" | "step"))
        .then(|| {
            self.doc.form_value_dom(id).map(|value| {
                (
                    value,
                    self.doc.nodes[id]
                        .element_data()
                        .is_some_and(crate::dom_api::is_text_control),
                )
            })
        })
        .flatten()
    }

    /// Attribute/default changes update native current state without setting the dirty flag.
    pub(crate) fn sanitize_changed_input_value(
        &mut self,
        id: crate::NodeId,
        name: &markup5ever::QualName,
        previous: Option<(DomString, bool)>,
    ) {
        let Some(e) = self.doc.nodes[id].element_data() else {
            return;
        };
        if name.ns != markup5ever::ns!()
            || e.name.ns != markup5ever::ns!(html)
            || &*e.name.local != "input"
        {
            return;
        }
        let old_value_mode = previous.as_ref().is_some_and(|(_, value_mode)| *value_mode);
        if !crate::dom_api::is_text_control(e) {
            if &*name.local == "type" && old_value_mode {
                let value = previous.map(|p| p.0).unwrap_or_default();
                let reflect = e
                    .attr(local_name!("type"))
                    .is_some_and(|t| !t.eq_ignore_ascii_case("file"));
                if reflect && !value.is_empty() {
                    self.set_attribute(
                        id,
                        markup5ever::QualName::new(None, markup5ever::ns!(), "value".into()),
                        &value,
                    );
                }
                if let Some(e) = self.doc.nodes[id].element_data_mut() {
                    e.form_state.value = None;
                    e.form_state.value_dirty = false;
                    e.form_state.last_change_by_user = false;
                }
            }
            return;
        }
        let dirty = e.form_state.value_dirty && (&*name.local != "type" || old_value_mode);
        let previous = previous.map(|p| p.0);
        let by_user = e.form_state.last_change_by_user;
        let value = match &*name.local {
            "type" if !old_value_mode => e
                .attr_dom(local_name!("value"))
                .cloned()
                .unwrap_or_default(),
            "type" => previous.unwrap_or_default(),
            "multiple"
                if e.attr_dom(local_name!("type"))
                    .map(crate::DomString::as_str_lossy)
                    .is_some_and(|t| t.eq_ignore_ascii_case("email")) =>
            {
                previous.unwrap_or_default()
            }
            "min" | "max" | "step"
                if e.attr_dom(local_name!("type"))
                    .map(crate::DomString::as_str_lossy)
                    .is_some_and(|t| t.eq_ignore_ascii_case("range")) =>
            {
                previous.unwrap_or_default()
            }
            "value" if !dirty => e
                .attr_dom(local_name!("value"))
                .cloned()
                .unwrap_or_default(),
            _ => return,
        };
        let sanitized = sanitize_text(e, &value);
        self.set_form_value_dom(id, &sanitized);
        let numeric = self.doc.nodes[id].element_data().is_some_and(|e| {
            e.attr_dom(local_name!("type"))
                .map(crate::DomString::as_str_lossy)
                .is_some_and(|t| t.eq_ignore_ascii_case("number"))
        });
        if let Some(e) = self.doc.nodes[id].element_data_mut() {
            if let Some(editor) = e.text_input_data_mut() {
                editor.numeric = numeric;
            }
            e.form_state.value_dirty = dirty;
            e.form_state.last_change_by_user = by_user && value == sanitized;
        }
    }
}
