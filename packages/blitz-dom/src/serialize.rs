//! HTML fragment serialization.
//!
//! Implements the "serializing HTML fragments" algorithm from the HTML
//! standard, which backs the `innerHTML` and `outerHTML` getters:
//! <https://html.spec.whatwg.org/multipage/parsing.html#serialising-html-fragments>

use markup5ever::{local_name, ns};

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
pub fn inner_html(node: &Node) -> String {
    let mut out = String::new();
    write_children(node, &mut out);
    out
}

/// `outerHTML`: serialize `node` itself followed by its descendants.
pub fn outer_html(node: &Node) -> String {
    let mut out = String::new();
    write_node(node, &mut out);
    out
}

fn write_children(node: &Node, out: &mut String) {
    let tree = node.tree();
    let children = match node.element_data().and_then(|el| el.template_contents) {
        Some(contents) => &tree[contents].children,
        None => &node.children,
    };
    for &child in children {
        write_node(&tree[child], out);
    }
}

fn write_node(node: &Node, out: &mut String) {
    match &node.data {
        NodeData::Element(el) | NodeData::AnonymousBlock(el) => {
            // AnonymousBlocks are layout artefacts, not part of the DOM.
            if matches!(node.data, NodeData::AnonymousBlock(_)) {
                write_children(node, out);
                return;
            }
            let tag = qualified_tag_name(&el.name);
            out.push('<');
            out.push_str(&tag);
            for attr in el.attrs() {
                out.push(' ');
                write_attribute_name(&attr.name, out);
                out.push_str("=\"");
                escape(&attr.value, true, out);
                out.push('"');
            }
            out.push('>');
            if el.name.ns == ns!(html) && is_void(&el.name.local) {
                return;
            }
            write_children(node, out);
            out.push_str("</");
            out.push_str(&tag);
            out.push('>');
        }
        NodeData::Text(text) => {
            let raw = node
                .parent
                .and_then(|p| node.tree()[p].element_data())
                .is_some_and(|el| el.name.ns == ns!(html) && is_raw_text_parent(&el.name.local));
            if raw {
                out.push_str(&text.content);
            } else {
                escape(&text.content, false, out);
            }
        }
        NodeData::Comment { contents } => {
            out.push_str("<!--");
            out.push_str(contents);
            out.push_str("-->");
        }
        NodeData::ProcessingInstruction { target, contents } => {
            out.push_str("<?");
            out.push_str(target);
            out.push(' ');
            out.push_str(contents);
            out.push('>');
        }
        NodeData::Doctype { name, .. } => {
            out.push_str("<!DOCTYPE ");
            out.push_str(name);
            out.push('>');
        }
        NodeData::Document(_) | NodeData::DocumentFragment => write_children(node, out),
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

fn write_attribute_name(name: &markup5ever::QualName, out: &mut String) {
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

/// "Escaping a string": `&`, U+00A0, and either `"` (attribute mode) or
/// `<` and `>` (text mode).
fn escape(s: &str, attribute_mode: bool, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '\u{a0}' => out.push_str("&nbsp;"),
            '"' if attribute_mode => out.push_str("&quot;"),
            '<' if !attribute_mode => out.push_str("&lt;"),
            '>' if !attribute_mode => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
}
