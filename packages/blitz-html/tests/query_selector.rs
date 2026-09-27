//! Selector queries: `query_selector*`, `matches_selector` and `closest`.

mod common;

use blitz_dom::MAX_SELECTOR_NESTING;
use common::{parse, q};

const PAGE: &str = "<!DOCTYPE html><div id=a><p title='((('>one</p><p>two</p></div>";

/// Run `f` on a thread with the 8 MiB stack embedders give script.
fn on_script_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(8 << 20)
        .spawn(f)
        .expect("the test thread starts")
        .join()
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
}

fn nested(depth: usize) -> String {
    format!("{}p{}", ":is(".repeat(depth), ")".repeat(depth))
}

#[test]
fn nesting_up_to_the_limit_parses_and_matches() {
    on_script_stack(|| {
        let doc = parse(PAGE);
        let a = q(&doc, "#a");
        let selector = nested(MAX_SELECTOR_NESTING);
        assert_eq!(doc.query_selector_all_in(a, &selector).unwrap().len(), 2);
        let p = doc.query_selector_in(a, "p").unwrap().unwrap();
        assert!(doc.matches_selector(p, &selector).unwrap());
    });
}

#[test]
fn deeper_nesting_is_a_parse_error_not_a_stack_overflow() {
    on_script_stack(|| {
        let doc = parse(PAGE);
        let a = q(&doc, "#a");
        for selector in [nested(MAX_SELECTOR_NESTING + 1), nested(100_000)] {
            assert!(doc.try_parse_selector_list(&selector).is_err());
            assert!(doc.query_selector_in(a, &selector).is_err());
            assert!(doc.query_selector_all_in(a, &selector).is_err());
            assert!(doc.matches_selector(a, &selector).is_err());
            assert!(doc.closest(a, &selector).is_err());
        }
        let not = format!("{}p{}", ":not(".repeat(100_000), ")".repeat(100_000));
        assert!(doc.query_selector(&not).is_err());
    });
}

#[test]
fn parentheses_in_strings_comments_and_escapes_do_not_count() {
    let doc = parse(PAGE);
    let many = "(".repeat(10 * MAX_SELECTOR_NESTING);
    for selector in [
        format!("p[title^=\"{many}\"]"),
        format!("p[title^='{many}']"),
        format!("p/*{many}*/"),
        format!("p, #\\({}", "\\(".repeat(10 * MAX_SELECTOR_NESTING)),
    ] {
        assert!(doc.try_parse_selector_list(&selector).is_ok(), "{selector}");
    }
    assert_eq!(doc.query_selector_all("p[title^='(']").unwrap().len(), 1);
}
