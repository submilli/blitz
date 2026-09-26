//! The streaming parser pauses at scripts and honours document.write.

use std::cell::RefCell;
use std::rc::Rc;

use blitz_dom::{BaseDocument, DocumentConfig};
use blitz_html::{ParseStep, StreamingParser};

fn body_html(doc: &Rc<RefCell<BaseDocument>>) -> String {
    let doc = doc.borrow();
    let body = doc.query_selector("body").unwrap().unwrap();
    doc.get_node(body).unwrap().inner_html()
}

#[test]
fn pauses_at_each_script_with_later_content_unparsed() {
    let doc = Rc::new(RefCell::new(BaseDocument::new(DocumentConfig::default())));
    let mut parser = StreamingParser::new(doc.clone());
    parser.feed("<p>a</p><script>one</script><p>b</p><script>two</script><p>c</p>");
    parser.end_of_input();

    let ParseStep::Script(first) = parser.run() else { panic!("expected a script") };
    assert_eq!(doc.borrow().text_content_of(first).unwrap(), "one");
    assert_eq!(body_html(&doc), "<p>a</p><script>one</script>");

    let ParseStep::Script(_) = parser.run() else { panic!("expected a second script") };
    assert!(body_html(&doc).ends_with("<p>b</p><script>two</script>"));

    assert!(matches!(parser.run(), ParseStep::Done));
    assert!(parser.is_finished());
    assert!(body_html(&doc).ends_with("<p>c</p>"));
}

#[test]
fn document_write_inserts_at_the_insertion_point() {
    let doc = Rc::new(RefCell::new(BaseDocument::new(DocumentConfig::default())));
    let mut parser = StreamingParser::new(doc.clone());
    parser.feed("<p>a</p><script>w</script><p>b</p>");
    parser.end_of_input();

    let ParseStep::Script(_) = parser.run() else { panic!("expected a script") };
    // While the script runs, its writes are parsed immediately.
    parser.write("<i>written</i>");
    assert!(matches!(parser.run_written(), ParseStep::Done));
    assert_eq!(body_html(&doc), "<p>a</p><script>w</script><i>written</i>");

    assert!(matches!(parser.run(), ParseStep::Done));
    assert_eq!(body_html(&doc), "<p>a</p><script>w</script><i>written</i><p>b</p>");
}

#[test]
fn inner_html_scripts_never_run_and_parser_scripts_are_not_queued() {
    let doc = Rc::new(RefCell::new(BaseDocument::new(DocumentConfig::default())));
    let mut parser = StreamingParser::new(doc.clone());
    parser.feed("<div id=d></div><script>x</script>");
    parser.end_of_input();
    while let ParseStep::Script(_) = parser.run() {}
    // Parser-inserted scripts are the parser's to run.
    assert!(doc.borrow_mut().take_connected_scripts().is_empty());
}

#[test]
fn scripts_inserted_by_dom_manipulation_are_queued_once() {
    let doc = Rc::new(RefCell::new(BaseDocument::new(DocumentConfig {
        html_parser_provider: Some(std::sync::Arc::new(blitz_html::HtmlProvider)),
        ..Default::default()
    })));
    let mut parser = StreamingParser::new(doc.clone());
    parser.feed("<body><div id=d></div></body>");
    parser.end_of_input();
    while let ParseStep::Script(_) = parser.run() {}

    let mut d = doc.borrow_mut();
    let div = d.query_selector("#d").unwrap().unwrap();
    let mut m = d.mutate();
    let name = blitz_dom::QualName::new(None, blitz_dom::ns!(html), "script".into());
    let script = m.create_element(name, vec![]);
    m.append_children(div, &[script]);
    // Scripts created by innerHTML-style fragment parsing never run.
    m.set_inner_html_detaching(div, "<script>no</script>");
    drop(m);
    assert_eq!(d.take_connected_scripts(), vec![script]);
    assert!(d.take_connected_scripts().is_empty());
}
