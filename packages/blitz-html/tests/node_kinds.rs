//! DocumentFragment, DocumentType and ProcessingInstruction nodes.

mod common;

use blitz_dom::node::{NodeData, NodeKind};
use common::{parse, q};

#[test]
fn doctype_is_the_first_child_of_the_document() {
    let doc = parse("<!DOCTYPE html><html><body></body></html>");
    let first = doc.get_node(doc.root_node().children[0]).unwrap();
    match &first.data {
        NodeData::Doctype {
            name,
            public_id,
            system_id,
        } => {
            assert_eq!(name, "html");
            assert!(public_id.is_empty() && system_id.is_empty());
        }
        other => panic!("expected a doctype, got {other:?}"),
    }
    assert_eq!(first.data.kind(), NodeKind::Doctype);
}

#[test]
fn template_contents_is_a_document_fragment() {
    let doc = parse("<template id=t><b>bold</b></template>");
    let t = q(&doc, "#t");
    let contents = doc
        .get_node(t)
        .unwrap()
        .element_data()
        .unwrap()
        .template_contents
        .unwrap();
    let fragment = doc.get_node(contents).unwrap();
    assert!(matches!(fragment.data, NodeData::DocumentFragment));
    assert!(fragment.parent.is_none(), "template contents are detached");
    assert_eq!(fragment.children.len(), 1);
    // The template element itself has no children.
    assert!(doc.get_node(t).unwrap().children.is_empty());
}

#[test]
fn mutator_creates_each_node_kind() {
    let mut doc = parse("");
    let mut m = doc.mutate();
    let fragment = m.create_document_fragment();
    let doctype = m.create_doctype("html", "", "");
    let pi = m.create_processing_instruction("xml-stylesheet", "href='a.css'");
    drop(m);
    assert_eq!(
        doc.get_node(fragment).unwrap().data.kind(),
        NodeKind::DocumentFragment
    );
    assert_eq!(
        doc.get_node(doctype).unwrap().data.kind(),
        NodeKind::Doctype
    );
    assert!(matches!(
        &doc.get_node(pi).unwrap().data,
        NodeData::ProcessingInstruction { target, .. } if target == "xml-stylesheet"
    ));
}
