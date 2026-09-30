//! Admission bounds nested tree searches before queries or style matching.
mod common;

use std::time::{Duration, Instant};

use common::{parse, q};
use style::selector_parser::SelectorParser;
use style::stylesheets::UrlExtraData;

fn attack(kind: &str, levels: usize) -> String {
    let mut selector = "section i".to_owned();
    for _ in 0..levels {
        selector = format!(":{kind}({selector}) i");
    }
    format!(":{kind}({selector}) b")
}

fn chain(css: &str) -> blitz_dom::BaseDocument {
    // Exercise guarded paths even for syntax disabled in the embedder defaults.
    style_config::set_pref!("layout.css.has-selector.enabled", true);
    style_config::set_pref!("layout.css.nth-child-of.enabled", true);
    style_config::set_pref!("layout.css.at-scope.enabled", true);
    parse(&format!(
        "<style id=s>b {{color: red}} {css}</style><main>{}<b id=target></b>{}</main>",
        "<i>".repeat(100),
        "</i>".repeat(100)
    ))
}

#[test]
fn reported_query_finishes_well_under_a_second() {
    let doc = chain("");
    let selector = attack("is", 5);
    let start = Instant::now();
    assert!(doc.query_selector(&selector).is_err());
    assert!(start.elapsed() < Duration::from_millis(500));
    eprintln!("k=5 admission: {:?}", start.elapsed());
    assert_eq!(doc.query_selector("b").unwrap(), Some(q(&doc, "#target")));
}

#[test]
fn every_query_path_rejects_excessive_search_nesting() {
    let doc = chain("");
    let target = q(&doc, "#target");
    let root = q(&doc, "main");
    let url = UrlExtraData(style::servo_arc::Arc::new(doc.url().clone()));
    let mut selectors = ["is", "where", "not"].map(|kind| attack(kind, 5)).to_vec();
    selectors.extend([
        format!("main:has({})", attack("is", 5)),
        format!("b:nth-child(1 of {})", attack("is", 5)),
        format!("b:is(:where(:not({})))", attack("is", 5)),
        "b:nth-child(1 of :nth-child(1 of :nth-child(1 of b)))".into(),
        format!("{}b{}", ":is(".repeat(129), ")".repeat(129)),
    ]);
    for selector in selectors {
        assert!(doc.query_selector_all(&selector).is_err(), "{selector}");
        assert!(doc.query_selector_in(root, &selector).is_err());
        assert!(doc.query_selector_all_in(root, &selector).is_err());
        assert!(doc.matches_selector(target, &selector).is_err());
        assert!(doc.closest(target, &selector).is_err());
        let raw = SelectorParser::parse_author_origin_no_namespace(&selector, &url).unwrap();
        assert_eq!(doc.query_selector_raw(&raw), None);
        assert!(doc.query_selector_all_raw(&raw).is_empty());
        let node = doc.get_node(root).unwrap();
        assert_eq!(node.query_selector_raw(&raw), None);
        assert!(node.query_selector_all_raw(&raw).is_empty());
        let node = doc.get_node(target).unwrap();
        assert!(!node.matches_selector_raw(&raw));
        assert_eq!(node.closest_raw(&raw), None);
    }
}

#[test]
fn accepted_boundaries_preserve_matching_and_list_semantics() {
    let doc = chain("");
    let target = q(&doc, "#target");
    for selector in [
        ":is(main i) b",
        ":where(main i) b",
        ":not(section i) b",
        "main:has(i b) b",
        "b:nth-child(1 of b)",
    ] {
        assert_eq!(
            doc.query_selector(selector).unwrap(),
            Some(target),
            "{selector}"
        );
    }
    assert!(
        doc.query_selector(&format!("b, {}", attack("is", 5)))
            .is_err()
    );
    let at_limit = format!("b{}", ".x".repeat(4095));
    assert!(doc.try_parse_selector_list(&at_limit).is_ok());
    assert!(doc.try_parse_selector_list(&(at_limit + ".x")).is_err());
    // A shared visit budget applies across separate list branches, too.
    assert!(
        doc.try_parse_selector_list(&vec!["b"; 4096].join(","))
            .is_ok()
    );
    assert!(
        doc.try_parse_selector_list(&vec!["b"; 4097].join(","))
            .is_err()
    );
}

#[test]
fn stylesheets_cssom_and_nested_parents_cannot_bypass_admission() {
    for selector in [attack("is", 5), attack("where", 5), attack("not", 5)] {
        let mut doc = chain(&format!("{selector} {{color: blue}}"));
        let target = q(&doc, "#target");
        doc.resolve(0.0);
        assert_eq!(doc.resolved_style_value(target, "color"), "rgb(255, 0, 0)");
        let sheet = q(&doc, "#s");
        doc.stylesheet_rule_set_selector_text(sheet, &[1], "b")
            .unwrap();
        doc.resolve(0.0);
        assert_eq!(doc.resolved_style_value(target, "color"), "rgb(0, 0, 255)");
        doc.stylesheet_rule_set_selector_text(sheet, &[1], &selector)
            .unwrap();
        doc.resolve(0.0);
        assert_eq!(doc.resolved_style_value(target, "color"), "rgb(255, 0, 0)");
        doc.stylesheet_rule_set_selector_text(sheet, &[1], "b")
            .unwrap();
        doc.resolve(0.0);
        assert_eq!(doc.resolved_style_value(target, "color"), "rgb(0, 0, 255)");
    }
    // Each individual rule is shallow; replacing & synthesizes the attack.
    // Negating a rejected parent must not make its nested child match broadly.
    let mut doc = chain("main i { & i { & i { & i { & i { & b {color: blue} } } } } }");
    doc.resolve(0.0);
    assert_eq!(
        doc.resolved_style_value(q(&doc, "#target"), "color"),
        "rgb(255, 0, 0)"
    );
    let mut doc = chain(&format!("{} {{ :not(&) {{color:blue}} }}", attack("is", 5)));
    doc.resolve(0.0);
    assert_eq!(
        doc.resolved_style_value(q(&doc, "#target"), "color"),
        "rgb(255, 0, 0)"
    );
}

#[test]
fn excessive_scope_bounds_drop_the_subtree_without_broadening_it() {
    for css in [
        format!("@scope ({}) {{ b {{color:blue}} }}", attack("is", 5)),
        format!(
            "@scope (main) to ({}) {{ b {{color:blue}} }}",
            attack("is", 5)
        ),
    ] {
        let mut doc = chain(&css);
        doc.resolve(0.0);
        assert_eq!(
            doc.resolved_style_value(q(&doc, "#target"), "color"),
            "rgb(255, 0, 0)"
        );
    }
    let mut doc = chain("@scope (main) { b {color:blue} }");
    doc.resolve(0.0);
    assert_eq!(
        doc.resolved_style_value(q(&doc, "#target"), "color"),
        "rgb(0, 0, 255)"
    );
}
