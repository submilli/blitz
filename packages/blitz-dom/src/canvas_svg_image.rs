//! What `feImage` primitives draw, captured with Canvas filter snapshots.
//!
//! A snapshot holds no DOM handles: fetched images are shared decoded data, and
//! a referenced element is serialized into its own parsed tree.

use crate::canvas_svg_filter::{CanvasSvgLimit, push_bounded, push_escaped_bounded, svg_color};
use crate::fe_image::FeImageTarget;
use crate::node::{ImageData, RasterImageData, SvgImageData};
use crate::{BaseDocument, NodeId};
use std::sync::Arc;

/// Nodes and markup of one referenced element's subtree.
const MAX_ELEMENT_NODES: usize = 1024;
const MAX_ELEMENT_MARKUP: usize = 256 * 1024;

/// An `feImage` primitive's source and placement.
#[derive(Clone, Debug)]
pub struct CanvasSvgImage {
    pub source: CanvasSvgImageSource,
    /// Its `preserveAspectRatio`.
    pub aspect: svgtypes::AspectRatio,
}

/// What an `feImage` draws.
#[derive(Clone, Debug)]
pub enum CanvasSvgImageSource {
    /// Transparent black: nothing loaded, failed, or a missing element.
    None,
    /// Decoded straight RGBA8 pixels.
    Raster(RasterImageData),
    /// An SVG document used as an image.
    Svg(SvgImageData),
    /// A same-document element's subtree.
    Element(CanvasSvgElement),
}

/// A referenced element as Chrome paints it (Blink's
/// `FEImage::CreateImageFilterForLayoutObject`): its subtree without the
/// element's own `transform`, culled to its visual rectangle with that
/// transform applied.
#[derive(Clone, Debug)]
pub struct CanvasSvgElement {
    pub tree: Arc<usvg::Tree>,
    /// The painted bounds, stroke included, in the parent's user space.
    pub bounds: usvg::NonZeroRect,
}

impl BaseDocument {
    /// Capture what `fe_image` draws, and whether its result taints a canvas.
    /// As in Blink (`SVGFEImageElement::TaintsOrigin`), only a fetched image
    /// can be readable: element references and missing or unresolvable
    /// `href`s taint, and fetched images taint when they are not CORS
    /// same-origin. A pending or failed fetch draws nothing and does not taint.
    pub(crate) fn snapshot_fe_image(
        &self,
        fe_image: NodeId,
        inline: &mut crate::fe_image::InlineDecodes,
    ) -> Result<(CanvasSvgImage, bool), CanvasSvgLimit> {
        let aspect = self.nodes[fe_image]
            .attr(crate::local_name!("preserveAspectRatio"))
            .and_then(|value| value.parse().ok())
            .unwrap_or_default();
        let (source, tainted) = match self.fe_image_target(fe_image, inline) {
            FeImageTarget::Unfetched => (CanvasSvgImageSource::None, true),
            FeImageTarget::Fragment(id) => (self.referenced_element(fe_image, &id)?, true),
            FeImageTarget::Fetched(None) => (CanvasSvgImageSource::None, false),
            FeImageTarget::Fetched(Some(ImageData::Raster(raster))) => {
                let tainted = !raster.origin_clean;
                (CanvasSvgImageSource::Raster(raster), tainted)
            }
            FeImageTarget::Fetched(Some(ImageData::Svg(svg))) => {
                let tainted = !svg.origin_clean;
                (CanvasSvgImageSource::Svg(svg), tainted)
            }
            FeImageTarget::Fetched(Some(ImageData::None)) => (CanvasSvgImageSource::None, false),
        };
        Ok((CanvasSvgImage { source, aspect }, tainted))
    }

    /// The rendered SVG element `id` names in `fe_image`'s tree, as a parsed
    /// subtree. Like Blink, an element without a box (not SVG, or not
    /// displayed) draws nothing.
    fn referenced_element(
        &self,
        fe_image: NodeId,
        id: &str,
    ) -> Result<CanvasSvgImageSource, CanvasSvgLimit> {
        let Some(target) = self.find_svg_reference(fe_image, id)? else {
            return Ok(CanvasSvgImageSource::None);
        };
        let node = &self.nodes[target];
        let Some(style) = node.primary_styles().map(|style| style.clone()) else {
            return Ok(CanvasSvgImageSource::None);
        };
        let svg = node
            .element_data()
            .is_some_and(|element| element.name.ns == markup5ever::ns!(svg));
        if !svg || style.clone_display().is_none() {
            return Ok(CanvasSvgImageSource::None);
        }
        let mut markup = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" color=\"{}\">",
            svg_color(&style.clone_color())
        );
        // Past the serialization bounds the reference draws nothing (it still
        // taints), rather than failing the whole draw.
        if self.serialize_subtree(target, &mut markup).is_err() {
            return Ok(CanvasSvgImageSource::None);
        }
        markup.push_str("</svg>");
        let options = crate::util::svg_options(&self.svg_fonts);
        let Ok(tree) = usvg::Tree::from_str(&markup, &options) else {
            return Ok(CanvasSvgImageSource::None);
        };
        let transform = node
            .attr(crate::local_name!("transform"))
            .and_then(|value| value.parse::<svgtypes::Transform>().ok())
            .map_or(usvg::Transform::identity(), |t| {
                usvg::Transform::from_row(
                    t.a as f32, t.b as f32, t.c as f32, t.d as f32, t.e as f32, t.f as f32,
                )
            });
        let bounds = tree
            .root()
            .abs_stroke_bounding_box()
            .to_non_zero_rect()
            .and_then(|bounds| bounds.transform(transform));
        Ok(bounds.map_or(CanvasSvgImageSource::None, |bounds| {
            CanvasSvgImageSource::Element(CanvasSvgElement {
                tree: Arc::new(tree),
                bounds,
            })
        }))
    }

    /// Serialize SVG elements and text below `root` iteratively, within the
    /// node and markup bounds, without `root`'s own transform. Scripts, `<use>`
    /// and event handlers never enter it.
    fn serialize_subtree(&self, root: NodeId, out: &mut String) -> Result<(), CanvasSvgLimit> {
        let mut pending = vec![(root, false)];
        let mut nodes = 0;
        while let Some((id, closing)) = pending.pop() {
            let node = &self.nodes[id];
            if let Some(text) = node.text_data() {
                push_escaped(out, text.content.as_str_lossy())?;
                continue;
            }
            let Some(element) = node.element_data() else {
                continue;
            };
            let tag = element.name.local.as_ref();
            // `<use>` can expand to a huge tree in the parser, and names the
            // HTML parser accepts may not be XML: neither enters the markup.
            if element.name.ns != markup5ever::ns!(svg)
                || matches!(tag, "script" | "use")
                || !xml_name(tag)
            {
                continue;
            }
            if closing {
                push(out, &format!("</{tag}>"))?;
                continue;
            }
            nodes += 1;
            if nodes > MAX_ELEMENT_NODES
                || pending.len() + node.children.len() > MAX_ELEMENT_NODES * 2
            {
                return Err(CanvasSvgLimit);
            }
            push(out, &format!("<{tag}"))?;
            for attr in element.attrs() {
                let local = attr.name.local.as_ref();
                if id == root && local == "transform" && attr.name.ns == markup5ever::ns!() {
                    continue;
                }
                let ns = &attr.name.ns;
                let name =
                    if *ns == markup5ever::ns!() && !local.starts_with("on") && local != "xmlns" {
                        local.to_owned()
                    } else if *ns == markup5ever::ns!(xlink) && local == "href" {
                        "xlink:href".to_owned()
                    } else {
                        continue;
                    };
                if name != "xlink:href" && !xml_name(&name) {
                    continue;
                }
                push(out, &format!(" {name}=\""))?;
                push_escaped(out, attr.value.as_str_lossy())?;
                push(out, "\"")?;
            }
            push(out, ">")?;
            pending.push((id, true));
            pending.extend(node.children.iter().rev().map(|child| (*child, false)));
        }
        Ok(())
    }
}

/// Whether `name` is an XML name without a prefix: a page may create elements
/// and attributes whose names would break the serialized markup, or name
/// prefixes it never declares.
fn xml_name(name: &str) -> bool {
    let start = |ch: char| ch.is_alphabetic() || ch == '_';
    let mut chars = name.chars();
    chars.next().is_some_and(start)
        && chars.all(|ch| start(ch) || ch.is_alphanumeric() || matches!(ch, '-' | '.'))
}

fn push(out: &mut String, text: &str) -> Result<(), CanvasSvgLimit> {
    push_bounded(out, text, MAX_ELEMENT_MARKUP)
}

fn push_escaped(out: &mut String, text: &str) -> Result<(), CanvasSvgLimit> {
    push_escaped_bounded(out, text, MAX_ELEMENT_MARKUP)
}
