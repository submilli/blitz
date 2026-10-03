//! Computed canvas filters. Parsing stays in Stylo; rendering owns execution.

use color::{AlphaColor, Srgb};
use style::color::AbsoluteColor;
use style::computed_values::filter::single_value::T as Filter;
use style_traits::ToCss;

use crate::{BaseDocument, CanvasFont, NodeId, util::ToColorColor};

/// One computed operation, in output-bitmap coordinates.
#[derive(Clone, Debug)]
pub enum CanvasFilter {
    Blur(f32),
    Brightness(f32),
    Contrast(f32),
    Grayscale(f32),
    HueRotate(f32),
    Invert(f32),
    Opacity(f32),
    Saturate(f32),
    Sepia(f32),
    DropShadow {
        dx: f32,
        dy: f32,
        blur: f32,
        color: AlphaColor<Srgb>,
        /// Retained for draw-time currentColor/origin-clean handling.
        unresolved_color: Option<String>,
    },
    /// A reference is resolved at draw time, never fetched by this parser.
    Reference(String),
}

/// An immutable, bounded list retaining the exact Canvas getter string.
#[derive(Clone, Debug)]
pub struct CanvasFilters {
    pub serialized: String,
    pub operations: Vec<CanvasFilter>,
}

impl Default for CanvasFilters {
    fn default() -> Self {
        Self {
            serialized: "none".into(),
            operations: Vec::new(),
        }
    }
}

impl BaseDocument {
    /// Resolve style-dependent filter colors immediately before drawing.
    /// Call after flushing styles; retained lengths and getter text stay unchanged.
    pub fn resolve_canvas_filter_colors(&self, node: NodeId, filters: &mut CanvasFilters) {
        for operation in &mut filters.operations {
            if let CanvasFilter::DropShadow {
                color,
                unresolved_color: Some(expression),
                ..
            } = operation
            {
                *color = self
                    .canvas_computed_style(node, "color", expression)
                    .map(|style| style.clone_color().as_color_color())
                    .unwrap_or_else(|| AbsoluteColor::BLACK.as_color_color());
            }
        }
    }

    /// Compute relative lengths at assignment time after styles have been flushed.
    /// Invalid/unresolved declarations and excessive lists leave state unchanged.
    pub fn canvas_filters(
        &self,
        node: NodeId,
        text: &str,
        font: &CanvasFont,
    ) -> Option<CanvasFilters> {
        // Chrome snapshots the drawing context font, including for hidden and
        // detached canvases. The already-computed shorthand fixes relative units.
        let parent = self.canvas_computed_style(node, "font", &font.serialized)?;
        let declarations = self.canvas_style_declarations("filter", text)?;
        let style = self.compute_canvas_style(&declarations, &parent);
        let filters = &style.get_effects().filter.0;
        if filters.len() > 32 {
            return None;
        }
        let operations = filters.iter().map(convert).collect::<Option<Vec<_>>>()?;
        Some(CanvasFilters {
            serialized: text.into(),
            operations,
        })
    }
}

fn convert(filter: &Filter) -> Option<CanvasFilter> {
    // Computed CSS can contain very large or nonfinite calc results. Admission
    // here bounds retained parameters; the renderer must also bound total work.
    let bounded = |value: f32| (value.is_finite() && value.abs() <= 1e6).then_some(value);
    Some(match filter {
        Filter::Blur(value) => CanvasFilter::Blur(bounded(value.px())?),
        Filter::Brightness(value) => CanvasFilter::Brightness(bounded(value.0)?),
        Filter::Contrast(value) => CanvasFilter::Contrast(bounded(value.0)?),
        Filter::Grayscale(value) => CanvasFilter::Grayscale(bounded(value.0)?),
        Filter::HueRotate(value) => CanvasFilter::HueRotate(bounded(value.radians())?),
        Filter::Invert(value) => CanvasFilter::Invert(bounded(value.0)?),
        Filter::Opacity(value) => CanvasFilter::Opacity(bounded(value.0)?),
        Filter::Saturate(value) => CanvasFilter::Saturate(bounded(value.0)?),
        Filter::Sepia(value) => CanvasFilter::Sepia(bounded(value.0)?),
        Filter::DropShadow(value) => CanvasFilter::DropShadow {
            unresolved_color: (!value.color.is_absolute()).then(|| value.color.to_css_string()),
            dx: bounded(value.horizontal.px())?,
            dy: bounded(value.vertical.px())?,
            blur: bounded(value.blur.px())?,
            color: value
                .color
                .resolve_to_absolute(&AbsoluteColor::BLACK)
                .as_color_color(),
        },
        Filter::Url(value) => CanvasFilter::Reference(value.to_css_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DocumentConfig, qual_name};

    #[test]
    fn filter_snapshots_compute_lengths_and_preserve_input() {
        let mut document = BaseDocument::new(DocumentConfig::default());
        let root = document.root_node().id;
        let mut mutation = document.mutate();
        let html = mutation.create_element(qual_name!("html", html), vec![]);
        mutation.append_children(root, &[html]);
        let canvas = mutation.create_element(qual_name!("canvas", html), vec![]);
        mutation.append_children(html, &[canvas]);
        mutation.set_attribute(canvas, qual_name!("style"), "font-size:20px");
        drop(mutation);
        document.flush_style_and_layout(0.0);
        let text = " blur(1em) opacity(50%) ";
        let font = document.canvas_font(canvas, "40px serif").unwrap();
        let snapshot = document.canvas_filters(canvas, text, &font).unwrap();
        assert_eq!(snapshot.serialized, text);
        assert!(matches!(
            snapshot.operations.as_slice(),
            [CanvasFilter::Blur(40.0), CanvasFilter::Opacity(0.5)]
        ));
        document.mutate().remove_node(canvas);
        let detached = document.canvas_filters(canvas, "blur(1em)", &font).unwrap();
        assert!(matches!(
            detached.operations.as_slice(),
            [CanvasFilter::Blur(40.0)]
        ));
        assert!(matches!(snapshot.operations[0], CanvasFilter::Blur(40.0)));
    }

    #[test]
    fn filter_current_color_resolves_at_draw_time_without_recomputing_lengths() {
        let mut document = BaseDocument::new(DocumentConfig::default());
        let root = document.root_node().id;
        let mut mutation = document.mutate();
        let html = mutation.create_element(qual_name!("html", html), vec![]);
        mutation.append_children(root, &[html]);
        let canvas = mutation.create_element(qual_name!("canvas", html), vec![]);
        mutation.append_children(html, &[canvas]);
        mutation.set_attribute(canvas, qual_name!("style"), "color:red");
        drop(mutation);
        document.flush_style_and_layout(0.0);
        let font = document.canvas_font(canvas, "20px serif").unwrap();
        let mut filters = document
            .canvas_filters(canvas, "drop-shadow(1em 0 currentColor)", &font)
            .unwrap();
        document
            .mutate()
            .set_attribute(canvas, qual_name!("style"), "color:blue;font-size:40px");
        document.flush_style_and_layout(0.0);
        document.resolve_canvas_filter_colors(canvas, &mut filters);
        let CanvasFilter::DropShadow {
            dx,
            color,
            unresolved_color,
            ..
        } = &filters.operations[0]
        else {
            panic!("drop shadow")
        };
        assert_eq!(*dx, 20.0);
        assert_eq!(color.components, [0.0, 0.0, 1.0, 1.0]);
        assert!(unresolved_color.is_some());
    }

    #[test]
    fn filters_reject_invalid_unresolved_and_excessive_input() {
        let document = BaseDocument::new(DocumentConfig::default());
        let node = document.root_node().id;
        let font = CanvasFont::default();
        for text in [
            "",
            "inherit",
            "initial",
            "var(--filter)",
            "blur(-1px)",
            "opacity(-1)",
            "blur(1px);color:red",
            "blur(1e9px)",
        ] {
            assert!(
                document.canvas_filters(node, text, &font).is_none(),
                "{text}"
            );
        }
        assert!(
            document
                .canvas_filters(node, &"opacity(1) ".repeat(33), &font)
                .is_none()
        );
        assert!(
            document
                .canvas_filters(node, "none", &font)
                .unwrap()
                .operations
                .is_empty()
        );
        assert_eq!(document.canvas_filters(node, "brightness(2) contrast(1.2) grayscale(.5) hue-rotate(90deg) invert(.2) saturate(2) sepia(.3) drop-shadow(1px 2px 3px red)", &font).unwrap().operations.len(), 8);
    }
}
