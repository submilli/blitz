//! Live DOM filter snapshots. SVG parsing and unit resolution stay in the engine.

use crate::canvas_svg_attributes::{Attribute, chrome_attribute};
use crate::canvas_svg_units::CanvasSvgUnits;
use crate::{BaseDocument, CanvasFilter, CanvasFilters, NodeId};
use std::borrow::Cow;
use std::sync::Arc;
use style_traits::ToCss;

const MAX_VISITS: usize = 4096;
const MAX_NODES: usize = 64;
const MAX_MARKUP: usize = 16 * 1024;
const SNAPSHOT_ID: &str = "canvas-filter";

/// A draw-time snapshot contains no DOM handles and cannot retain removed nodes.
#[derive(Clone, Debug, Default)]
pub struct CanvasSvgFilter {
    markup: String,
    /// What the parser does not carry, per filter primitive in document order.
    primitives: Vec<Primitive>,
    /// `primitiveUnits="objectBoundingBox"` on the filter element.
    bounding_box_units: bool,
}

/// Page-controlled graph or serialization limits are checked before retaining data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CanvasSvgLimit;

/// Facts about one primitive that the parsed filter does not record.
#[derive(Clone, Copy, Debug, Default)]
struct Primitive {
    /// Its own paint depends on currentColor. Chrome taints a canvas only when
    /// such paint reaches the filter output, and passes displacement through
    /// when its map is tainted.
    tainted: bool,
    /// A nonpositive width or height made its subregion empty.
    empty_region: bool,
}

/// A parsed snapshot paired with what the parser does not carry.
#[derive(Clone, Debug)]
pub struct ResolvedCanvasSvgFilter {
    pub filter: Arc<usvg::filter::Filter>,
    pub units: CanvasSvgUnits,
    primitives: Vec<Primitive>,
}

impl ResolvedCanvasSvgFilter {
    /// Whether primitive `index` (in `filter.primitives()`) paints with currentColor.
    /// A primitive without a recorded flag counts as tainted, so a disagreement
    /// between the snapshot and the parser can never expose pixels.
    pub fn tainted(&self, index: usize) -> bool {
        self.primitives
            .get(index)
            .is_none_or(|primitive| primitive.tainted)
    }

    /// Whether primitive `index` has an empty subregion, which Chrome renders as
    /// transparent black.
    pub fn empty_region(&self, index: usize) -> bool {
        self.primitives
            .get(index)
            .is_some_and(|primitive| primitive.empty_region)
    }
}

impl CanvasSvgFilter {
    /// Resolve filterUnits/primitiveUnits against the geometric source bounds.
    /// The parser performs no I/O: images and resource references are disabled.
    pub fn resolve(&self, bounds: [f64; 4], viewport: [u16; 2]) -> Option<ResolvedCanvasSvgFilter> {
        if !bounds.into_iter().all(|n| n.is_finite() && n.abs() <= 1e6)
            || bounds[2] <= bounds[0]
            || bounds[3] <= bounds[1]
        {
            return None;
        }
        let [x, y, right, bottom] = bounds;
        let mut svg = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\"><defs>{}</defs><rect x=\"{x}\" y=\"{y}\" width=\"{}\" height=\"{}\" filter=\"url(#",
            viewport[0],
            viewport[1],
            self.markup,
            right - x,
            bottom - y
        );
        svg.push_str(SNAPSHOT_ID);
        svg.push_str(")\"/></svg>");
        let options = usvg::Options {
            image_href_resolver: usvg::ImageHrefResolver {
                resolve_data: Box::new(|_, _, _| None),
                resolve_string: Box::new(|_, _| None),
            },
            ..usvg::Options::default()
        };
        let tree = usvg::Tree::from_str(&svg, &options).ok()?;
        let filter = tree
            .filters()
            .iter()
            .find(|f| f.id() == SNAPSHOT_ID)?
            .clone();
        Some(ResolvedCanvasSvgFilter {
            filter,
            units: CanvasSvgUnits::new(bounds, self.bounding_box_units),
            primitives: self.primitives.clone(),
        })
    }
}

impl BaseDocument {
    /// Refresh every URL immediately before a draw, including saved/restored lists.
    /// Unresolved/external URLs remain unapplied and never trigger ambient fetching.
    pub fn resolve_canvas_svg_filters(
        &self,
        owner: NodeId,
        filters: &mut CanvasFilters,
    ) -> Result<(), CanvasSvgLimit> {
        let mut snapshots = Vec::with_capacity(filters.operations.len());
        for operation in &filters.operations {
            snapshots.push(match operation {
                CanvasFilter::Reference(url) => self.canvas_svg_filter(owner, url)?,
                _ => None,
            });
        }
        filters.svg = snapshots;
        Ok(())
    }

    fn canvas_svg_filter(
        &self,
        owner: NodeId,
        reference: &str,
    ) -> Result<Option<CanvasSvgFilter>, CanvasSvgLimit> {
        let Ok(url) = url::Url::parse(reference) else {
            return Ok(None);
        };
        let document_url = self.document_url(owner);
        if url[..url::Position::AfterQuery] != document_url[..url::Position::AfterQuery] {
            return Ok(None);
        }
        let Some(fragment) = url.fragment() else {
            return Ok(None);
        };
        if fragment.len() > 4096 {
            return Err(CanvasSvgLimit);
        }
        let id = percent_encoding::percent_decode_str(fragment).decode_utf8_lossy();
        let Some(filter) = self.find_canvas_filter(owner, &id)? else {
            return Ok(None);
        };
        self.snapshot_canvas_filter(filter)
    }

    fn find_canvas_filter(
        &self,
        owner: NodeId,
        id: &str,
    ) -> Result<Option<NodeId>, CanvasSvgLimit> {
        let mut visits = 0;
        let mut tree = owner;
        while let Some(parent) = self.nodes[tree].parent {
            visits += 1;
            if visits > MAX_VISITS {
                return Err(CanvasSvgLimit);
            }
            tree = parent;
        }
        let root = if self.nodes[tree].is_shadow_root() {
            tree
        } else {
            self.node_document(owner)
        };
        let mut pending = vec![root];
        while let Some(node_id) = pending.pop() {
            visits += 1;
            if visits > MAX_VISITS {
                return Err(CanvasSvgLimit);
            }
            let node = &self.nodes[node_id];
            if let Some(element) = node.element_data() {
                for attr in element.attrs() {
                    visits += 1;
                    if visits > MAX_VISITS {
                        return Err(CanvasSvgLimit);
                    }
                    if attr.name.ns.as_ref().is_empty()
                        && attr.name.local.as_ref() == "id"
                        && attr.value.utf16_len_bound() <= 4096
                        && attr.value.as_str_lossy() == id
                    {
                        return Ok(Some(node_id));
                    }
                }
            }
            if pending.len() + node.children.len() > MAX_VISITS {
                return Err(CanvasSvgLimit);
            }
            pending.extend(node.children.iter().rev().copied());
        }
        Ok(None)
    }

    fn snapshot_canvas_filter(
        &self,
        filter: NodeId,
    ) -> Result<Option<CanvasSvgFilter>, CanvasSvgLimit> {
        let Some(element) = self.nodes[filter].element_data() else {
            return Ok(None);
        };
        if element.name.local.as_ref() != "filter"
            || element.name.ns.as_ref() != "http://www.w3.org/2000/svg"
        {
            return Ok(None);
        }
        let mut writer = Writer::default();
        let mut pending = vec![(filter, false)];
        let mut nodes = 0;
        while let Some((node_id, closing)) = pending.pop() {
            let node = &self.nodes[node_id];
            let Some(element) = node.element_data() else {
                continue;
            };
            let tag = element.name.local.as_ref();
            if element.name.ns.as_ref() != "http://www.w3.org/2000/svg"
                || matches!(tag, "title" | "desc" | "metadata")
            {
                continue;
            }
            if closing {
                writer.push(&format!("</{tag}>"))?;
                continue;
            }
            nodes += 1;
            if nodes > MAX_NODES || pending.len() + node.children.len() > MAX_NODES * 2 {
                return Err(CanvasSvgLimit);
            }
            // No geometry, scripts, nested documents or resource-bearing primitives
            // can enter the synthetic parser document.
            if !allowed_tag(tag) {
                return Ok(None);
            }
            writer.push(&format!("<{tag}"))?;
            if node_id == filter {
                writer.push(&format!(" id=\"{SNAPSHOT_ID}\""))?;
            }
            let Written { empty_region } =
                match writer.attributes(element, tag, node_id == filter)? {
                    Element::Written(written) => written,
                    Element::Unresolvable => return Ok(None),
                };
            let tainted = match node
                .primary_styles()
                .map(|s| s.clone())
                .or_else(|| self.resolve_undisplayed_style(node_id))
            {
                Some(style) => writer.computed_style(&style, tag)?,
                None => false,
            };
            if node.parent == Some(filter) && is_primitive(tag) {
                writer.snapshot.primitives.push(Primitive {
                    tainted,
                    empty_region,
                });
            }
            writer.push(">")?;
            pending.push((node_id, true));
            pending.extend(node.children.iter().rev().map(|child| (*child, false)));
        }
        Ok(Some(writer.snapshot))
    }
}

/// How an element's attributes entered the snapshot.
enum Element {
    Written(Written),
    /// A resource reference or an empty filter region leaves the graph unresolved.
    Unresolvable,
}

struct Written {
    empty_region: bool,
}

/// Serializes one snapshot within its markup and attribute budgets.
#[derive(Default)]
struct Writer {
    snapshot: CanvasSvgFilter,
    attributes: usize,
}

impl Writer {
    fn push(&mut self, text: &str) -> Result<(), CanvasSvgLimit> {
        if self.snapshot.markup.len() + text.len() > MAX_MARKUP {
            return Err(CanvasSvgLimit);
        }
        self.snapshot.markup.push_str(text);
        Ok(())
    }

    fn push_escaped(&mut self, text: &str) -> Result<(), CanvasSvgLimit> {
        if self.snapshot.markup.len() + text.len() * 6 > MAX_MARKUP {
            return Err(CanvasSvgLimit);
        }
        escape(&mut self.snapshot.markup, text);
        Ok(())
    }

    /// Write the element's own attributes as Chrome reads them. Presentation
    /// colors come from computed style instead.
    fn attributes(
        &mut self,
        element: &crate::node::ElementData,
        tag: &str,
        is_filter: bool,
    ) -> Result<Element, CanvasSvgLimit> {
        let mut written = Written {
            empty_region: false,
        };
        for attr in element.attrs() {
            self.attributes += 1;
            if self.attributes > MAX_VISITS {
                return Err(CanvasSvgLimit);
            }
            let name = attr.name.local.as_ref();
            if (is_filter && name == "id")
                || !attr.name.ns.as_ref().is_empty()
                || name == "xmlns"
                || name.starts_with("on")
                || matches!(
                    name,
                    "color"
                        | "style"
                        | "flood-color"
                        | "flood-opacity"
                        | "lighting-color"
                        | "color-interpolation-filters"
                )
            {
                continue;
            }
            if name == "href" {
                return Ok(Element::Unresolvable);
            }
            if attr.value.utf16_len_bound() > 4096 {
                return Err(CanvasSvgLimit);
            }
            let value = attr.value.as_str_lossy();
            if value.len() > 4096 {
                return Err(CanvasSvgLimit);
            }
            if is_filter && name == "primitiveUnits" {
                self.snapshot.bounding_box_units = value == "objectBoundingBox";
            }
            let value: Cow<'_, str> = match chrome_attribute(tag, name, value) {
                Attribute::Keep(value) => Cow::Borrowed(value),
                Attribute::Replace(value) => Cow::Owned(value),
                Attribute::Omit => continue,
                // Chrome ignores a filter without area, like a missing reference.
                Attribute::EmptySize if is_filter => return Ok(Element::Unresolvable),
                Attribute::EmptySize => {
                    written.empty_region = true;
                    continue;
                }
            };
            self.push(&format!(" {name}=\""))?;
            self.push_escaped(&value)?;
            self.push("\"")?;
        }
        Ok(Element::Written(written))
    }

    /// Write computed colors and interpolation as a style attribute. Returns
    /// whether the element's own paint depends on currentColor.
    fn computed_style(
        &mut self,
        style: &style::properties::ComputedValues,
        tag: &str,
    ) -> Result<bool, CanvasSvgLimit> {
        let color = style.clone_color();
        let flood = style.clone_flood_color();
        let lighting = style.clone_lighting_color();
        let tainted = match tag {
            "feFlood" | "feDropShadow" => !flood.is_absolute(),
            "feDiffuseLighting" | "feSpecularLighting" => !lighting.is_absolute(),
            _ => false,
        };
        let flood = svg_color(&flood.resolve_to_absolute(&color));
        let lighting = svg_color(&lighting.resolve_to_absolute(&color));
        let interpolation = style.clone_color_interpolation_filters().to_css_string();
        let interpolation = if interpolation.eq_ignore_ascii_case("srgb") {
            "sRGB"
        } else {
            "linearRGB"
        };
        self.push(" style=\"color:")?;
        self.push_escaped(&svg_color(&color))?;
        self.push(";flood-color:")?;
        self.push_escaped(&flood)?;
        self.push(";lighting-color:")?;
        self.push_escaped(&lighting)?;
        self.push(&format!(
            ";flood-opacity:{};color-interpolation-filters:{interpolation}\"",
            style.clone_flood_opacity()
        ))?;
        Ok(tainted)
    }
}

fn svg_color(color: &style::color::AbsoluteColor) -> String {
    use crate::util::ToColorColor;
    let color = color.as_color_color();
    let [r, g, b, a] = color.components;
    format!(
        "rgba({},{},{},{})",
        (r.clamp(0.0, 1.0) * 255.0).round(),
        (g.clamp(0.0, 1.0) * 255.0).round(),
        (b.clamp(0.0, 1.0) * 255.0).round(),
        a.clamp(0.0, 1.0)
    )
}

fn allowed_tag(tag: &str) -> bool {
    matches!(
        tag,
        "filter"
            | "feColorMatrix"
            | "feComponentTransfer"
            | "feFuncR"
            | "feFuncG"
            | "feFuncB"
            | "feFuncA"
            | "feGaussianBlur"
            | "feOffset"
            | "feFlood"
            | "feComposite"
            | "feBlend"
            | "feMerge"
            | "feMergeNode"
            | "feDropShadow"
            | "feMorphology"
            | "feConvolveMatrix"
            | "feDisplacementMap"
            | "feTurbulence"
            | "feTile"
            | "feDiffuseLighting"
            | "feSpecularLighting"
            | "feDistantLight"
            | "fePointLight"
            | "feSpotLight"
    )
}

/// Filter children that the parser turns into graph primitives, in document order.
fn is_primitive(tag: &str) -> bool {
    tag.starts_with("fe")
        && !matches!(
            tag,
            "feFuncR"
                | "feFuncG"
                | "feFuncB"
                | "feFuncA"
                | "feMergeNode"
                | "feDistantLight"
                | "fePointLight"
                | "feSpotLight"
        )
}

fn escape(output: &mut String, text: &str) {
    for ch in text.chars() {
        output.push_str(match ch {
            '&' => "&amp;",
            '<' => "&lt;",
            '>' => "&gt;",
            '"' => "&quot;",
            _ => {
                output.push(ch);
                continue;
            }
        });
    }
}

#[cfg(test)]
mod tests;
