//! Paused tree-builder handles must outlive detachments made by script.
use blitz_dom::{BaseDocument, DocumentConfig};
use blitz_html::{ParseStep, StreamingParser};
use std::{cell::RefCell, rc::Rc};

#[test]
fn paused_parser_preserves_detached_open_formatting_template_and_form_handles() {
    for (markup, selector, tail) in [
        ("<div id=kept>", "#kept", "tail</div>"),
        ("<p><b id=kept>bold</p>", "#kept", "tail"),
        ("<template id=kept><b>", "#kept", "tail</b></template>"),
        ("<table><form id=kept>", "#kept", "<input></table>"),
    ] {
        let doc = Rc::new(RefCell::new(BaseDocument::new(DocumentConfig::default())));
        let mut parser = StreamingParser::new(doc.clone());
        parser.feed(markup);
        assert!(matches!(parser.run(), ParseStep::Done));
        assert!(!parser.is_finished());
        let target = doc.borrow().query_selector(selector).unwrap().unwrap();
        let parent = doc.borrow().get_node(target).unwrap().parent.unwrap();
        doc.borrow_mut()
            .mutate()
            .remove_child(parent, target)
            .unwrap();
        assert!(
            !doc.borrow_mut()
                .reclaim_detached_nodes(&[])
                .contains(&target),
            "{markup}"
        );
        assert!(doc.borrow().get_node(target).is_some());
        parser.feed(tail);
        parser.end_of_input();
        assert!(matches!(parser.run(), ParseStep::Done));
        assert!(parser.is_finished());
        // Retaining the completed parser must not retain its former handles.
        assert!(
            doc.borrow_mut()
                .reclaim_detached_nodes(&[])
                .contains(&target),
            "{markup}"
        );
    }
}

#[test]
fn cancelled_parser_releases_detached_tree_without_borrowing_document() {
    let doc = Rc::new(RefCell::new(BaseDocument::new(DocumentConfig::default())));
    let mut parser = StreamingParser::new(doc.clone());
    parser.feed("<div id=kept><b>");
    assert!(matches!(parser.run(), ParseStep::Done));
    let target = doc.borrow().query_selector("#kept").unwrap().unwrap();
    let parent = doc.borrow().get_node(target).unwrap().parent.unwrap();
    let mut document = doc.borrow_mut();
    document.mutate().remove_child(parent, target).unwrap();
    assert!(document.reclaim_detached_nodes(&[]).is_empty());
    drop(parser);
    assert!(document.reclaim_detached_nodes(&[]).contains(&target));
}

#[test]
fn exhausted_parser_releases_handles_before_the_parser_is_dropped() {
    let doc = Rc::new(RefCell::new(BaseDocument::new(DocumentConfig {
        node_limit: Some(32),
        ..DocumentConfig::default()
    })));
    let mut parser = StreamingParser::new(doc.clone());
    parser.feed("<div id=kept><b>");
    assert!(matches!(parser.run(), ParseStep::Done));
    let target = doc.borrow().query_selector("#kept").unwrap().unwrap();
    let parent = doc.borrow().get_node(target).unwrap().parent.unwrap();
    doc.borrow_mut()
        .mutate()
        .remove_child(parent, target)
        .unwrap();
    parser.feed(&"<i>".repeat(32));
    assert!(matches!(parser.run(), ParseStep::Done));
    assert!(parser.is_finished());
    assert!(
        doc.borrow_mut()
            .reclaim_detached_nodes(&[])
            .contains(&target)
    );
}
