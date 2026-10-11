//! The painted and placed markups of a [`Plan`].

use super::MAX_ELEMENT_MARKUP;
use super::plan::{Kind, Plan, Slot, included};
use super::style::{StyleWriter, Values};
use crate::canvas_svg_filter::{CanvasSvgLimit, push_bounded, push_escaped_bounded};
use crate::{BaseDocument, NodeId};

/// Serialized markup of one reference.
pub(super) struct Markup {
    /// What Blink paints: no transforms, `<use>` offsets or nested viewports.
    pub painted: String,
    /// The same with them, to measure the bounds; `None` when no element had
    /// any, so the painted markup measures them.
    pub placed: Option<String>,
}

/// Attributes Blink applies through paint property nodes, which
/// `PaintResourceSubtree` leaves out. `overflow` is written as `visible`.
const EFFECTS: [&str; 8] = [
    "opacity",
    "filter",
    "clip-path",
    "mask",
    "mix-blend-mode",
    "isolation",
    "clip",
    "overflow",
];

/// Write `plan`'s sources: the referenced element in a root sized to
/// `viewport` (where its percentages resolve), its copies in `<defs>`.
pub(super) fn write(
    doc: &BaseDocument,
    plan: &Plan,
    viewport: Option<[f32; 2]>,
) -> Result<Markup, CanvasSvgLimit> {
    let mut writer = Writer {
        doc,
        plan,
        style: StyleWriter::new(doc.guard.read()),
        painted: String::new(),
        placed: String::new(),
        transformed: false,
    };
    writer.push("<svg xmlns=\"http://www.w3.org/2000/svg\"")?;
    if let Some([width, height]) = viewport {
        writer.push(&format!(" width=\"{width}\" height=\"{height}\""))?;
    }
    writer.push(">")?;
    writer.source(0)?;
    writer.push("<defs>")?;
    for index in 1..plan.sources.len() {
        writer.source(index)?;
    }
    writer.push("</defs></svg>")?;
    Ok(Markup {
        placed: writer.transformed.then_some(writer.placed),
        painted: writer.painted,
    })
}

struct Writer<'a> {
    doc: &'a BaseDocument,
    plan: &'a Plan,
    style: StyleWriter<'a>,
    painted: String,
    placed: String,
    /// Whether anything was written to the placed markup only.
    transformed: bool,
}

impl Writer<'_> {
    /// Serialize source `index` iteratively; copies carry their number as ID.
    fn source(&mut self, index: usize) -> Result<(), CanvasSvgLimit> {
        let root = self.plan.sources[index].node;
        let kind = self.plan.sources[index].kind;
        let mut pending = vec![(root, false, None)];
        while let Some((id, closing, parent)) = pending.pop() {
            let node = &self.doc.nodes[id];
            if let Some(text) = node.text_data() {
                self.push_escaped(text.content.as_str_lossy())?;
                continue;
            }
            let Some(tag) = included(self.doc, id) else {
                continue;
            };
            if closing {
                self.push(&format!("</{tag}>"))?;
                continue;
            }
            // A `<use>` whose target is missing or cyclic renders nothing.
            if tag == "use" && self.plan.reference(id, Slot::Href).is_none() {
                continue;
            }
            self.push(&format!("<{tag}"))?;
            if id == root && kind != Kind::Root {
                self.push(&format!(" id=\"{}\"", copy_id(index)))?;
            }
            self.attributes(id, tag)?;
            let style = node.primary_styles().expect("included nodes are styled");
            let parent_style =
                parent.and_then(|parent: NodeId| self.doc.nodes[parent].primary_styles());
            let values = match (kind, &parent_style) {
                (Kind::Instance, _) => Values::Declared,
                (_, Some(parent)) => Values::BelowParent(parent),
                (_, None) => Values::All,
            };
            let plan = self.plan;
            let mut declarations = self
                .style
                .write(&style, values, |slot| plan.reference(id, slot).map(copy_id))?;
            // Blink does not clip to nested viewports while painting.
            if matches!(tag, "svg" | "symbol") {
                declarations.push_str("overflow:visible;");
            }
            if !declarations.is_empty() {
                self.push(" style=\"")?;
                self.push_escaped(&declarations)?;
                self.push("\"")?;
            }
            self.push(">")?;
            pending.push((id, true, parent));
            pending.extend(
                node.children
                    .iter()
                    .rev()
                    .map(|child| (*child, false, Some(id))),
            );
        }
        Ok(())
    }

    /// Write the element's own attributes. Styles, IDs and effects stay out,
    /// references name copies, and positioning goes to the placed markup only.
    fn attributes(&mut self, id: NodeId, tag: &str) -> Result<(), CanvasSvgLimit> {
        let element = self.doc.nodes[id]
            .element_data()
            .expect("included nodes are elements");
        for attr in element.attrs() {
            let local = attr.name.local.as_ref();
            if attr.name.ns != markup5ever::ns!()
                || local.starts_with("on")
                || matches!(local, "id" | "class" | "style" | "xmlns" | "href")
                || EFFECTS.contains(&local)
                || super::style::from_style(local)
                || !xml_name(local)
            {
                continue;
            }
            if let Some((_, slot)) = Slot::MARKERS.iter().find(|(name, _)| *name == local) {
                if let Some(copy) = self.plan.reference(id, *slot) {
                    self.push(&format!(" {local}=\"url(#{})\"", copy_id(copy)))?;
                }
                continue;
            }
            let positioning = local == "transform"
                || (tag == "use" && matches!(local, "x" | "y"))
                || (matches!(tag, "svg" | "symbol")
                    && matches!(local, "x" | "y" | "viewBox" | "preserveAspectRatio"));
            let text = format!(" {local}=\"");
            if positioning {
                self.transformed = true;
                self.push_placed(&text)?;
                push_escaped_bounded(
                    &mut self.placed,
                    attr.value.as_str_lossy(),
                    MAX_ELEMENT_MARKUP,
                )?;
                self.push_placed("\"")?;
                continue;
            }
            self.push(&text)?;
            self.push_escaped(attr.value.as_str_lossy())?;
            self.push("\"")?;
        }
        self.href(id, tag, element)
    }

    /// References name their copies; images keep their own `href`.
    fn href(
        &mut self,
        id: NodeId,
        tag: &str,
        element: &crate::node::ElementData,
    ) -> Result<(), CanvasSvgLimit> {
        if tag == "image" {
            if let Some(href) = super::plan::href(element) {
                self.push(" href=\"")?;
                self.push_escaped(href)?;
                self.push("\"")?;
            }
            return Ok(());
        }
        if let Some(copy) = self.plan.reference(id, Slot::Href) {
            self.push(&format!(" href=\"#{}\"", copy_id(copy)))?;
        }
        Ok(())
    }

    /// Append to both markups.
    fn push(&mut self, text: &str) -> Result<(), CanvasSvgLimit> {
        push_bounded(&mut self.painted, text, MAX_ELEMENT_MARKUP)?;
        self.push_placed(text)
    }

    fn push_placed(&mut self, text: &str) -> Result<(), CanvasSvgLimit> {
        push_bounded(&mut self.placed, text, MAX_ELEMENT_MARKUP)
    }

    fn push_escaped(&mut self, text: &str) -> Result<(), CanvasSvgLimit> {
        push_escaped_bounded(&mut self.painted, text, MAX_ELEMENT_MARKUP)?;
        push_escaped_bounded(&mut self.placed, text, MAX_ELEMENT_MARKUP)
    }
}

/// The ID of copy `index`. Elements keep no IDs of their own, so it is unique.
fn copy_id(index: usize) -> String {
    format!("c{index}")
}

/// Whether `name` is an XML name without a prefix: a page may create elements
/// and attributes whose names would break the serialized markup, or name
/// prefixes it never declares.
pub(super) fn xml_name(name: &str) -> bool {
    let start = |ch: char| ch.is_alphabetic() || ch == '_';
    let mut chars = name.chars();
    chars.next().is_some_and(start)
        && chars.all(|ch| start(ch) || ch.is_alphanumeric() || matches!(ch, '-' | '.'))
}
