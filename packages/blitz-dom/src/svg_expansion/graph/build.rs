//! Which elements usvg's `svgtree` builds from a parsed document: those it
//! knows where they stand, and `<use>` targets wherever they stand.

use super::{Element, Refused};
use crate::svg_expansion::SVG_NS;
use crate::svg_expansion::styles::Styles;
use std::collections::HashMap;
use usvg::roxmltree::{self, Node};

/// The elements usvg knows (its `EId`), besides `<style>`, which it skips.
const KNOWN: &[&str] = &[
    "a",
    "circle",
    "clipPath",
    "defs",
    "ellipse",
    "feBlend",
    "feColorMatrix",
    "feComponentTransfer",
    "feComposite",
    "feConvolveMatrix",
    "feDiffuseLighting",
    "feDisplacementMap",
    "feDistantLight",
    "feDropShadow",
    "feFlood",
    "feFuncA",
    "feFuncB",
    "feFuncG",
    "feFuncR",
    "feGaussianBlur",
    "feImage",
    "feMerge",
    "feMergeNode",
    "feMorphology",
    "feOffset",
    "fePointLight",
    "feSpecularLighting",
    "feSpotLight",
    "feTile",
    "feTurbulence",
    "filter",
    "g",
    "image",
    "line",
    "linearGradient",
    "marker",
    "mask",
    "path",
    "pattern",
    "polygon",
    "polyline",
    "radialGradient",
    "rect",
    "stop",
    "svg",
    "switch",
    "symbol",
    "text",
    "textPath",
    "tref",
    "tspan",
    "use",
];

/// Builds the elements of a document as usvg's `svgtree` does.
pub(super) struct Builder<'s, 'a, 'input> {
    pub styles: &'s Styles<'a>,
    pub elements: Vec<Element<'a, 'input>>,
    pub index: HashMap<roxmltree::NodeId, usize>,
    /// Declarations kept so far, and how many may be.
    pub declarations: usize,
    pub max_declarations: usize,
    pub max_depth: usize,
}

impl<'a, 'input> Builder<'_, 'a, 'input> {
    /// Build `root` and what usvg builds below it, as a root of its own;
    /// `copied_only` when usvg builds it only as a `<use>` copy.
    pub(super) fn subtree(
        &mut self,
        root: Node<'a, 'input>,
        copied_only: bool,
    ) -> Result<(), Refused> {
        let mut pending = vec![(root, None, 1usize, false)];
        while let Some((node, parent, depth, in_text)) = pending.pop() {
            let parent_tag = parent.map(|p: usize| self.elements[p].tag);
            let Some(tag) = built_tag(node, parent_tag, in_text) else {
                continue;
            };
            if depth > self.max_depth {
                return Err(Refused);
            }
            // Never past the maximum: each element keeps at most what is left.
            let allowance = self.max_declarations - self.declarations;
            let declarations = self.styles.declarations(node, allowance).ok_or(Refused)?;
            self.declarations += declarations.len();
            let id = self.elements.len();
            self.index.insert(node.id(), id);
            if let Some(parent) = parent {
                self.elements[parent].children.push(id);
            }
            self.elements.push(Element {
                node,
                tag,
                parent,
                children: Vec::new(),
                declarations,
                once: Vec::new(),
                inherited: Default::default(),
                instance: None,
                instanced_by: Vec::new(),
                copied_only,
            });
            // `<use>` content is not built; its target is copied instead.
            if tag != "use" {
                let in_text = tag == "text" || (in_text && tag != "tref");
                let children = node.children().filter(Node::is_element);
                pending.extend(
                    children
                        .rev()
                        .map(|child| (child, Some(id), depth + 1, in_text)),
                );
            }
        }
        Ok(())
    }
}

/// The element each ID names for `<use>` and `<tref>`: among all elements,
/// the first winning.
pub(super) fn first_ids<'a, 'input>(
    doc: &'a roxmltree::Document<'input>,
) -> HashMap<&'a str, Node<'a, 'input>> {
    let mut first = HashMap::new();
    for node in doc.descendants() {
        if let Some(id) = node.attribute("id") {
            first.entry(id).or_insert(node);
        }
    }
    first
}

/// The tag usvg builds `node` as, below an element of tag `parent`, or in
/// text content; `None` when it skips the element and its subtree.
fn built_tag<'a>(node: Node<'a, '_>, parent: Option<&str>, in_text: bool) -> Option<&'a str> {
    if !matches!(node.tag_name().namespace(), None | Some(SVG_NS)) {
        return None;
    }
    let tag = node.tag_name().name();
    if !KNOWN.contains(&tag) {
        return None;
    }
    if !in_text {
        return Some(tag);
    }
    // Text content keeps only spans (links among them), and a `textPath`
    // only directly in its `text`.
    let span =
        matches!(tag, "tspan" | "tref" | "a") || (tag == "textPath" && parent == Some("text"));
    span.then_some(tag)
}
