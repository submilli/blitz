//! DOM Standard node operations in `blitz_dom::dom_api`.

mod common;

use blitz_dom::dom_api::{DomError, node_type};
use common::{parse, q};

const PAGE: &str = "<!DOCTYPE html><html><head></head><body><div id=a><p id=p1>one</p><p id=p2>two</p></div></body></html>";

#[test]
fn node_type_and_name() {
    let doc = parse(PAGE);
    let root = doc.root_node().id;
    assert_eq!(doc.node_type(root), node_type::DOCUMENT);
    assert_eq!(doc.node_name(root), "#document");
    assert_eq!(doc.node_name(q(&doc, "#a")), "DIV");
    let text = doc.get_node(q(&doc, "#p1")).unwrap().children[0];
    assert_eq!(doc.node_type(text), node_type::TEXT);
    assert_eq!(doc.node_name(text), "#text");
}

#[test]
fn text_content_get_and_set() {
    let mut doc = parse(PAGE);
    let a = q(&doc, "#a");
    assert_eq!(doc.text_content_of(a).as_deref(), Some("onetwo"));
    assert_eq!(doc.text_content_of(doc.root_node().id), None);

    let old_child = q(&doc, "#p1");
    doc.mutate().set_text_content(a, "replaced");
    assert_eq!(doc.text_content_of(a).as_deref(), Some("replaced"));
    // Removed children are detached, not dropped.
    let old = doc.get_node(old_child).expect("old child still valid");
    assert!(old.parent.is_none());
}

#[test]
fn pre_insert_enforces_hierarchy_rules() {
    let mut doc = parse(PAGE);
    let a = q(&doc, "#a");
    let body = q(&doc, "body");
    let root = doc.root_node().id;
    let mut m = doc.mutate();
    // An ancestor cannot be inserted into its descendant.
    assert_eq!(m.pre_insert(body, a, None), Err(DomError::HierarchyRequest));
    // A doctype cannot go into an element; text cannot go into a document.
    let doctype = m.create_doctype("html", "", "");
    assert_eq!(m.pre_insert(doctype, a, None), Err(DomError::HierarchyRequest));
    let text = m.create_text_node("x");
    assert_eq!(m.pre_insert(text, root, None), Err(DomError::HierarchyRequest));
    // A second document element is rejected.
    let extra = m.create_element(blitz_dom::QualName::new(None, blitz_dom::ns!(html), "div".into()), vec![]);
    assert_eq!(m.pre_insert(extra, root, None), Err(DomError::HierarchyRequest));
    // The reference child must be a child of the parent.
    assert_eq!(m.pre_insert(text, a, Some(body)), Err(DomError::NotFound));
}

#[test]
fn fragments_insert_their_children() {
    let mut doc = parse(PAGE);
    let a = q(&doc, "#a");
    let p2 = q(&doc, "#p2");
    let mut m = doc.mutate();
    let fragment = m.create_document_fragment();
    let x = m.create_text_node("x");
    let y = m.create_text_node("y");
    m.pre_insert(x, fragment, None).unwrap();
    m.pre_insert(y, fragment, None).unwrap();
    m.pre_insert(fragment, a, Some(p2)).unwrap();
    drop(m);
    assert!(doc.get_node(fragment).unwrap().children.is_empty());
    assert_eq!(doc.text_content_of(a).as_deref(), Some("onexytwo"));
}

#[test]
fn replace_and_remove_child() {
    let mut doc = parse(PAGE);
    let a = q(&doc, "#a");
    let (p1, p2) = (q(&doc, "#p1"), q(&doc, "#p2"));
    let mut m = doc.mutate();
    let text = m.create_text_node("new");
    assert_eq!(m.replace_child(a, text, p1), Ok(p1));
    assert_eq!(m.remove_child(a, p1), Err(DomError::NotFound));
    assert_eq!(m.remove_child(a, p2), Ok(p2));
    drop(m);
    assert_eq!(doc.text_content_of(a).as_deref(), Some("new"));
}

#[test]
fn clone_node_shallow_and_deep_including_template_contents() {
    let mut doc = parse("<div id=a title=t><b>x</b></div><template id=t><i>in</i></template>");
    let (a, t) = (q(&doc, "#a"), q(&doc, "#t"));
    let mut m = doc.mutate();
    let shallow = m.clone_node(a, false);
    let deep = m.clone_node(a, true);
    let template = m.clone_node(t, true);
    drop(m);
    assert!(doc.get_node(shallow).unwrap().children.is_empty());
    assert_eq!(doc.get_node(deep).unwrap().outer_html(), r#"<div id="a" title="t"><b>x</b></div>"#);
    assert_eq!(doc.get_node(template).unwrap().inner_html(), "<i>in</i>");
    assert!(doc.get_node(deep).unwrap().parent.is_none());
}

#[test]
fn attributes_by_qualified_name() {
    let mut doc = parse(PAGE);
    let a = q(&doc, "#a");
    let mut m = doc.mutate();
    // HTML elements lowercase names.
    m.set_attribute_by_name(a, "Data-X", "1").unwrap();
    assert_eq!(m.set_attribute_by_name(a, "bad name", "1"), Err(DomError::InvalidCharacter));
    assert_eq!(m.toggle_attribute(a, "hidden", None), Ok(true));
    assert_eq!(m.toggle_attribute(a, "hidden", Some(true)), Ok(true));
    assert_eq!(m.toggle_attribute(a, "hidden", None), Ok(false));
    drop(m);
    assert_eq!(doc.attribute_by_name(a, "DATA-x").map(|a| a.value.as_str()), Some("1"));
    assert_eq!(doc.attribute_names(a), vec!["id", "data-x"]);
    doc.mutate().remove_attribute_by_name(a, "data-x");
    assert!(doc.attribute_by_name(a, "data-x").is_none());
}

#[test]
fn inner_html_setter_detaches_old_children() {
    let mut doc = parse(PAGE);
    let a = q(&doc, "#a");
    let p1 = q(&doc, "#p1");
    doc.mutate().set_inner_html_detaching(a, "<span>new</span>");
    assert_eq!(doc.get_node(a).unwrap().inner_html(), "<span>new</span>");
    assert!(doc.get_node(p1).is_some(), "old children stay valid");
}

#[test]
fn dom_generation_changes_after_mutation() {
    let mut doc = parse(PAGE);
    let before = doc.dom_generation();
    let a = q(&doc, "#a");
    doc.mutate().set_text_content(a, "x");
    assert!(doc.dom_generation() > before);
}
