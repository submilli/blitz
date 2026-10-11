//! The painted and placed markups of a [`Plan`].

use super::MAX_ATTRIBUTE_LEN;
use super::MAX_ELEMENT_MARKUP;
use super::elements::unmarked_shape;
use super::elements::{attribute_reference, image_href, included, xml_name};
use super::plan::{Kind, Plan, Slot};
use super::style::{StyleWriter, Values, from_style};
use super::viewport::view_box_size;
use crate::canvas_svg_filter::{CanvasSvgLimit, push_bounded, push_escaped_bounded};
use crate::node::ElementData;
use crate::{BaseDocument, NodeId};
use std::str::FromStr;

/// Serialized markup of one reference.
pub(super) struct Markup {
    /// What Blink paints: no transforms, `<use>` offsets or nested viewports.
    pub painted: String,
    /// The same with them, to measure the bounds; `None` when no element had
    /// any, so the painted markup measures them.
    pub placed: Option<String>,
    /// Bytes of painted markup each source wrote, which the parser copies
    /// into every build of it.
    pub bytes: Vec<usize>,
}

/// The size written for an empty or missing viewport.
const EMPTY_VIEWPORT: f32 = 1e-6;

/// Attributes Blink applies through paint property nodes, which
/// `PaintResourceSubtree` leaves out. `overflow` is written as `visible` on
/// viewports, and on markers that ask for it.
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
    // An empty side resolves percentages to nothing, as in Blink; the parser
    // needs a positive size.
    let [width, height] = viewport
        .unwrap_or_default()
        .map(|side| if side > 0.0 { side } else { EMPTY_VIEWPORT });
    writer.push(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\">"
    ))?;
    let mut bytes = Vec::with_capacity(plan.sources.len());
    for index in 0..plan.sources.len() {
        if index == 1 {
            writer.push("<defs>")?;
        }
        let start = writer.painted.len();
        writer.source(index)?;
        bytes.push(writer.painted.len() - start);
    }
    if plan.sources.len() > 1 {
        writer.push("</defs>")?;
    }
    writer.push("</svg>")?;
    Ok(Markup {
        placed: writer.transformed.then_some(writer.placed),
        painted: writer.painted,
        bytes,
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
            let element = node.element_data().expect("included nodes are elements");
            let styled = node.primary_styles();
            let unstyled = styled.is_none();
            self.attributes(id, tag, element, unstyled)?;
            if let Some(style) = styled {
                let parent_style =
                    parent.and_then(|parent: NodeId| self.doc.nodes[parent].primary_styles());
                let values = match (kind, &parent_style) {
                    (Kind::Instance, _) => Values::Declared,
                    (_, Some(parent)) => Values::BelowParent(parent),
                    (_, None) => Values::All,
                };
                let plan = self.plan;
                let copy = |slot| plan.reference(id, slot).map(copy_id);
                for (name, value) in self.style.write(&style, tag, values, copy)? {
                    self.push(&format!(" {name}=\""))?;
                    self.push_escaped(&value)?;
                    self.push("\"")?;
                }
            }
            if unstyled && unmarked_shape(tag) {
                self.push(" marker-start=\"none\" marker-mid=\"none\" marker-end=\"none\"")?;
            }
            // Blink does not clip to nested viewports while painting, but
            // clips markers as their `overflow` says (`SVGShapePainter`).
            if matches!(tag, "svg" | "symbol") || (tag == "marker" && visible_overflow(element)) {
                self.push(" overflow=\"visible\"")?;
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

    /// Write the element's own attributes. IDs, classes, styles, handlers and
    /// effects stay out, references name copies, and positioning goes to the
    /// placed markup only. An element without styles keeps its presentation
    /// attributes.
    fn attributes(
        &mut self,
        id: NodeId,
        tag: &str,
        element: &ElementData,
        unstyled: bool,
    ) -> Result<(), CanvasSvgLimit> {
        for attr in element.attrs() {
            let local = attr.name.local.as_ref();
            if attr.name.ns != markup5ever::ns!()
                || local.starts_with("on")
                || matches!(local, "id" | "class" | "style" | "xmlns" | "href")
                || EFFECTS.contains(&local)
                || !xml_name(local)
            {
                continue;
            }
            let value = attr.value.as_str_lossy();
            if from_style(local) {
                if unstyled {
                    self.unstyled_presentation(id, tag, local, value)?;
                }
                continue;
            }
            if let Some(painted) = self.painted_position(id, tag, element, local) {
                self.transformed = true;
                if let Some(painted) = painted {
                    self.push_painted(&format!(" {local}=\"{painted}\""))?;
                }
                self.push_placed(&format!(" {local}=\""))?;
                self.push_placed_escaped(value)?;
                self.push_placed("\"")?;
                continue;
            }
            self.push(&format!(" {local}=\""))?;
            self.push_escaped(value)?;
            self.push("\"")?;
        }
        self.href(id, tag, element)
    }

    /// How a positioning attribute appears in the painted markup, or `None`
    /// when it is not one: left out (`Some(None)`), or a nested viewport's
    /// size made its `viewBox` size, so that percentages inside resolve as
    /// Blink resolves them while nothing scales.
    fn painted_position(
        &self,
        id: NodeId,
        tag: &str,
        element: &ElementData,
        local: &str,
    ) -> Option<Option<String>> {
        let axis = match local {
            "width" => 0,
            "height" => 1,
            "transform" => return Some(None),
            "x" | "y" if matches!(tag, "use" | "svg" | "symbol") => return Some(None),
            "viewBox" | "preserveAspectRatio" if matches!(tag, "svg" | "symbol") => {
                return Some(None);
            }
            _ => return None,
        };
        let view_box = match tag {
            "svg" => view_box_size(element)?,
            // A `<use>` sizes the viewport of the `<symbol>` it instances.
            "use" => {
                let copy = self.plan.reference(id, Slot::Href)?;
                let target = self.doc.nodes[self.plan.sources[copy].node].element_data()?;
                if target.name.local.as_ref() != "symbol" {
                    return None;
                }
                view_box_size(target)?
            }
            _ => return None,
        };
        Some(Some(view_box[axis].to_string()))
    }

    /// A presentation attribute of an element without styles: written as it
    /// stands, but for a reference, which names its copy or falls back as a
    /// paint does.
    fn unstyled_presentation(
        &mut self,
        id: NodeId,
        tag: &str,
        local: &str,
        value: &str,
    ) -> Result<(), CanvasSvgLimit> {
        let slot = Slot::of_attribute(local);
        // Markers Blink would not draw are written as `none` elsewhere.
        if local.starts_with("marker") && unmarked_shape(tag) {
            return Ok(());
        }
        // A reference the plan could not read must not reach the parser,
        // which would resolve it itself: like a missing one, it paints
        // nothing.
        let unread = slot.is_some()
            && value.contains("url(")
            && (value.len() > MAX_ATTRIBUTE_LEN || attribute_reference(value).is_none());
        if unread {
            self.push(&format!(" {local}=\"none\""))?;
            return Ok(());
        }
        if value.len() > MAX_ATTRIBUTE_LEN {
            return Ok(());
        }
        let reference = slot.zip(attribute_reference(value));
        let written = match reference {
            Some((slot, (_, fallback))) => {
                let paint = matches!(slot, Slot::Fill | Slot::Stroke);
                let fallback = paint_fallback(&fallback);
                match self.plan.reference(id, slot) {
                    Some(copy) if paint => format!("url(#{}) {fallback}", copy_id(copy)),
                    Some(copy) => format!("url(#{})", copy_id(copy)),
                    None if paint => fallback,
                    None => "none".to_owned(),
                }
            }
            None => value.to_owned(),
        };
        self.push(&format!(" {local}=\""))?;
        self.push_escaped(&written)?;
        self.push("\"")
    }

    /// References name their copies; images keep their own `href`.
    fn href(&mut self, id: NodeId, tag: &str, element: &ElementData) -> Result<(), CanvasSvgLimit> {
        if tag == "image" {
            if let Some(href) = image_href(element) {
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
        self.push_painted(text)?;
        self.push_placed(text)
    }

    fn push_escaped(&mut self, text: &str) -> Result<(), CanvasSvgLimit> {
        push_escaped_bounded(&mut self.painted, text, MAX_ELEMENT_MARKUP)?;
        self.push_placed_escaped(text)
    }

    fn push_painted(&mut self, text: &str) -> Result<(), CanvasSvgLimit> {
        push_bounded(&mut self.painted, text, MAX_ELEMENT_MARKUP)
    }

    fn push_placed(&mut self, text: &str) -> Result<(), CanvasSvgLimit> {
        push_bounded(&mut self.placed, text, MAX_ELEMENT_MARKUP)
    }

    fn push_placed_escaped(&mut self, text: &str) -> Result<(), CanvasSvgLimit> {
        push_escaped_bounded(&mut self.placed, text, MAX_ELEMENT_MARKUP)
    }
}

/// A paint's fallback rewritten from what it parses as: a color, or
/// `currentColor`. Anything else, another reference included, paints nothing.
fn paint_fallback(fallback: &str) -> String {
    let fallback = fallback.trim();
    if fallback.eq_ignore_ascii_case("currentcolor") {
        return "currentColor".to_owned();
    }
    match svgtypes::Color::from_str(fallback) {
        Ok(color) => format!(
            "rgba({},{},{},{})",
            color.red,
            color.green,
            color.blue,
            f32::from(color.alpha) / 255.0
        ),
        Err(_) => "none".to_owned(),
    }
}

/// Whether a `<marker>` element's own `overflow` lets it paint past its
/// viewport.
fn visible_overflow(element: &ElementData) -> bool {
    element
        .attr(markup5ever::local_name!("overflow"))
        .is_some_and(|value| matches!(value.trim(), "visible" | "auto"))
}

/// The ID of copy `index`. Elements keep no IDs of their own, so it is unique.
fn copy_id(index: usize) -> String {
    format!("c{index}")
}
