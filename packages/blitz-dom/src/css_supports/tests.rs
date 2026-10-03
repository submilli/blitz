use crate::{BaseDocument, DocumentConfig, MAX_CSS_NESTING};

#[test]
fn conditions_use_author_style_grammar() {
    let doc = BaseDocument::new(DocumentConfig::default());
    for condition in [
        "not (display: foo)",
        "((display: grid) or (color: invalid)) and (not (display: foo))",
        "selector(a > b)",
        "(--custom:)",
        "(display: block !important)",
        "display: block",
        "(display: block",
        "not unknown(foo)",
    ] {
        assert!(doc.css_supports_condition(condition), "{condition}");
    }
    for condition in [
        "(display: block) and (color: red) or (color: blue)",
        "not not (display: foo)",
        "selector(a, b)",
        "selector(:made-up)",
        "(--: red)",
        "(--x: ;)",
        "(display: block))",
        "(display: block) trailing",
        "",
    ] {
        assert!(!doc.css_supports_condition(condition), "{condition}");
    }
}

#[test]
fn property_values_are_literal_and_exclude_priority() {
    let doc = BaseDocument::new(DocumentConfig::default());
    for (property, value) in [("DISPLAY", "block"), ("--x", "{}"), ("color", "var(--x)")] {
        assert!(doc.css_supports_property(property, value));
    }
    for (property, value) in [
        (" display", "block"),
        ("d\\69splay", "block"),
        ("display", "block !important"),
        ("display", "block; color: red"),
        ("--x", ""),
        ("--", "red"),
        ("--x", ";"),
        ("--x", "red !important"),
    ] {
        assert!(
            !doc.css_supports_property(property, value),
            "{property}: {value}"
        );
    }
}

#[test]
fn both_overloads_respect_css_admission() {
    let doc = BaseDocument::new(DocumentConfig::default());
    let nested = |depth| format!("{}display: block{}", "(".repeat(depth), ")".repeat(depth));
    assert!(doc.css_supports_condition(&nested(MAX_CSS_NESTING)));
    assert!(!doc.css_supports_condition(&nested(MAX_CSS_NESTING + 1)));
    let value = |depth| format!("{}x{}", "(".repeat(depth), ")".repeat(depth));
    assert!(doc.css_supports_property("--x", &value(MAX_CSS_NESTING)));
    assert!(!doc.css_supports_property("--x", &value(MAX_CSS_NESTING + 1)));
    assert!(doc.css_supports_condition("(display: block)"));
}
