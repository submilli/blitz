//! The viewport a referenced element's lengths resolve against, and whether
//! its geometry depends on it.

use super::MAX_ATTRIBUTE_LEN;
use crate::canvas_svg_filter::CanvasSvgLimit;
use crate::node::ElementData;
use crate::{BaseDocument, NodeId};
use std::str::FromStr;
use style::properties::ComputedValues;
use svgtypes::LengthUnit;

/// Ancestors inspected for the nearest viewport.
const MAX_ANCESTORS: usize = 4096;

impl BaseDocument {
    /// The size of the viewport `target` resolves lengths against (Blink's
    /// `SVGViewportResolver::ResolveViewport`): an outermost `<svg>`'s own
    /// CSS content box; else the nearest `<svg>` ancestor's `viewBox` size, or
    /// its viewport, the outermost `<svg>`'s being its content box. A side
    /// may be zero; `None` outside any `<svg>`.
    pub(super) fn svg_viewport(&self, target: NodeId) -> Result<Option<[f32; 2]>, CanvasSvgLimit> {
        if self.outermost_svg(target) {
            let content = self.nodes[target].unrounded_layout().content_box_size();
            return Ok(finite([content.width, content.height]));
        }
        // The `<svg>` ancestors up to the first that fixes its viewport.
        let mut chain = Vec::new();
        let mut size = None;
        let mut current = self.nodes[target].parent;
        let mut ancestors = 0;
        while let Some(id) = current {
            ancestors += 1;
            if ancestors > MAX_ANCESTORS {
                return Err(CanvasSvgLimit);
            }
            let node = &self.nodes[id];
            let Some(element) = node.element_data() else {
                break;
            };
            if element.name.ns != markup5ever::ns!(svg) {
                break;
            }
            current = node.parent;
            if element.name.local.as_ref() != "svg" {
                continue;
            }
            if let Some(view_box) = view_box_size(element) {
                size = Some(view_box);
                break;
            }
            if self.outermost_svg(id) {
                let content = node.unrounded_layout().content_box_size();
                size = Some([content.width, content.height]);
                break;
            }
            chain.push(id);
        }
        let Some(mut size) = size else {
            return Ok(None);
        };
        // Inner viewports without a `viewBox`, outermost first, size against
        // the viewport around them.
        for &id in chain.iter().rev() {
            let node = &self.nodes[id];
            let element = node.element_data().expect("the chain holds elements");
            let font_size = node
                .primary_styles()
                .map_or(16.0, |style| style.clone_font_size().computed_size().px());
            let length = |name: &str, around: f32| {
                element
                    .attr(markup5ever::LocalName::from(name))
                    .and_then(|value| resolve_length(value, around, font_size))
                    .unwrap_or(around)
            };
            size = [length("width", size[0]), length("height", size[1])];
        }
        Ok(finite(size))
    }

    /// Whether `node` is an `<svg>` element whose parent is not SVG content.
    fn outermost_svg(&self, node: NodeId) -> bool {
        let node = &self.nodes[node];
        let svg = node.element_data().is_some_and(|element| {
            element.name.ns == markup5ever::ns!(svg) && element.name.local.as_ref() == "svg"
        });
        svg && node
            .parent
            .and_then(|parent| self.nodes[parent].element_data())
            .is_none_or(|parent| parent.name.ns != markup5ever::ns!(svg))
    }
}

/// A size whose sides are finite and not negative.
fn finite(size: [f32; 2]) -> Option<[f32; 2]> {
    size.iter()
        .all(|side| side.is_finite() && *side >= 0.0)
        .then_some(size)
}

/// A valid `viewBox`'s size.
pub(super) fn view_box_size(element: &ElementData) -> Option<[f32; 2]> {
    let value = element.attr(markup5ever::local_name!("viewBox"))?;
    if value.len() > MAX_ATTRIBUTE_LEN {
        return None;
    }
    let view_box = svgtypes::ViewBox::from_str(value).ok()?;
    let size = [view_box.w as f32, view_box.h as f32];
    size.iter()
        .all(|side| side.is_finite() && *side > 0.0)
        .then_some(size)
}

/// An SVG length in user units, percentages of `around`. Negative and
/// invalid lengths are `None`, which keeps the default.
fn resolve_length(value: &str, around: f32, font_size: f32) -> Option<f32> {
    if value.len() > MAX_ATTRIBUTE_LEN {
        return None;
    }
    let length = svgtypes::Length::from_str(value.trim()).ok()?;
    let number = length.number as f32;
    let resolved = match length.unit {
        LengthUnit::None | LengthUnit::Px => number,
        LengthUnit::Percent => number / 100.0 * around,
        LengthUnit::Em => number * font_size,
        LengthUnit::Ex => number * font_size / 2.0,
        LengthUnit::In => number * 96.0,
        LengthUnit::Cm => number * 96.0 / 2.54,
        LengthUnit::Mm => number * 96.0 / 25.4,
        LengthUnit::Pt => number * 4.0 / 3.0,
        LengthUnit::Pc => number * 16.0,
    };
    (resolved.is_finite() && resolved >= 0.0).then_some(resolved)
}

/// Whether the element itself depends on its viewport, as Blink's layout
/// decides it: percentages in shape and image geometry
/// (`CalculateGeometryDependsOnViewport`) and stroke lengths
/// (`ComputeStrokeHasRelativeLengths`), any relative unit in a `<line>`'s
/// points and in `<svg>` and `<use>` positions and sizes
/// (`SelfHasRelativeLengths`), and all text.
pub(super) fn self_dependent(
    tag: &str,
    element: &ElementData,
    style: Option<&ComputedValues>,
) -> bool {
    let attr = |name: &str| {
        element
            .attr(markup5ever::LocalName::from(name))
            .filter(|value| value.len() <= MAX_ATTRIBUTE_LEN)
    };
    let percent = |names: &[&str]| {
        names
            .iter()
            .any(|name| attr(name).is_some_and(|value| value.contains('%')))
    };
    let relative = |names: &[&str]| {
        names
            .iter()
            .any(|name| attr(name).is_some_and(relative_length))
    };
    let stroke = style.is_some_and(stroke_dependent);
    match tag {
        "rect" => percent(&["x", "y", "width", "height", "rx", "ry"]) || stroke,
        "circle" => percent(&["cx", "cy", "r"]) || stroke,
        "ellipse" => percent(&["cx", "cy", "rx", "ry"]) || stroke,
        "line" => relative(&["x1", "y1", "x2", "y2"]) || stroke,
        "path" | "polyline" | "polygon" => stroke,
        "image" => percent(&["x", "y", "width", "height"]),
        "text" => true,
        "svg" | "use" => relative(&["x", "y", "width", "height"]),
        _ => false,
    }
}

/// Stroke lengths given in percentages, read without serializing them.
fn stroke_dependent(style: &ComputedValues) -> bool {
    use style::values::generics::svg::{SVGLength, SVGStrokeDashArray};
    let svg = style.get_inherited_svg();
    let width = match &svg.stroke_width {
        SVGLength::LengthPercentage(width) => width.0.has_percentage(),
        SVGLength::ContextValue => false,
    };
    let offset = match &svg.stroke_dashoffset {
        SVGLength::LengthPercentage(offset) => offset.has_percentage(),
        SVGLength::ContextValue => false,
    };
    let dashes = match &svg.stroke_dasharray {
        SVGStrokeDashArray::Values(dashes) => dashes.iter().any(|dash| dash.0.has_percentage()),
        SVGStrokeDashArray::ContextValue => false,
    };
    width || offset || dashes
}

/// Blink's `SVGLength::IsRelative`: a percentage or a font-relative unit.
fn relative_length(value: &str) -> bool {
    svgtypes::Length::from_str(value.trim()).is_ok_and(|length| {
        matches!(
            length.unit,
            LengthUnit::Percent | LengthUnit::Em | LengthUnit::Ex
        )
    })
}
