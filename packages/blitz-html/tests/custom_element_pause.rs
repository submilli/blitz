//! The streaming parser stops before creating an element whose custom
//! element definition exists, so the embedder can construct it (HTML "create
//! an element for a token" steps 7–10).

use std::cell::RefCell;
use std::rc::Rc;

use blitz_dom::{BaseDocument, DocumentConfig, NodeId, QualName, ns};
use blitz_html::{ParseStep, StreamingParser};

fn parse(
    markup: &str,
    defined: &'static [&'static str],
) -> (Rc<RefCell<BaseDocument>>, StreamingParser) {
    let doc = Rc::new(RefCell::new(BaseDocument::new(DocumentConfig::default())));
    let mut parser = StreamingParser::new(doc.clone());
    parser.set_definition_lookup(Box::new(|name| defined.contains(&name)));
    parser.feed(markup);
    parser.end_of_input();
    (doc, parser)
}

fn body_html(doc: &Rc<RefCell<BaseDocument>>) -> String {
    let doc = doc.borrow();
    let body = doc.query_selector("body").unwrap().unwrap();
    doc.get_node(body).unwrap().inner_html()
}

fn element(doc: &Rc<RefCell<BaseDocument>>, name: &str) -> NodeId {
    doc.borrow_mut()
        .mutate()
        .create_element(QualName::new(None, ns!(html), name.into()), Vec::new())
}

fn attribute_count(doc: &Rc<RefCell<BaseDocument>>, id: NodeId) -> usize {
    doc.borrow()
        .get_node(id)
        .unwrap()
        .element_data()
        .unwrap()
        .attrs()
        .len()
}

/// The local name of `id`'s parent: the constructed element was inserted.
fn parent_name(doc: &Rc<RefCell<BaseDocument>>, id: NodeId) -> String {
    let doc = doc.borrow();
    let parent = doc.get_node(id).unwrap().parent.unwrap();
    doc.get_node(parent)
        .unwrap()
        .element_data()
        .unwrap()
        .name
        .local
        .to_string()
}

#[test]
fn stops_before_a_defined_element_and_creates_the_constructed_one() {
    let (doc, mut parser) = parse("<p>a</p><x-a id=x title=t></x-a><y-b></y-b>", &["x-a"]);

    let ParseStep::CustomElement(name) = parser.run() else {
        panic!("expected a custom element")
    };
    assert_eq!(&*name, "x-a");
    assert_eq!(body_html(&doc), "<p>a</p>");

    // Step 9 appends the token's attributes before the element is inserted.
    let constructed = element(&doc, "x-a");
    assert!(parser.construct_custom_element(constructed));
    assert_eq!(attribute_count(&doc, constructed), 2);
    assert!(doc.borrow().get_node(constructed).unwrap().parent.is_none());
    assert_eq!(body_html(&doc), "<p>a</p>");

    assert!(matches!(parser.run(), ParseStep::CustomElementCreated));
    assert_eq!(parent_name(&doc, constructed), "body");
    assert_eq!(body_html(&doc), r#"<p>a</p><x-a id="x" title="t"></x-a>"#);

    // An undefined name does not stop the parser.
    assert!(matches!(parser.run(), ParseStep::Done));
    assert_eq!(
        body_html(&doc),
        r#"<p>a</p><x-a id="x" title="t"></x-a><y-b></y-b>"#
    );
}

#[test]
fn without_a_constructed_element_the_parser_creates_it() {
    let (doc, mut parser) = parse("<x-a id=x></x-a>", &["x-a"]);
    assert!(matches!(parser.run(), ParseStep::CustomElement(_)));
    // An element of another name is refused.
    let other = element(&doc, "x-b");
    assert!(!parser.construct_custom_element(other));
    assert_eq!(attribute_count(&doc, other), 0);

    assert!(matches!(parser.run(), ParseStep::CustomElementCreated));
    assert_eq!(body_html(&doc), r#"<x-a id="x"></x-a>"#);
    assert!(matches!(parser.run(), ParseStep::Done));
}

#[test]
fn a_processed_tag_refuses_a_late_element() {
    let (doc, mut parser) = parse("<x-a></x-a><p>b</p>", &["x-a"]);
    assert!(matches!(parser.run(), ParseStep::CustomElement(_)));
    // A nested run (a microtask's write into the parser) created it first.
    assert!(matches!(parser.run(), ParseStep::CustomElementCreated));
    let late = element(&doc, "x-a");
    assert!(!parser.construct_custom_element(late));
    assert!(matches!(parser.run(), ParseStep::Done));
    assert_eq!(body_html(&doc), "<x-a></x-a><p>b</p>");
}

#[test]
fn written_custom_elements_stop_the_written_parse() {
    let (doc, mut parser) = parse("<p>b</p><script>w</script><p>after</p>", &["x-a"]);
    assert!(matches!(parser.run(), ParseStep::Script(_)));
    parser.write("<x-a></x-a><i>rest</i>");
    assert!(matches!(parser.run_written(), ParseStep::CustomElement(_)));
    let constructed = element(&doc, "x-a");
    assert!(parser.construct_custom_element(constructed));
    assert!(matches!(
        parser.run_written(),
        ParseStep::CustomElementCreated
    ));
    assert_eq!(parent_name(&doc, constructed), "body");
    assert!(matches!(parser.run_written(), ParseStep::Done));
    assert_eq!(
        body_html(&doc),
        "<p>b</p><script>w</script><x-a></x-a><i>rest</i>"
    );
    assert!(matches!(parser.run(), ParseStep::Done));
    assert!(body_html(&doc).ends_with("<i>rest</i><p>after</p>"));
}

#[test]
fn foreign_content_and_frameset_modes_do_not_stop() {
    let (doc, mut parser) = parse("<svg><x-a></x-a></svg>", &["x-a"]);
    assert!(matches!(parser.run(), ParseStep::Done));
    assert_eq!(body_html(&doc), "<svg><x-a></x-a></svg>");

    // An HTML integration point creates HTML elements again.
    let (_, mut parser) = parse("<svg><foreignObject><x-a></x-a>", &["x-a"]);
    assert!(matches!(parser.run(), ParseStep::CustomElement(_)));

    let (_, mut parser) = parse("<frameset><x-a></x-a></frameset>", &["x-a"]);
    assert!(matches!(parser.run(), ParseStep::Done));
}

/// The steps before the element's creation run before the parser stops:
/// reconstructing the active formatting elements clones `<b>` first.
#[test]
fn formatting_elements_are_reconstructed_before_construction() {
    let (doc, mut parser) = parse("<p><b>1</p><x-a></x-a>", &["x-a"]);
    assert!(matches!(parser.run(), ParseStep::CustomElement(_)));
    assert_eq!(body_html(&doc), "<p><b>1</b></p><b></b>");
    let constructed = element(&doc, "x-a");
    assert!(parser.construct_custom_element(constructed));
    assert!(matches!(parser.run(), ParseStep::CustomElementCreated));
    assert_eq!(parent_name(&doc, constructed), "b");
    assert!(matches!(parser.run(), ParseStep::Done));
    assert_eq!(body_html(&doc), "<p><b>1</b></p><b><x-a></x-a></b>");
}

/// Implied elements exist when the parser stops: `<x-a>` in the "in head"
/// insertion mode first ends the head and inserts the body.
#[test]
fn implied_elements_are_inserted_before_construction() {
    let (doc, mut parser) = parse("<title>t</title><x-a></x-a>", &["x-a"]);
    assert!(matches!(parser.run(), ParseStep::CustomElement(_)));
    assert!(doc.borrow().query_selector("body").unwrap().is_some());
    assert_eq!(body_html(&doc), "");
}

/// Pending table text is flushed (foster-parented) first, and the element is
/// foster-parented too.
#[test]
fn table_text_and_foster_parenting_precede_and_follow_construction() {
    let (doc, mut parser) = parse("<table>abc<x-a></x-a></table>", &["x-a"]);
    assert!(matches!(parser.run(), ParseStep::CustomElement(_)));
    assert_eq!(body_html(&doc), "abc<table></table>");
    let constructed = element(&doc, "x-a");
    assert!(parser.construct_custom_element(constructed));
    assert!(matches!(parser.run(), ParseStep::CustomElementCreated));
    assert!(matches!(parser.run(), ParseStep::Done));
    assert_eq!(body_html(&doc), "abc<x-a></x-a><table></table>");
}

/// Template contents belong to an inert document without custom element
/// definitions: the parser does not stop there.
#[test]
fn template_contents_do_not_stop() {
    let (_, mut parser) = parse(
        "<template><x-a></x-a><div><x-a></x-a></div></template><table><template><x-a>",
        &["x-a"],
    );
    assert!(matches!(parser.run(), ParseStep::Done));
}

/// A declarative shadow root's contents belong to the document, so a
/// custom element there is constructed; an inert template inside it is not.
#[test]
fn declarative_shadow_roots_stop_but_inert_templates_inside_them_do_not() {
    let (_, mut parser) = parse(
        "<div><template shadowrootmode=open><template><x-a></x-a></template>\
         <x-a></x-a></template></div>",
        &["x-a"],
    );
    assert!(matches!(parser.run(), ParseStep::CustomElement(_)));
    assert!(matches!(parser.run(), ParseStep::CustomElementCreated));
    assert!(matches!(parser.run(), ParseStep::Done));
}

/// Script can give the constructed element children before its insertion;
/// inserting it under its own descendant is not possible, so it stays where
/// script put it and the tree keeps no cycle.
#[test]
fn a_constructed_ancestor_of_the_insertion_point_is_not_inserted() {
    let (doc, mut parser) = parse("<p>a</p><x-a></x-a><i>b</i>", &["x-a"]);
    assert!(matches!(parser.run(), ParseStep::CustomElement(_)));
    let constructed = element(&doc, "x-a");
    assert!(parser.construct_custom_element(constructed));
    // An attribute callback's `this.appendChild(document.body)`.
    let body = doc.borrow().query_selector("body").unwrap().unwrap();
    doc.borrow_mut()
        .mutate()
        .append_children(constructed, &[body]);
    assert!(matches!(parser.run(), ParseStep::CustomElementCreated));
    assert!(matches!(parser.run(), ParseStep::Done));
    let doc = doc.borrow();
    assert_eq!(doc.get_node(constructed).unwrap().parent, None);
    assert_eq!(doc.get_node(body).unwrap().parent, Some(constructed));
}

#[test]
fn elements_with_handler_attributes_are_recorded() {
    let (doc, mut parser) = parse(
        "<p onclick=f()>a</p><x-a onload=g()></x-a><i>b</i>",
        &["x-a"],
    );
    assert!(matches!(parser.run(), ParseStep::CustomElement(_)));
    let p = doc.borrow().query_selector("p").unwrap().unwrap();
    assert_eq!(parser.take_handler_elements(), vec![p]);
    let constructed = element(&doc, "x-a");
    assert!(parser.construct_custom_element(constructed));
    assert_eq!(parser.take_handler_elements(), vec![constructed]);
    assert!(matches!(parser.run(), ParseStep::CustomElementCreated));
    assert!(matches!(parser.run(), ParseStep::Done));
    assert!(parser.take_handler_elements().is_empty());
}
