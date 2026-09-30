//! Hostile CSS is rejected before recursive Stylo parsing on an embedder stack.
mod common;

use std::sync::{Arc, Mutex};

use blitz_dom::{BaseDocument, CssomError, DocumentConfig, MAX_CSS_NESTING};
use blitz_html::DocumentHtmlParser;
use blitz_traits::net::{Bytes, NetHandler, NetProvider, Request};
use common::{parse, q};

fn on_script_stack(f: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(8 << 20)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap();
}

fn hostile_values() -> Vec<String> {
    [":is(", "[", "{", "(", "calc("]
        .into_iter()
        .map(|open| open.repeat(100_000))
        .chain([
            format!("{}p{}", ":is(".repeat(100_000), ")".repeat(100_000)),
            format!(
                "{}{}x{}",
                "(".repeat(100_000),
                ")".repeat(99_872),
                ")".repeat(128)
            ),
        ])
        .collect()
}

#[test]
fn style_markup_and_replacement_reject_deep_blocks() {
    on_script_stack(|| {
        for css in hostile_values() {
            let mut doc = parse(&format!("<style id=s>{css}</style><div id=d></div>"));
            let sheet = q(&doc, "#s");
            assert_eq!(doc.stylesheet_rule_count(sheet, &[]), Some(0));
            let target = q(&doc, "#d");
            doc.mutate()
                .set_inner_html(target, &format!("<style>{css}</style>"));
            let replacement = doc.query_selector_in(target, "style").unwrap().unwrap();
            assert_eq!(doc.stylesheet_rule_count(replacement, &[]), Some(0));
            doc.resolve(0.0);
        }
    });
}

#[test]
fn declarations_queries_and_transforms_reject_deep_blocks() {
    on_script_stack(|| {
        let mut doc = parse("<div id=d style='color:red'></div>");
        let target = q(&doc, "#d");
        for value in hostile_values() {
            assert!(!doc.css_declaration_is_valid("width", &value));
            assert!(!doc.css_declaration_is_valid("--custom", &value));
            assert_eq!(
                doc.style_attr_set_property("color:red", "width", &value, false),
                None
            );
            let declarations = format!("--custom:{value}");
            assert!(doc.style_attr_serialize(&declarations).is_empty());
            doc.mutate()
                .set_attribute_by_name(target, "style", &declarations)
                .unwrap();
            doc.resolve(0.0);
            assert!(!doc.css_supports_condition(&value));
            assert!(!doc.evaluate_media_query(&value));
            assert_eq!(doc.serialize_media_query(&value), "not all");
            assert!(blitz_dom::parse_transform_matrix(&value).is_none());
            assert!(matches!(
                doc.register_custom_property("--test", "*", false, Some(&value)),
                blitz_dom::RegisterCustomPropertyResult::InvalidInitialValue
            ));
        }
        assert!(doc.css_declaration_is_valid("width", "calc(1px + 2px)"));
        assert!(doc.css_supports_condition("(display: grid)"));
        assert!(doc.evaluate_media_query("(min-width: 1px)"));
    });
}

#[test]
fn cssom_rejects_deep_rules_selectors_and_declarations() {
    on_script_stack(|| {
        let mut doc = parse("<style id=s>p { color:red }</style><p>text</p>");
        let owner = q(&doc, "#s");
        let sheet = doc.retain_stylesheet(owner).unwrap();
        for value in hostile_values() {
            assert_eq!(
                doc.retained_stylesheet_insert_rule(&sheet, &[], &value, 1),
                Err(CssomError::Syntax)
            );
            doc.stylesheet_rule_set_selector_text(owner, &[0], &value)
                .unwrap();
            doc.stylesheet_rule_style_set_property(owner, &[0], "color", &value, false)
                .unwrap();
            assert_eq!(
                doc.stylesheet_rule_style_get_property(owner, &[0], "color"),
                Some(("red".into(), false))
            );
            doc.stylesheet_rule_style_set_css_text(owner, &[0], &format!("--custom:{value}"))
                .unwrap();
            assert_eq!(
                doc.stylesheet_rule_style_css_text(owner, &[0]),
                Some(String::new())
            );
            doc.stylesheet_rule_style_set_css_text(owner, &[0], "color:red")
                .unwrap();
        }
        doc.resolve(0.0);
        assert_eq!(
            doc.resolved_style_value(q(&doc, "p"), "color"),
            "rgb(255, 0, 0)"
        );
    });
}

#[test]
fn repeated_cssom_insertion_cannot_bypass_sheet_depth() {
    on_script_stack(|| {
        let mut doc = parse("<style id=s></style>");
        let owner = q(&doc, "#s");
        let mut path = Vec::new();
        for _ in 0..MAX_CSS_NESTING {
            doc.stylesheet_insert_rule(owner, &path, "@media all {}", 0)
                .unwrap();
            path.push(0);
        }
        assert_eq!(
            doc.stylesheet_insert_rule(owner, &path, "@media all {}", 0),
            Err(CssomError::Syntax)
        );
    });
}

#[test]
fn cssom_mutations_respect_the_enclosing_rule_depth() {
    on_script_stack(|| {
        let mut doc = parse("<style id=s></style>");
        let owner = q(&doc, "#s");
        let mut path = Vec::new();
        for _ in 0..MAX_CSS_NESTING - 1 {
            doc.stylesheet_insert_rule(owner, &path, "@media all {}", 0)
                .unwrap();
            path.push(0);
        }
        doc.stylesheet_insert_rule(owner, &path, "p {}", 0).unwrap();
        path.push(0);

        let deep = "calc(".repeat(MAX_CSS_NESTING);
        doc.stylesheet_rule_style_set_property(owner, &path, "--x", &deep, false)
            .unwrap();
        assert_eq!(
            doc.stylesheet_rule_style_get_property(owner, &path, "--x"),
            Some((String::new(), false))
        );
        doc.stylesheet_rule_set_selector_text(
            owner,
            &path,
            &format!("{}p", ":is(".repeat(MAX_CSS_NESTING)),
        )
        .unwrap();
        assert_eq!(
            doc.stylesheet_rule_info(owner, &path).unwrap().css_text,
            "p { }"
        );
        doc.stylesheet_rule_style_set_css_text(owner, &path, &format!("--x:{deep}"))
            .unwrap();
        assert_eq!(
            doc.stylesheet_rule_style_css_text(owner, &path),
            Some(String::new())
        );
    });
}

#[test]
fn custom_property_substitution_cannot_synthesize_unbounded_nesting() {
    on_script_stack(|| {
        use std::fmt::Write;

        let mut markup = String::from("<style>#d{--v0:1px;");
        for index in 1..5_000 {
            write!(markup, "--v{index}:calc(var(--v{}));", index - 1).unwrap();
        }
        markup.push_str(
            "width:var(--v4999)}</style><div id=d></div><p id=alive style='color:red'></p>",
        );
        let mut doc = parse(&markup);
        doc.resolve(0.0);
        assert_ne!(doc.resolved_style_value(q(&doc, "#d"), "width"), "1px");
        assert_eq!(
            doc.resolved_style_value(q(&doc, "#alive"), "color"),
            "rgb(255, 0, 0)"
        );
    });
}

#[test]
fn custom_property_substitution_preserves_values_at_the_limits() {
    on_script_stack(|| {
        use std::fmt::Write;

        let mut aliases = String::from("<style>#aliases{--base:10px;--v0:var(--base);");
        for index in 1..32 {
            write!(aliases, "--v{index}:var(--v{});", index - 1).unwrap();
        }
        aliases.push_str("width:var(--v31);height:var(--base)}</style><div id=aliases></div>");
        let mut aliases = parse(&aliases);
        aliases.resolve(0.0);
        assert_eq!(
            aliases.resolved_style_value(q(&aliases, "#aliases"), "width"),
            "10px"
        );
        assert_eq!(
            aliases.resolved_style_value(q(&aliases, "#aliases"), "height"),
            "10px"
        );

        let base = nested_calc("1px", 64);
        let within_limit = nested_calc("var(--base)", 64);
        let mut accepted = parse(&format!(
            "<style>#accepted{{--base:{base};--value:{within_limit};width:var(--value)}}</style>\
             <div id=accepted></div>"
        ));
        accepted.resolve(0.0);
        assert_eq!(
            accepted.resolved_style_value(q(&accepted, "#accepted"), "width"),
            "1px"
        );

        let over_limit = nested_calc("var(--base)", 65);
        let mut rejected = parse(&format!(
            "<style>#rejected{{--base:{base};--value:{over_limit};width:var(--value)}}</style>\
             <div id=rejected></div>"
        ));
        rejected.resolve(0.0);
        assert_ne!(
            rejected.resolved_style_value(q(&rejected, "#rejected"), "width"),
            "1px"
        );
    });
}

#[test]
fn typed_attribute_values_are_bounded_before_recursive_parsing() {
    on_script_stack(|| {
        use std::fmt::Write;

        let mut ordinary = parse(
            "<div id=ordinary data-width='1px' \
             style='width:attr(data-width type(<length>))'></div>",
        );
        ordinary.resolve(0.0);
        assert_eq!(
            ordinary.resolved_style_value(q(&ordinary, "#ordinary"), "width"),
            "1px"
        );

        let deep = "calc(".repeat(100_000);
        let mut hostile = parse(&format!(
            "<div id=hostile data-width='{deep}' \
             style='width:attr(data-width type(<length>))'></div>\
             <p id=alive style='color:red'></p>"
        ));
        hostile.resolve(0.0);
        assert_eq!(
            hostile.resolved_style_value(q(&hostile, "#alive"), "color"),
            "rgb(255, 0, 0)"
        );

        for links in 30..=34 {
            let mut markup = String::from("<div id=subject");
            for index in 0..=links {
                let value = if index == links {
                    "1px".to_string()
                } else if index == 31 {
                    format!(
                        "calc(attr(data-a{} type(<length>)) + attr(data-a{} type(<length>)))",
                        index + 1,
                        index + 1
                    )
                } else {
                    format!("attr(data-a{} type(<length>))", index + 1)
                };
                write!(markup, " data-a{index}='{value}'").unwrap();
            }
            markup.push_str(
                " style='width:attr(data-a0 type(<length>))'></div>\
                 <p id=alive style='color:red'></p>",
            );
            let mut chained = parse(&markup);
            chained.resolve(0.0);
            assert_eq!(
                chained.resolved_style_value(q(&chained, "#alive"), "color"),
                "rgb(255, 0, 0)"
            );
            if links == 30 {
                assert_eq!(
                    chained.resolved_style_value(q(&chained, "#subject"), "width"),
                    "1px"
                );
            }
        }
    });
}

fn nested_calc(inner: &str, depth: usize) -> String {
    format!("{}{}{}", "calc(".repeat(depth), inner, ")".repeat(depth))
}

struct Replay {
    css: String,
    calls: Mutex<Vec<String>>,
}
impl NetProvider for Replay {
    fn fetch(&self, _: usize, request: Request, handler: Box<dyn NetHandler>) {
        self.calls
            .lock()
            .unwrap()
            .push(request.url.path().to_string());
        handler.bytes(request.url.to_string(), Bytes::from(self.css.clone()));
    }
}

#[test]
fn linked_and_imported_sheets_use_the_same_admission() {
    on_script_stack(|| {
        for css in hostile_values() {
            let replay = Arc::new(Replay {
                css: format!("p {{color:red}} {css}"),
                calls: Mutex::new(Vec::new()),
            });
            let mut doc = BaseDocument::new(DocumentConfig {
                base_url: Some("https://example.test/".into()),
                net_provider: Some(replay.clone()),
                ..Default::default()
            });
            DocumentHtmlParser::parse_into_mutator(
                &mut doc.mutate(),
                "<link id=l rel=stylesheet href=linked.css><style id=s>@import 'imported.css';</style><p>alive</p>",
            );
            doc.resolve(0.0);
            doc.resolve(0.0);
            assert_eq!(
                *replay.calls.lock().unwrap(),
                ["/linked.css", "/imported.css"]
            );
            assert_eq!(doc.stylesheet_rule_count(q(&doc, "#l"), &[]), Some(0));
            assert_eq!(doc.stylesheet_rule_count(q(&doc, "#s"), &[]), Some(1));
            assert_eq!(
                doc.resolved_style_value(q(&doc, "p"), "color"),
                "rgb(0, 0, 0)"
            );
        }
    });
}

#[test]
fn ordinary_nested_css_and_token_delimiters_still_apply() {
    on_script_stack(|| {
        let delimiters = "([{".repeat(1000);
        let mut doc = parse(&format!(
            "<style>/*{delimiters}*/ @media (min-width: 1px) {{\
            div {{ & > p:is(.selected) {{color:red; --text: '{delimiters}';}} }} }}</style>\
            <div><p class=selected>text</p></div>"
        ));
        doc.resolve(0.0);
        assert_eq!(
            doc.resolved_style_value(q(&doc, "p"), "color"),
            "rgb(255, 0, 0)"
        );
        assert!(doc.css_declaration_is_valid("--custom", &format!("url({})", "[{".repeat(1000))));
    });
}
