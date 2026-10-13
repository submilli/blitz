//! The parser nests nodes at most 512 levels below the parse's root element,
//! as Chrome 155 does (Blink's `kMaximumHTMLParserDOMTreeDepth`); deeper
//! nodes attach to their parent's parent.

mod common;

use blitz_dom::{BaseDocument, NodeData, NodeId};
use common::{parse, q};

/// Element levels below `root` along first element children.
fn depth_below(doc: &BaseDocument, root: NodeId) -> usize {
    let mut depth = 0;
    let mut node = root;
    while let Some(child) = first_element_child(doc, node) {
        node = child;
        depth += 1;
    }
    depth
}

fn first_element_child(doc: &BaseDocument, node: NodeId) -> Option<NodeId> {
    let children = &doc.get_node(node).unwrap().children;
    children
        .iter()
        .copied()
        .find(|&child| doc.get_node(child).unwrap().is_element())
}

/// The node `levels` first element children below `root`.
fn descend(doc: &BaseDocument, root: NodeId, levels: usize) -> NodeId {
    (0..levels).fold(root, |node, _| first_element_child(doc, node).unwrap())
}

fn child_count(doc: &BaseDocument, node: NodeId) -> usize {
    doc.get_node(node).unwrap().children.len()
}

fn divs(n: usize) -> String {
    format!("{}x{}", "<div>".repeat(n), "</div>".repeat(n))
}

#[test]
fn a_document_nests_511_levels_below_its_body() {
    let doc = parse(&format!("<!doctype html><body>{}<p>after</p>", divs(600)));
    let body = q(&doc, "body");
    assert_eq!(depth_below(&doc, body), 511);
    // The deepest div on the first-child chain is empty: the divs past the
    // cap went beside it, and the text into the last of them.
    let deepest = descend(&doc, body, 511);
    assert_eq!(child_count(&doc, deepest), 0);
    let p = q(&doc, "p");
    assert_eq!(doc.get_node(p).unwrap().parent, Some(body));
}

#[test]
fn a_fragment_nests_512_levels_below_its_context() {
    let mut doc = parse("<!doctype html><body><div id=host></div>");
    let host = q(&doc, "#host");
    doc.mutate().set_inner_html(host, &"<div>".repeat(600));
    assert_eq!(depth_below(&doc, host), 512);
    assert_eq!(child_count(&doc, descend(&doc, host, 510)), 1);
    // The 512th div and the 88 past the cap.
    assert_eq!(child_count(&doc, descend(&doc, host, 511)), 600 - 511);
}

#[test]
fn foster_parented_content_nests_511_levels_below_the_body() {
    let mut doc = parse("<!doctype html><body>");
    let body = q(&doc, "body");
    doc.mutate()
        .set_inner_html(body, &format!("<table>{}</table>", "<div>".repeat(600)));
    assert_eq!(depth_below(&doc, body), 511);
}

#[test]
fn a_comment_past_the_cap_attaches_to_the_deepest_parent_but_one() {
    let mut doc = parse("<!doctype html><body>");
    let body = q(&doc, "body");
    doc.mutate()
        .set_inner_html(body, &format!("{}<!--c-->", "<span>".repeat(600)));
    // Chrome: the 511th span (index 510) holds the comment.
    let holder = descend(&doc, body, 511);
    let holds_comment = doc
        .get_node(holder)
        .unwrap()
        .children
        .iter()
        .any(|&child| matches!(doc.get_node(child).unwrap().data, NodeData::Comment { .. }));
    assert!(holds_comment);
}

#[test]
fn template_contents_count_the_open_elements_around_them() {
    // Chrome counts the open elements, not the contents' own depth: 510
    // below `<html><body><template>`, 512 in a template's innerHTML, 511
    // below a fragment's `<template>`.
    let deep = "<div>".repeat(600);
    let mut doc = parse(&format!(
        "<!doctype html><body><template id=parsed>{deep}</template><template id=set></template><div id=host></div>"
    ));
    let set = q(&doc, "#set");
    let host = q(&doc, "#host");
    doc.mutate().set_inner_html(set, &deep);
    doc.mutate()
        .set_inner_html(host, &format!("<template id=nested>{deep}</template>"));
    let depth = |selector: &str| depth_below(&doc, template_contents(&doc, q(&doc, selector)));
    assert_eq!(depth("#parsed"), 510);
    assert_eq!(depth("#nested"), 511);
    // Parsed with the template as context; script's `innerHTML` setter then
    // moves the nodes into its contents.
    assert_eq!(depth_below(&doc, set), 512);
}

fn template_contents(doc: &BaseDocument, template: NodeId) -> NodeId {
    doc.get_node(template)
        .unwrap()
        .element_data()
        .unwrap()
        .template_contents
        .expect("a template has contents")
}

/// Chrome 155 (`docs/reconstruction/probes/SUB-1578/parser-placement.js` in
/// submilli-browser), after 600 nested divs: the non-div children of the
/// 510th level, the children of its last element and of the deepest div.
/// Only elements and comments follow the limit (Blink's `Attach`); text,
/// foster-parented nodes and adoption-agency moves stay where HTML puts
/// them, and a template past the limit gets its content beside it.
const PLACEMENT: &[(&str, &str, [&str; 3])] = &[
    ("text", "x", ["", "#x", ""]),
    ("table", "<table>x<i>y</i></table>z", ["#x i table", "", ""]),
    ("adoption", "<b>1<p>2</b>3", ["b", "#1", ""]),
    (
        "template",
        "<template><b>x</b></template>",
        ["template b", "#x", ""],
    ),
    ("comment", "<!--c-->", ["!", "", ""]),
];

#[test]
fn nodes_past_the_limit_go_where_chrome_puts_them() {
    for (name, tail, chrome) in PLACEMENT {
        let doc = parse(&format!(
            "<!doctype html><body>{}{tail}",
            "<div>".repeat(600)
        ));
        let deepest = descend(&doc, q(&doc, "body"), 511);
        let level = doc.get_node(deepest).unwrap().parent.unwrap();
        let last = *element_children(&doc, level).last().unwrap();
        let non_divs: Vec<String> = describe_children(&doc, level)
            .into_iter()
            .filter(|child| child != "div")
            .collect();
        let ours = [
            non_divs.join(" "),
            describe_children(&doc, last).join(" "),
            describe_children(&doc, deepest).join(" "),
        ];
        assert_eq!(ours, *chrome, "{name}");
    }
    let doc = parse(&format!(
        "<!doctype html><body>{}<template><b>x</b></template>",
        "<div>".repeat(600)
    ));
    // Its contents were never needed.
    let template = q(&doc, "template");
    let contents = doc
        .get_node(template)
        .unwrap()
        .element_data()
        .unwrap()
        .template_contents;
    assert!(contents.is_none_or(|contents| doc.get_node(contents).unwrap().children.is_empty()));
}

fn element_children(doc: &BaseDocument, node: NodeId) -> Vec<NodeId> {
    let children = &doc.get_node(node).unwrap().children;
    children
        .iter()
        .copied()
        .filter(|&child| doc.get_node(child).unwrap().is_element())
        .collect()
}

/// Children as the probe prints them: `#text`, `!` for a comment, or the
/// element's local name.
fn describe_children(doc: &BaseDocument, node: NodeId) -> Vec<String> {
    let children = &doc.get_node(node).unwrap().children;
    children
        .iter()
        .map(|&child| match &doc.get_node(child).unwrap().data {
            NodeData::Text(text) => format!("#{}", text.content.as_str_lossy()),
            NodeData::Comment { .. } => "!".to_owned(),
            NodeData::Element(element) => element.name.local.to_string(),
            _ => "?".to_owned(),
        })
        .collect()
}

#[test]
fn a_declarative_shadow_root_past_the_limit_keeps_its_content() {
    // The template is never attached, so it has no parent to take its
    // content past the limit: the shadow root keeps it, as in Blink.
    let doc = parse(&format!(
        "<!doctype html><body>{}<div id=host><template shadowrootmode=open><p id=inside>shadow</p></template></div>",
        "<div>".repeat(600)
    ));
    let host = q(&doc, "#host");
    let root = doc
        .shadow_root_of(host)
        .expect("the host has a shadow root");
    assert_eq!(describe_children(&doc, root), ["p"]);
    assert!(describe_children(&doc, host).is_empty());
}
