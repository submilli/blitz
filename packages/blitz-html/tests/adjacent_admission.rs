//! Adjacent insertion delegates placement, parsing and rollback to the engine.
mod common;
use blitz_dom::{BaseDocument, DocumentConfig, dom_api::DomError};
use blitz_html::{DocumentHtmlParser, HtmlProvider};
use common::{parse, q};
use std::{cell::Cell, sync::Arc};

#[derive(Debug, PartialEq)]
enum Rejected {
    Dom(DomError),
    Policy,
}
impl From<DomError> for Rejected {
    fn from(error: DomError) -> Self {
        Self::Dom(error)
    }
}

#[test]
fn rejected_adjacent_fragments_preserve_every_position_and_release_templates() {
    for position in ["beforebegin", "afterbegin", "beforeend", "afterend"] {
        let mut doc = parse("<div id=parent><b id=a>old</b><i>tail</i></div>");
        let a = q(&doc, "#a");
        let parent = q(&doc, "#parent");
        let old = doc.get_node(a).unwrap().children[0];
        let before = doc.node_count();
        let markup = doc.get_node(parent).unwrap().inner_html();
        let result = doc.mutate().try_insert_adjacent_html_admitted(
            a,
            position,
            "<template><p>new</p></template>",
            |document, scratch| {
                assert_eq!(document.get_node(parent).unwrap().inner_html(), markup);
                assert_eq!(document.get_node(old).unwrap().parent, Some(a));
                assert!(
                    !document
                        .get_node(scratch.unwrap())
                        .unwrap()
                        .children
                        .is_empty()
                );
                Err::<(), _>(Rejected::Policy)
            },
        );
        assert_eq!(result, Err(Rejected::Policy));
        assert_eq!(doc.node_count(), before);
        assert_eq!(doc.get_node(parent).unwrap().inner_html(), markup);
    }
}

#[test]
fn accepted_adjacent_html_uses_table_context_and_returns_admission() {
    let mut doc = parse("<table><tbody id=a><tr><td>old</td></tr></tbody></table>");
    let a = q(&doc, "#a");
    let result = doc.mutate().try_insert_adjacent_html_admitted(
        a,
        "AfterBegin",
        "<tr><td>new</td></tr>",
        |document, scratch| {
            let first = document.get_node(scratch.unwrap()).unwrap().children[0];
            assert_eq!(document.node_name(first), "TR");
            Ok::<_, Rejected>(42)
        },
    );
    assert_eq!(result, Ok(42));
    assert_eq!(
        doc.get_node(a).unwrap().inner_html(),
        "<tr><td>new</td></tr><tr><td>old</td></tr>"
    );
}

#[test]
fn adjacent_parser_quota_failure_releases_scratch_and_skips_admission() {
    let mut doc = BaseDocument::new(DocumentConfig {
        node_limit: Some(32),
        html_parser_provider: Some(Arc::new(HtmlProvider)),
        ..Default::default()
    });
    DocumentHtmlParser::parse_into_mutator(&mut doc.mutate(), "<div id=a>old</div>");
    let a = q(&doc, "#a");
    let before = doc.node_count();
    let called = Cell::new(false);
    let result = doc.mutate().try_insert_adjacent_html_admitted(
        a,
        "beforeend",
        &"<b>new</b>".repeat(32),
        |_, _| {
            called.set(true);
            Ok::<_, Rejected>(())
        },
    );
    assert_eq!(result, Err(Rejected::Dom(DomError::QuotaExceeded)));
    assert!(!called.get());
    assert_eq!(doc.node_count(), before);
    assert_eq!(doc.get_node(a).unwrap().inner_html(), "old");
}

#[test]
fn adjacent_node_moves_and_hierarchy_errors_are_transactional() {
    let mut doc = parse("<div id=a><b id=first>first</b><i id=last>last</i></div>");
    let a = q(&doc, "#a");
    let first = q(&doc, "#first");
    let last = q(&doc, "#last");
    assert!(
        doc.mutate()
            .insert_adjacent_node(first, "beforebegin", last)
            .unwrap()
    );
    assert_eq!(doc.get_node(a).unwrap().children.as_slice(), &[last, first]);
    assert!(
        doc.mutate()
            .insert_adjacent_node(first, "afterend", first)
            .unwrap()
    );
    assert_eq!(doc.get_node(a).unwrap().children.as_slice(), &[last, first]);
    assert_eq!(
        doc.mutate().insert_adjacent_node(first, "afterbegin", a),
        Err(DomError::HierarchyRequest)
    );
    assert_eq!(doc.get_node(a).unwrap().children.as_slice(), &[last, first]);
    doc.mutate().remove_child(a, first).unwrap();
    assert!(
        !doc.mutate()
            .insert_adjacent_node(first, "beforebegin", last)
            .unwrap()
    );
    assert_eq!(doc.get_node(last).unwrap().parent, Some(a));
}

#[test]
fn adjacent_text_retains_code_units_and_cleans_up_on_hierarchy_failure() {
    let mut doc = parse("<div id=a></div>");
    let a = q(&doc, "#a");
    let text = blitz_dom::DomString::from_utf16(vec![0xd800, b'<' as u16, 0xdc00]);
    assert!(
        doc.mutate()
            .try_insert_adjacent_text(a, "beforeend", text.clone())
            .unwrap()
    );
    let child = doc.get_node(a).unwrap().children[0];
    assert_eq!(doc.character_data_dom(child).unwrap(), &text);
    let html = q(&doc, "html");
    let before = doc.node_count();
    assert_eq!(
        doc.mutate()
            .try_insert_adjacent_text(html, "beforebegin", "bad".into()),
        Err(DomError::HierarchyRequest)
    );
    assert_eq!(doc.node_count(), before);
    let parent = doc.get_node(a).unwrap().parent.unwrap();
    doc.mutate().remove_child(parent, a).unwrap();
    assert!(
        !doc.mutate()
            .try_insert_adjacent_text(a, "afterend", "lost".into())
            .unwrap()
    );
    assert_eq!(doc.node_count(), before);
}

#[test]
fn adjacent_html_in_templates_inserts_ordinary_children_and_root_uses_body_context() {
    let mut doc = parse("<template id=a><b>inert</b></template>");
    let a = q(&doc, "#a");
    doc.mutate()
        .insert_adjacent_html(a, "beforeend", "<i>ordinary</i>")
        .unwrap();
    assert_eq!(doc.get_node(a).unwrap().inner_html(), "<b>inert</b>");
    let child = doc.get_node(a).unwrap().children[0];
    assert_eq!(doc.node_name(child), "I");
    let html = q(&doc, "html");
    doc.mutate()
        .insert_adjacent_html(html, "afterbegin", "<p>new</p>")
        .unwrap();
    let child = doc.get_node(html).unwrap().children[0];
    assert_eq!(doc.node_name(child), "P");
    let before = doc.node_count();
    assert_eq!(
        doc.mutate().insert_adjacent_html(html, "beforebegin", ""),
        Err(DomError::NoModificationAllowed)
    );
    assert_eq!(
        doc.mutate().insert_adjacent_html(a, "afterbegın", ""),
        Err(DomError::Syntax)
    );
    assert_eq!(doc.node_count(), before);
}
