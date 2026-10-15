//! `feImage` element references, captured as Chrome paints them (Blink's
//! `FEImage::CreateImageFilterForLayoutObject`).
//!
//! Blink records the referenced subtree with `PaintResourceSubtree`, which
//! draws content but none of the paint property nodes below it: transforms
//! (`<use>` offsets and nested viewports included), opacity, filters, clips
//! and masks are all left out. It culls that picture to the element's visual
//! rectangle *with* the transforms. [`plan`] decides what to serialize and
//! [`markup`] writes it twice: the painted markup, and a placed one with the
//! transforms that only measures the bounds.
//!
//! Each element carries its computed paint styles, since the parser sees no
//! stylesheet. Every element a kept reference names (a `<use>` target, paint
//! server, marker or text path) becomes a numbered copy in `<defs>`, as
//! elements keep no IDs of their own; a `<use>` copy keeps only its declared
//! styles so that it inherits from the `<use>`, as Blink's instance trees do.
//! Patterns paint their content through `PaintResourceSubtree` too, so every
//! copy leaves transforms and effects out alike.

mod bounds;
mod elements;
mod lookup;
mod markup;
mod numbers;
mod plan;
mod scc;
mod style;
mod viewport;

use crate::canvas_svg_filter::CanvasSvgLimit;
use crate::canvas_svg_image::CanvasSvgElement;
use crate::{BaseDocument, NodeId};

/// Elements of one reference: the subtree and its copies.
const MAX_ELEMENT_NODES: usize = 1024;
/// What the parser may build once every copy is built as many times as the
/// plan counts (per `<use>`, painted element and marked vertex), so that a few
/// nested references cannot multiply into a huge tree: each element counts
/// once, plus once per [`WEIGHT_BYTES`] of its attribute and text content.
const MAX_EXPANDED_WEIGHT: usize = 8 * 1024;
const WEIGHT_BYTES: usize = 256;
/// Markup of one reference.
const MAX_ELEMENT_MARKUP: usize = 256 * 1024;
/// Painted markup the parser may copy once every source is built as many
/// times as the plan counts.
const MAX_EXPANDED_BYTES: usize = 4 * 1024 * 1024;
/// Attribute values read as references or lengths; longer ones are ignored,
/// as the filter snapshot ignores them.
const MAX_ATTRIBUTE_LEN: usize = 4096;

impl BaseDocument {
    /// The SVG element `target` as an `feImage` draws it, or `None` when it
    /// draws nothing: past the bounds, or when it has no rendered content.
    pub(crate) fn capture_svg_element(
        &self,
        target: NodeId,
    ) -> Result<Option<CanvasSvgElement>, CanvasSvgLimit> {
        let tag = self.nodes[target]
            .element_data()
            .map(|element| element.name.local.as_ref());
        if tag.is_none_or(|tag| elements::HIDDEN_CONTAINERS.contains(&tag)) {
            return Ok(None);
        }
        let plan = plan::Plan::new(self, target)?;
        let viewport = self.svg_viewport(target)?;
        // Percentages resolve against the viewport (see `bounded_numbers`).
        let huge = |size: [f32; 2]| {
            size.iter()
                .any(|side| f64::from(*side) > numbers::MAX_COORDINATE)
        };
        if viewport.is_some_and(huge) {
            return Err(CanvasSvgLimit);
        }
        let markup = markup::write(self, &plan, viewport)?;
        let expanded = plan
            .builds
            .iter()
            .zip(&markup.bytes)
            .fold(0usize, |total, (builds, bytes)| {
                total.saturating_add(builds.saturating_mul(*bytes))
            });
        if expanded > MAX_EXPANDED_BYTES {
            return Err(CanvasSvgLimit);
        }
        let Some(painted) = self.parse_element_markup(&markup.painted) else {
            return Ok(None);
        };
        let placed = match &markup.placed {
            Some(placed) => self.parse_placed_markup(placed),
            None => Some(painted.tree.clone()),
        };
        let bounds =
            placed.and_then(|placed| bounds::measure(&placed).and_then(|rect| rect.to_non_zero_rect()));
        Ok(bounds.map(|bounds| CanvasSvgElement {
            tree: painted.tree,
            nested_rasters: painted.nested_rasters,
            bounds,
            // Blink maps only a viewport with area (an empty one scales by 1).
            viewport: viewport
                .filter(|size| plan.viewport_dependent && size.iter().all(|side| *side > 0.0)),
        }))
    }

    /// Parse serialized markup within the decoded image budget, which charges
    /// the tree and its nested rasters while a snapshot holds them.
    fn parse_element_markup(&self, markup: &str) -> Option<crate::node::SvgImageData> {
        crate::svg_bounds::parse_svg_image(markup.as_bytes(), &self.svg_fonts, &self.image_budget)
            .ok()
    }

    /// Parse the placed markup, which is only measured, so its nested rasters
    /// are not decoded.
    fn parse_placed_markup(&self, markup: &str) -> Option<std::sync::Arc<usvg::Tree>> {
        crate::svg_bounds::parse_svg_geometry(
            markup.as_bytes(),
            &self.svg_fonts,
            &self.image_budget,
        )
        .ok()
        .map(|svg| svg.tree)
    }
}

#[cfg(test)]
mod tests;
