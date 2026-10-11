//! `feImage` element references, captured as Chrome paints them (Blink's
//! `FEImage::CreateImageFilterForLayoutObject`).
//!
//! Blink records the referenced subtree with `PaintResourceSubtree`, which
//! draws content but none of the paint property nodes below it: transforms
//! (`<use>` offsets and nested viewports included), opacity, filters, clips
//! and masks are all left out. It culls that picture to the element's visual
//! rectangle *with* the transforms. One [`plan`] therefore writes two markups:
//! the painted one, and a placed one with the transforms that only measures
//! the bounds.
//!
//! Each element carries its computed paint styles, since the parser sees no
//! stylesheet. `<use>` targets and paint servers outside the subtree become
//! numbered copies in `<defs>`; a `<use>` copy keeps only its declared styles
//! so that it inherits from the `<use>`, as Blink's instance trees do.

mod markup;
mod plan;
mod style;
mod viewport;

use crate::canvas_svg_filter::CanvasSvgLimit;
use crate::canvas_svg_image::CanvasSvgElement;
use crate::{BaseDocument, NodeId};

/// Elements of one reference: the subtree and its copies.
const MAX_ELEMENT_NODES: usize = 1024;
/// Elements the parser may build once `<use>` instances expand, so a few
/// nested `<use>`s cannot multiply into a huge tree.
const MAX_EXPANDED_NODES: usize = 8 * 1024;
/// Markup of one reference.
const MAX_ELEMENT_MARKUP: usize = 256 * 1024;

/// Containers Blink lays out without painting (`LayoutSVGHiddenContainer`):
/// a reference to one draws nothing, and lengths inside never make a
/// reference viewport-dependent.
const HIDDEN_CONTAINERS: [&str; 9] = [
    "defs",
    "linearGradient",
    "radialGradient",
    "pattern",
    "clipPath",
    "mask",
    "marker",
    "filter",
    "symbol",
];

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
        if tag.is_none_or(|tag| HIDDEN_CONTAINERS.contains(&tag)) {
            return Ok(None);
        }
        let plan = plan::Plan::new(self, target)?;
        let viewport = self.svg_viewport(target)?;
        let markup = markup::write(self, &plan, viewport)?;
        let Some(tree) = self.parse_element_markup(&markup.painted) else {
            return Ok(None);
        };
        let placed = match &markup.placed {
            Some(placed) => self.parse_element_markup(placed),
            None => Some(tree.clone()),
        };
        let bounds =
            placed.and_then(|placed| placed.root().abs_stroke_bounding_box().to_non_zero_rect());
        Ok(bounds.map(|bounds| CanvasSvgElement {
            tree,
            bounds,
            viewport: viewport.filter(|_| plan.viewport_dependent),
        }))
    }

    /// Parse serialized markup within the decoded image budget, which charges
    /// the tree while a snapshot holds it.
    fn parse_element_markup(&self, markup: &str) -> Option<std::sync::Arc<usvg::Tree>> {
        crate::svg_bounds::parse_svg_image(markup.as_bytes(), &self.svg_fonts, &self.image_budget)
            .ok()
            .map(|svg| svg.tree)
    }
}

#[cfg(test)]
mod tests;
