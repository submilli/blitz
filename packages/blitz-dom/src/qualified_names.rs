//! Shared qualified-name extraction for the existing scripting binding contract.
use crate::{
    QualName,
    dom_api::{DomError, is_valid_element_name},
    ns,
};

/// Validate namespace constraints and extract a qualified name without changing
/// case. Preserves the existing binding's stricter single-colon name policy.
/// Current DOM contextual validation and lossless non-scalar name storage are
/// separate compatibility work: <https://dom.spec.whatwg.org/#validate-and-extract>.
pub fn validate_qualified_name(namespace: &str, qualified: &str) -> Result<QualName, DomError> {
    if !is_valid_element_name(qualified) {
        return Err(DomError::InvalidCharacter);
    }
    let (prefix, local) = qualified
        .split_once(':')
        .map_or((None, qualified), |(prefix, local)| (Some(prefix), local));
    if prefix.is_some_and(str::is_empty) || local.is_empty() || local.contains(':') {
        return Err(DomError::InvalidCharacter);
    }
    if (prefix.is_some() && namespace.is_empty())
        || (prefix == Some("xml") && namespace != ns!(xml).as_ref())
        || ((qualified == "xmlns" || prefix == Some("xmlns")) && namespace != ns!(xmlns).as_ref())
        || (namespace == ns!(xmlns).as_ref() && qualified != "xmlns" && prefix != Some("xmlns"))
    {
        return Err(DomError::Namespace);
    }
    Ok(QualName::new(
        prefix.map(Into::into),
        namespace.into(),
        local.into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extraction_preserves_case_and_expanded_name_parts() {
        for (namespace, qualified, prefix, local) in [
            ("", "plain", None, "plain"),
            ("urn:Mixed", "p:Mixed", Some("p"), "Mixed"),
            (ns!(svg).as_ref(), "linearGradient", None, "linearGradient"),
            (ns!(xml).as_ref(), "xml:lang", Some("xml"), "lang"),
            (ns!(xmlns).as_ref(), "xmlns", None, "xmlns"),
            (ns!(xmlns).as_ref(), "xmlns:x", Some("xmlns"), "x"),
            ("urn:test", "XML:Name", Some("XML"), "Name"),
        ] {
            let name = validate_qualified_name(namespace, qualified).unwrap();
            assert_eq!(name.ns.as_ref(), namespace);
            assert_eq!(name.prefix.as_deref(), prefix);
            assert_eq!(name.local.as_ref(), local);
        }
    }

    #[test]
    fn invalid_names_precede_namespace_constraint_errors() {
        for name in [
            "", "a b", ":a", "a:", "a:b:c", "0a", "a/b", "a<b", "a>b", "a\0b",
        ] {
            for namespace in ["", "urn:test", ns!(xmlns).as_ref()] {
                assert_eq!(
                    validate_qualified_name(namespace, name),
                    Err(DomError::InvalidCharacter),
                    "{namespace:?} {name:?}"
                );
            }
        }
    }

    #[test]
    fn reserved_namespaces_and_prefixes_are_checked_independently() {
        for (namespace, name) in [
            ("", "p:name"),
            ("urn:test", "xml:name"),
            ("urn:test", "xmlns"),
            ("urn:test", "xmlns:name"),
            (ns!(xmlns).as_ref(), "name"),
            (ns!(xmlns).as_ref(), "p:name"),
        ] {
            assert_eq!(
                validate_qualified_name(namespace, name),
                Err(DomError::Namespace),
                "{namespace:?} {name:?}"
            );
        }
        assert_eq!(DomError::Namespace.name(), "NamespaceError");
    }

    #[test]
    fn preserved_binding_policy_is_explicit_for_non_xml_names() {
        for name in ["a=b", "a?b", "_!", "é?", "😀", "a:1"] {
            assert!(
                validate_qualified_name("urn:test", name).is_ok(),
                "{name:?}"
            );
        }
        assert!(validate_qualified_name("urn:test", "0:a").is_err());
        assert!(validate_qualified_name("urn:test", "a:b:c").is_err());
    }
}
