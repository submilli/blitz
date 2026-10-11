//! Element reference tests, by concern, with shared document builders.

mod bounds;
mod references;
mod styles;
mod viewport;

use crate::canvas_svg_image::CanvasSvgElement;
use crate::{BaseDocument, DocumentConfig, DocumentMutator, NodeId, QualName, qual_name};

/// A document with `css` in a `<style>` and an empty outermost `<svg>`.
pub(super) fn page(css: &str) -> (BaseDocument, NodeId) {
    let mut doc = BaseDocument::new(DocumentConfig {
        base_url: Some("https://example.test/page".into()),
        ..Default::default()
    });
    let root = doc.root_node_id;
    let mut m = doc.mutate();
    let html = m.create_element(qual_name!("html", html), vec![]);
    m.append_children(root, &[html]);
    let style = m.create_element(qual_name!("style", html), vec![]);
    m.set_text_content(style, css);
    let svg = m.create_element(qual_name!("svg", svg), vec![]);
    m.append_children(html, &[style, svg]);
    drop(m);
    (doc, svg)
}

/// Append an SVG element to `parent`.
pub(super) fn add(
    m: &mut DocumentMutator<'_>,
    parent: NodeId,
    tag: &str,
    attrs: &[(&str, &str)],
) -> NodeId {
    let name = QualName {
        prefix: None,
        ns: crate::Namespace::from("http://www.w3.org/2000/svg"),
        local: tag.into(),
    };
    let element = m.create_element(name, vec![]);
    for (attr, value) in attrs {
        let name = QualName {
            prefix: None,
            ns: crate::Namespace::from(""),
            local: (*attr).into(),
        };
        m.set_attribute(element, name, *value);
    }
    m.append_children(parent, &[element]);
    element
}

pub(super) fn capture(doc: &mut BaseDocument, target: NodeId) -> Option<CanvasSvgElement> {
    doc.flush_style_and_layout(0.0);
    doc.capture_svg_element(target).ok().flatten()
}

/// The painted markup of `target`'s reference.
pub(super) fn painted(doc: &mut BaseDocument, target: NodeId) -> String {
    doc.flush_style_and_layout(0.0);
    let plan = super::plan::Plan::new(doc, target).unwrap();
    let viewport = doc.svg_viewport(target).unwrap();
    super::markup::write(doc, &plan, viewport).unwrap().painted
}

/// The paths of a parsed tree, in paint order.
pub(super) fn paths(element: &CanvasSvgElement) -> Vec<usvg::Path> {
    fn collect(group: &usvg::Group, found: &mut Vec<usvg::Path>) {
        for node in group.children() {
            match node {
                usvg::Node::Path(path) => found.push((**path).clone()),
                usvg::Node::Group(group) => collect(group, found),
                _ => {}
            }
        }
    }
    let mut found = Vec::new();
    collect(element.tree.root(), &mut found);
    found
}

pub(super) fn fill_color(path: &usvg::Path) -> Option<(u8, u8, u8)> {
    match path.fill()?.paint() {
        usvg::Paint::Color(color) => Some((color.red, color.green, color.blue)),
        _ => None,
    }
}
