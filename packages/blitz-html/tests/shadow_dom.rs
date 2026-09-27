//! Shadow DOM: shadow roots, slotting, and the flat tree that is styled,
//! laid out and exposed to accessibility.

mod common;

use accesskit::NodeId as AxId;
use blitz_dom::shadow::{AttachShadowError, ShadowRootInit};
use blitz_dom::{BaseDocument, NodeId};
use common::{parse, q};

fn attach(doc: &mut BaseDocument, host: NodeId, html: &str) -> NodeId {
    let mut m = doc.mutate();
    let root = m.attach_shadow(host, ShadowRootInit { open: true, ..Default::default() }).expect("attaches");
    m.set_inner_html_detaching(root, html);
    root
}

#[test]
fn a_shadow_root_is_its_own_tree() {
    let mut doc = parse("<div id=host><span id=light>light</span></div><p id=p></p>");
    let host = q(&doc, "#host");
    let root = attach(&mut doc, host, "<b id=inner>shadow</b>");
    let inner = doc.get_node(root).unwrap().children[0];

    assert_eq!(doc.shadow_root_of(host), Some(root));
    assert_eq!(doc.shadow_host_of(root), Some(host));
    assert_eq!(doc.containing_shadow_root(inner), Some(root));
    // Not a child of the host; ids stay in the shadow tree.
    assert!(!doc.get_node(host).unwrap().children.contains(&root));
    assert_eq!(doc.get_element_by_id("inner"), None);
    // Connected through its host.
    assert!(doc.is_connected(inner));
    assert!(doc.get_node(inner).unwrap().flags.is_in_document());

    // Only some elements may host, and only once.
    let p = q(&doc, "#p");
    let mut m = doc.mutate();
    assert_eq!(m.attach_shadow(host, ShadowRootInit::default()), Err(AttachShadowError::AlreadyAttached));
    let img = m.create_element(blitz_dom::QualName::new(None, blitz_dom::ns!(html), blitz_dom::LocalName::from("img")), vec![]);
    assert_eq!(m.attach_shadow(img, ShadowRootInit::default()), Err(AttachShadowError::NotSupported));
    assert!(m.attach_shadow(p, ShadowRootInit::default()).is_ok());
}

#[test]
fn slots_take_host_children_by_name() {
    let mut doc = parse("<div id=host><span id=a slot=title>A</span><span id=b>B</span><span id=c slot=nope>C</span></div>");
    let host = q(&doc, "#host");
    let root = attach(&mut doc, host, "<h1><slot name=title></slot></h1><p><slot id=default>fallback</slot></p>");
    doc.resolve(0.0);
    let (a, b, c) = (q(&doc, "#a"), q(&doc, "#b"), q(&doc, "#c"));
    let slots: Vec<NodeId> = {
        let h1 = doc.get_node(root).unwrap().children[0];
        let p = doc.get_node(root).unwrap().children[1];
        vec![doc.get_node(h1).unwrap().children[0], doc.get_node(p).unwrap().children[0]]
    };
    assert_eq!(doc.assigned_slot(a), Some(slots[0]));
    assert_eq!(doc.assigned_slot(b), Some(slots[1]));
    assert_eq!(doc.assigned_slot(c), None);
    assert_eq!(doc.assigned_nodes(slots[1]), vec![b]);
    // The flat tree: the host renders its shadow tree, slots their nodes.
    assert_eq!(doc.get_node(host).unwrap().flat_children(), doc.get_node(root).unwrap().children.as_slice());
    assert_eq!(doc.get_node(slots[0]).unwrap().flat_children(), &[a]);
    assert_eq!(doc.get_node(c).unwrap().flat_parent_id(), None, "unassigned: not rendered");
}

#[test]
fn shadow_content_is_laid_out_and_accessible() {
    let mut doc = parse("<div id=host><span id=light slot=x>Slotted text</span><span>Not rendered</span></div>");
    let host = q(&doc, "#host");
    attach(&mut doc, host, "<button style='display:block;width:50px;height:20px'>Shadow button</button><slot name=x></slot>");
    doc.resolve(0.0);
    let tree: std::collections::HashMap<AxId, accesskit::Node> = doc.build_accessibility_tree().nodes.into_iter().collect();
    let labels: Vec<String> = tree.values().filter_map(|n| n.label().map(str::to_string)).collect();
    assert!(labels.iter().any(|l| l == "Shadow button"), "{labels:?}");
    let light = q(&doc, "#light");
    assert!(tree.contains_key(&AxId(light.as_u64())), "slotted content is in the tree");
    // Laid out: the button has a box.
    let button = {
        let root = doc.shadow_root_of(host).unwrap();
        doc.get_node(root).unwrap().children[0]
    };
    let layout = doc.get_node(button).unwrap().final_layout();
    assert!(layout.size.width > 0.0 && layout.size.height > 0.0, "{layout:?}");
    let host_layout = doc.get_node(host).unwrap().final_layout();
    assert!(host_layout.size.height > 0.0);
}

fn width(doc: &BaseDocument, id: NodeId) -> f32 {
    doc.get_node(id).unwrap().final_layout().size.width
}

#[test]
fn shadow_styles_are_scoped() {
    let mut doc = parse(
        "<style>b { display: block; width: 11px } .light { display: block; width: 22px }</style>\
         <div id=host><span class=light id=l slot=s>light</span></div><b id=outer>outer</b>\
         <div id=hidden></div><p id=none style='display: none'></p>",
    );
    let host = q(&doc, "#host");
    let root = attach(
        &mut doc,
        host,
        "<style>b { display: block; width: 77px } :host { display: block; width: 300px } ::slotted(.light) { width: 33px; height: 44px }</style>\
         <b>inner</b><slot name=s></slot>",
    );
    let hidden = q(&doc, "#hidden");
    attach(&mut doc, hidden, "<style>:host { display: none }</style>visible?");
    doc.resolve(0.0);

    let inner_b = doc.get_node(root).unwrap().children[1];
    assert_eq!(width(&doc, inner_b), 77.0, "the shadow sheet styles its tree");
    assert_eq!(width(&doc, q(&doc, "#outer")), 11.0, "and not the document");
    assert_eq!(width(&doc, host), 300.0, ":host styles the host");
    // The outer tree's normal declarations beat ::slotted() (the shadow
    // cascade), but ::slotted() still applies where the document is silent.
    assert_eq!(width(&doc, q(&doc, "#l")), 22.0);
    assert_eq!(doc.get_node(q(&doc, "#l")).unwrap().final_layout().size.height, 44.0);
    let hidden_display = doc.get_node(hidden).unwrap().primary_styles().map(|s| format!("{:?}", s.get_box().display));
    assert_eq!(hidden_display, Some(format!("{:?}", doc.get_node(q(&doc, "#none")).unwrap().primary_styles().unwrap().get_box().display)), ":host {{ display: none }} hides the host");
}
