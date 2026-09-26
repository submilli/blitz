//! `insertAdjacentHTML` and media query evaluation (`matchMedia`).

mod common;

use blitz_dom::dom_api::DomError;
use common::{parse, q};

#[test]
fn insert_adjacent_html_in_all_four_positions() {
    let mut doc = parse("<div id=wrap><div id=a><p>x</p></div></div>");
    let (wrap, a) = (q(&doc, "#wrap"), q(&doc, "#a"));
    let mut m = doc.mutate();
    m.insert_adjacent_html(a, "beforebegin", "<i>1</i>").unwrap();
    m.insert_adjacent_html(a, "afterBegin", "<i>2</i>").unwrap();
    m.insert_adjacent_html(a, "beforeend", "<i>3</i>").unwrap();
    m.insert_adjacent_html(a, "afterend", "<i>4</i>").unwrap();
    drop(m);
    assert_eq!(
        doc.get_node(wrap).unwrap().inner_html(),
        r#"<i>1</i><div id="a"><i>2</i><p>x</p><i>3</i></div><i>4</i>"#
    );
}

#[test]
fn insert_adjacent_html_parses_in_the_context_element() {
    // Table content only parses correctly in a table context.
    let mut doc = parse("<table><tbody id=body></tbody></table>");
    let body = q(&doc, "#body");
    doc.mutate().insert_adjacent_html(body, "beforeend", "<tr><td>cell</td></tr>").unwrap();
    assert_eq!(doc.get_node(body).unwrap().inner_html(), "<tr><td>cell</td></tr>");
}

#[test]
fn insert_adjacent_html_errors() {
    let mut doc = parse("<div id=a></div>");
    let a = q(&doc, "#a");
    let html = doc.query_selector("html").unwrap().unwrap();
    let mut m = doc.mutate();
    assert_eq!(m.insert_adjacent_html(a, "nowhere", "x"), Err(DomError::Syntax));
    // Siblings of the document element would be children of the document.
    assert_eq!(m.insert_adjacent_html(html, "afterend", "x"), Err(DomError::NoModificationAllowed));
}

#[test]
fn media_queries_evaluate_against_the_viewport() {
    let mut doc = parse("");
    assert!(doc.evaluate_media_query("screen"));
    assert!(!doc.evaluate_media_query("print"));
    assert!(doc.evaluate_media_query("(min-width: 700px)"));
    assert!(!doc.evaluate_media_query("(min-width: 900px)"));
    assert!(doc.evaluate_media_query("print, (max-height: 600px)"));
    assert_eq!(doc.serialize_media_query("SCREEN   and (min-width:1px)"), "screen and (min-width: 1px)");
}
