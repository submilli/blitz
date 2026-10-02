//! HTML fragment serialization.
//!
//! Implements the "serializing HTML fragments" algorithm from the HTML
//! standard, which backs the `innerHTML` and `outerHTML` getters:
//! <https://html.spec.whatwg.org/multipage/parsing.html#serialising-html-fragments>

use markup5ever::{local_name, ns};

use crate::DomString;
use crate::node::{Node, NodeData};

/// Elements whose start tag is serialized without children or an end tag.
fn is_void(name: &markup5ever::LocalName) -> bool {
    matches!(
        *name,
        local_name!("area")
            | local_name!("base")
            | local_name!("basefont")
            | local_name!("bgsound")
            | local_name!("br")
            | local_name!("col")
            | local_name!("embed")
            | local_name!("frame")
            | local_name!("hr")
            | local_name!("img")
            | local_name!("input")
            | local_name!("keygen")
            | local_name!("link")
            | local_name!("meta")
            | local_name!("param")
            | local_name!("source")
            | local_name!("track")
            | local_name!("wbr")
    )
}

/// Elements whose text children are serialized without escaping. `noscript`
/// is included because documents here always have scripting enabled.
fn is_raw_text_parent(name: &markup5ever::LocalName) -> bool {
    matches!(
        *name,
        local_name!("style")
            | local_name!("script")
            | local_name!("xmp")
            | local_name!("iframe")
            | local_name!("noembed")
            | local_name!("noframes")
            | local_name!("plaintext")
            | local_name!("noscript")
    )
}

/// `innerHTML`: serialize the children of `node` (the template contents for
/// a `<template>` element).
pub fn inner_html(node: &Node) -> DomString {
    let mut steps = Vec::new();
    push_children(node, &mut steps);
    write_steps(steps, DomString::new(), &ShadowOptions::default())
}

/// `outerHTML`: serialize `node` itself followed by its descendants.
pub fn outer_html(node: &Node) -> DomString {
    write_steps(
        vec![Step::Node(node)],
        DomString::new(),
        &ShadowOptions::default(),
    )
}

/// Serialization work still to do, popped from the end: a node to write,
/// or the end tag of an element whose children come first. An explicit
/// stack, so a deep tree cannot overflow the native stack.
enum Step<'a> {
    Node(&'a Node),
    EndTag(String),
    Shadow(&'a Node),
}

#[derive(Default)]
struct ShadowOptions<'a> {
    serializable: bool,
    roots: &'a [crate::NodeId],
}

impl crate::BaseDocument {
    /// HTML getHTML fragment serialization, including explicitly requested roots.
    pub fn get_html(
        &self,
        id: crate::NodeId,
        serializable: bool,
        roots: &[crate::NodeId],
    ) -> DomString {
        let Some(node) = self.get_node(id) else {
            return DomString::new();
        };
        let options = ShadowOptions {
            serializable,
            roots,
        };
        let mut steps = Vec::new();
        push_children(node, &mut steps);
        push_shadow(node, &mut steps, &options);
        write_steps(steps, DomString::new(), &options)
    }
}

fn push_shadow<'a>(node: &'a Node, steps: &mut Vec<Step<'a>>, options: &ShadowOptions<'_>) {
    let Some(root) = node.element_data().and_then(|el| el.shadow_root) else {
        return;
    };
    let root = &node.tree()[root];
    if root
        .shadow_root_data
        .as_ref()
        .is_some_and(|data| options.serializable && data.serializable)
        || options.roots.contains(&root.id)
    {
        steps.push(Step::Shadow(root));
    }
}

fn write_steps(
    mut steps: Vec<Step<'_>>,
    mut out: DomString,
    options: &ShadowOptions<'_>,
) -> DomString {
    while let Some(step) = steps.pop() {
        match step {
            Step::Node(node) => write_node(node, &mut steps, &mut out, options),
            Step::Shadow(root) => {
                let data = root
                    .shadow_root_data
                    .as_ref()
                    .expect("shadow serialization step");
                out.push_str(if data.open {
                    "<template shadowrootmode=\"open\""
                } else {
                    "<template shadowrootmode=\"closed\""
                });
                for (enabled, name) in [
                    (data.delegates_focus, "shadowrootdelegatesfocus"),
                    (data.serializable, "shadowrootserializable"),
                ] {
                    if enabled {
                        out.push(' ');
                        out.push_str(name);
                        out.push_str("=\"\"");
                    }
                }
                if data.manual_slots {
                    out.push_str(" shadowrootslotassignment=\"manual\"");
                }
                if data.clonable {
                    out.push_str(" shadowrootclonable=\"\"");
                }
                out.push('>');
                steps.push(Step::EndTag("template".to_string()));
                push_children(root, &mut steps);
            }
            Step::EndTag(tag) => {
                out.push_str("</");
                out.push_str(&tag);
                out.push('>');
            }
        }
    }
    out
}

/// Queue `node`'s children (the template contents for a `<template>`) so
/// they are written in order.
fn push_children<'a>(node: &'a Node, steps: &mut Vec<Step<'a>>) {
    let tree = node.tree();
    let children = match node.element_data().and_then(|el| el.template_contents) {
        Some(contents) => &tree[contents].children,
        None => &node.children,
    };
    steps.extend(children.iter().rev().map(|&child| Step::Node(&tree[child])));
}

/// Write `node`'s own markup, and queue its children and end tag.
fn write_node<'a>(
    node: &'a Node,
    steps: &mut Vec<Step<'a>>,
    out: &mut DomString,
    options: &ShadowOptions<'_>,
) {
    match &node.data {
        NodeData::Element(el) | NodeData::AnonymousBlock(el) => {
            // AnonymousBlocks are layout artefacts, not part of the DOM.
            if matches!(node.data, NodeData::AnonymousBlock(_)) {
                push_children(node, steps);
                return;
            }
            let tag = qualified_tag_name(&el.name);
            out.push('<');
            out.push_str(&tag);
            if let Some(is) = &el.custom_element_is
                && !el.has_attr(markup5ever::LocalName::from("is"))
            {
                out.push_str(" is=\"");
                escape_dom(&is.as_str().into(), true, out);
                out.push('"');
            }
            for attr in el.attrs() {
                out.push(' ');
                write_attribute_name(&attr.name, out);
                out.push_str("=\"");
                escape_dom(&attr.value, true, out);
                out.push('"');
            }
            out.push('>');
            if el.name.ns == ns!(html) && is_void(&el.name.local) {
                return;
            }
            steps.push(Step::EndTag(tag));
            push_children(node, steps);
            push_shadow(node, steps, options);
        }
        NodeData::Text(text) => {
            let raw = node
                .parent
                .and_then(|p| node.tree()[p].element_data())
                .is_some_and(|el| el.name.ns == ns!(html) && is_raw_text_parent(&el.name.local));
            if raw {
                out.push_dom(&text.content);
            } else {
                escape_text(&text.content, out);
            }
        }
        NodeData::Comment { contents } => {
            out.push_str("<!--");
            out.push_dom(contents);
            out.push_str("-->");
        }
        NodeData::ProcessingInstruction { target, contents } => {
            out.push_str("<?");
            out.push_str(target);
            out.push(' ');
            out.push_dom(contents);
            out.push('>');
        }
        NodeData::Doctype { name, .. } => {
            out.push_str("<!DOCTYPE ");
            out.push_str(name);
            out.push('>');
        }
        NodeData::Document(_) | NodeData::DocumentFragment => push_children(node, steps),
    }
}

/// The tag name as serialized: the local name for HTML, SVG and MathML
/// elements, otherwise the qualified name.
fn qualified_tag_name(name: &markup5ever::QualName) -> String {
    let local = name.local.to_string();
    if name.ns == ns!(html) || name.ns == ns!(svg) || name.ns == ns!(mathml) {
        return local;
    }
    match &name.prefix {
        Some(prefix) => format!("{prefix}:{local}"),
        None => local,
    }
}

fn write_attribute_name(name: &markup5ever::QualName, out: &mut DomString) {
    if name.ns == ns!() {
        out.push_str(&name.local);
    } else if name.ns == ns!(xml) {
        out.push_str("xml:");
        out.push_str(&name.local);
    } else if name.ns == ns!(xmlns) {
        if name.local != local_name!("xmlns") {
            out.push_str("xmlns:");
        }
        out.push_str(&name.local);
    } else if name.ns == ns!(xlink) {
        out.push_str("xlink:");
        out.push_str(&name.local);
    } else {
        if let Some(prefix) = &name.prefix {
            out.push_str(prefix);
            out.push(':');
        }
        out.push_str(&name.local);
    }
}

/// Escape markup characters while copying every other UTF-16 code unit verbatim.
fn escape_text(text: &DomString, out: &mut DomString) {
    escape_dom(text, false, out);
}

fn escape_dom(text: &DomString, attribute_mode: bool, out: &mut DomString) {
    let units = text.to_utf16();
    let mut start = 0;
    for (i, &unit) in units.iter().enumerate() {
        let escaped = match unit {
            38 => "&amp;",
            160 => "&nbsp;",
            34 if attribute_mode => "&quot;",
            60 if !attribute_mode => "&lt;",
            62 if !attribute_mode => "&gt;",
            _ => continue,
        };
        out.push_units(&units[start..i]);
        out.push_str(escaped);
        start = i + 1;
    }
    out.push_units(&units[start..]);
}
