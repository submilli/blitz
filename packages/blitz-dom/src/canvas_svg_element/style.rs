//! Computed styles written as presentation attributes, for a parser that
//! sees no stylesheet. Attributes, unlike a `style` attribute, are parsed one
//! value each, so no value (a font family name, say) can add declarations.
//!
//! Only properties Blink paints with enter: those it applies through paint
//! property nodes (opacity, filters, clips, masks, blending) are left out, as
//! `PaintResourceSubtree` leaves them out.

use super::MAX_ATTRIBUTE_LEN;
use super::plan::Slot;
use crate::canvas_svg_filter::{CanvasSvgLimit, svg_color};
use style::properties::{ComputedValues, LonghandId, LonghandIdSet, PropertyDeclarationId};
use style::shared_lock::SharedRwLockReadGuard;
use style::values::computed::Color;
use style::values::computed::url::ComputedUrl;
use style::values::generics::svg::{SVGPaint, SVGPaintFallback, SVGPaintKind};
use style::values::generics::url::UrlOrNone;
use style_traits::ToCss;

/// Declarations read while finding which properties an element declares.
const MAX_DECLARATION_VISITS: usize = 64 * 1024;
/// Blink's largest font size, in pixels.
const MAX_FONT_SIZE: f32 = 10_000.0;

/// Which computed values an element writes.
#[derive(Clone, Copy, Debug)]
pub(super) enum Values<'s> {
    /// Every property: the root of a subtree styled where it stands.
    All,
    /// Those that differ from its parent's, which the parser inherits.
    BelowParent(&'s ComputedValues),
    /// Only those its own rules declare: it inherits the rest from the
    /// `<use>` instancing it (Blink matches rules against the original
    /// element, and inherits through the instance tree).
    Declared,
}

/// The properties written, named as their presentation attributes are. All
/// are inherited but `display` (written only as `none`) and the stop
/// properties (written only on `<stop>`).
pub(super) const WRITTEN: [(&str, LonghandId); 25] = [
    ("color", LonghandId::Color),
    ("fill", LonghandId::Fill),
    ("fill-opacity", LonghandId::FillOpacity),
    ("fill-rule", LonghandId::FillRule),
    ("stroke", LonghandId::Stroke),
    ("stroke-width", LonghandId::StrokeWidth),
    ("stroke-opacity", LonghandId::StrokeOpacity),
    ("stroke-dasharray", LonghandId::StrokeDasharray),
    ("stroke-dashoffset", LonghandId::StrokeDashoffset),
    ("stroke-linecap", LonghandId::StrokeLinecap),
    ("stroke-linejoin", LonghandId::StrokeLinejoin),
    ("stroke-miterlimit", LonghandId::StrokeMiterlimit),
    ("paint-order", LonghandId::PaintOrder),
    ("marker-start", LonghandId::MarkerStart),
    ("marker-mid", LonghandId::MarkerMid),
    ("marker-end", LonghandId::MarkerEnd),
    ("display", LonghandId::Display),
    ("visibility", LonghandId::Visibility),
    ("font-family", LonghandId::FontFamily),
    ("font-size", LonghandId::FontSize),
    ("font-style", LonghandId::FontStyle),
    ("font-weight", LonghandId::FontWeight),
    ("text-anchor", LonghandId::TextAnchor),
    ("stop-color", LonghandId::StopColor),
    ("stop-opacity", LonghandId::StopOpacity),
];

/// The marker properties.
const MARKERS: [LonghandId; 3] = [
    LonghandId::MarkerStart,
    LonghandId::MarkerMid,
    LonghandId::MarkerEnd,
];

/// Shapes the parser draws markers on but Blink does not.
pub(super) fn unmarked_shape(tag: &str) -> bool {
    matches!(tag, "rect" | "circle" | "ellipse")
}

/// Whether an attribute is a presentation attribute written from style.
pub(super) fn from_style(name: &str) -> bool {
    WRITTEN.iter().any(|(property, _)| *property == name)
}

/// Writes elements' styles, charging the declarations read.
pub(super) struct StyleWriter<'a> {
    guard: SharedRwLockReadGuard<'a>,
    visits: usize,
}

impl<'a> StyleWriter<'a> {
    pub(super) fn new(guard: SharedRwLockReadGuard<'a>) -> Self {
        Self { guard, visits: 0 }
    }

    /// The presentation attributes to write for `tag`'s `style`, by name.
    /// `copy` names the copy a `url()` at a slot is written as, if any.
    /// Values longer than an attribute the snapshot reads are left out.
    pub(super) fn write(
        &mut self,
        style: &ComputedValues,
        tag: &str,
        values: Values<'_>,
        copy: impl Fn(Slot) -> Option<String>,
    ) -> Result<Vec<(&'static str, String)>, CanvasSvgLimit> {
        let declared = match values {
            Values::Declared => Some(self.declared(style)?),
            Values::All | Values::BelowParent(_) => None,
        };
        let element = Element {
            style,
            tag,
            color: style.clone_color(),
            keep_current_color: matches!(values, Values::Declared),
        };
        let mut out = Vec::new();
        for (name, id) in WRITTEN {
            // The parser would draw inherited markers on shapes Blink never
            // marks.
            if unmarked_shape(tag) && MARKERS.contains(&id) {
                out.push((name, "none".to_owned()));
                continue;
            }
            if declared
                .as_ref()
                .is_some_and(|declared| !declared.contains(id))
            {
                continue;
            }
            if let Values::BelowParent(parent) = values
                && inherited_unchanged(id, style, parent)
            {
                continue;
            }
            let Some(value) = element.value(id, &copy) else {
                continue;
            };
            if value.len() <= MAX_ATTRIBUTE_LEN {
                out.push((name, value));
            }
        }
        Ok(out)
    }

    /// The longhands `style`'s rules declare, presentation attributes and the
    /// `style` attribute included. Blocks of another document's lock (left on
    /// a node adopted since it was styled) are skipped.
    fn declared(&mut self, style: &ComputedValues) -> Result<LonghandIdSet, CanvasSvgLimit> {
        let mut declared = LonghandIdSet::new();
        let Some(rules) = style.rules.as_ref() else {
            return Ok(declared);
        };
        for node in rules.self_and_ancestors() {
            charge(&mut self.visits, 1)?;
            let Some(source) = node.style_source() else {
                continue;
            };
            let Some(block) = source.get().try_read_with(&self.guard) else {
                continue;
            };
            let declarations = block.declarations();
            charge(&mut self.visits, declarations.len())?;
            for declaration in declarations {
                if let PropertyDeclarationId::Longhand(id) = declaration.id() {
                    declared.insert(id);
                }
            }
        }
        Ok(declared)
    }
}

/// Count `visits` more declarations read, within the bound.
fn charge(total: &mut usize, visits: usize) -> Result<(), CanvasSvgLimit> {
    *total = total.saturating_add(visits);
    if *total > MAX_DECLARATION_VISITS {
        return Err(CanvasSvgLimit);
    }
    Ok(())
}

/// What one element's values are written from.
struct Element<'s> {
    style: &'s ComputedValues,
    tag: &'s str,
    color: style::color::AbsoluteColor,
    keep_current_color: bool,
}

impl Element<'_> {
    /// Property `id`'s value as SVG syntax, or `None` when it is not written.
    fn value(&self, id: LonghandId, copy: &impl Fn(Slot) -> Option<String>) -> Option<String> {
        let (style, color) = (self.style, &self.color);
        let svg = style.get_inherited_svg();
        let current = self.keep_current_color;
        let value = match id {
            LonghandId::Color => svg_color(color),
            LonghandId::Fill => paint(&svg.fill, color, current, copy(Slot::Fill)),
            LonghandId::Stroke => paint(&svg.stroke, color, current, copy(Slot::Stroke)),
            LonghandId::FillOpacity => svg.fill_opacity.to_css_string(),
            LonghandId::FillRule => svg.fill_rule.to_css_string(),
            LonghandId::StrokeWidth => svg.stroke_width.to_css_string(),
            LonghandId::StrokeOpacity => svg.stroke_opacity.to_css_string(),
            LonghandId::StrokeDasharray => svg.stroke_dasharray.to_css_string(),
            LonghandId::StrokeDashoffset => svg.stroke_dashoffset.to_css_string(),
            LonghandId::StrokeLinecap => svg.stroke_linecap.to_css_string(),
            LonghandId::StrokeLinejoin => svg.stroke_linejoin.to_css_string(),
            LonghandId::StrokeMiterlimit => svg.stroke_miterlimit.to_css_string(),
            LonghandId::PaintOrder => svg.paint_order.to_css_string(),
            LonghandId::MarkerStart => marker(&svg.marker_start, copy(Slot::MarkerStart)),
            LonghandId::MarkerMid => marker(&svg.marker_mid, copy(Slot::MarkerMid)),
            LonghandId::MarkerEnd => marker(&svg.marker_end, copy(Slot::MarkerEnd)),
            LonghandId::TextAnchor => svg.text_anchor.to_css_string(),
            // Only `none` matters to painting; other values lay out boxes.
            LonghandId::Display => style.clone_display().is_none().then(|| "none".to_owned())?,
            LonghandId::Visibility => style.clone_visibility().to_css_string(),
            LonghandId::FontFamily => style.clone_font_family().to_css_string(),
            LonghandId::FontSize => {
                let size = style.clone_font_size().computed_size().px();
                format!("{}px", size.clamp(0.0, MAX_FONT_SIZE))
            }
            LonghandId::FontStyle => style.clone_font_style().to_css_string(),
            LonghandId::FontWeight => style.clone_font_weight().value().to_string(),
            LonghandId::StopColor if self.tag == "stop" => {
                let stop = style.clone_stop_color();
                if current && stop.is_currentcolor() {
                    "currentColor".to_owned()
                } else {
                    svg_color(&stop.resolve_to_absolute(color))
                }
            }
            LonghandId::StopOpacity if self.tag == "stop" => style.clone_stop_opacity().to_string(),
            _ => return None,
        };
        Some(value)
    }
}

/// Whether the parser would inherit `id`'s value from `parent` unchanged.
/// Values are compared in place, as some (dash arrays) can be long. Paints
/// that use currentColor are written resolved, so they change with `color`.
fn inherited_unchanged(id: LonghandId, style: &ComputedValues, parent: &ComputedValues) -> bool {
    let (svg, above) = (style.get_inherited_svg(), parent.get_inherited_svg());
    match id {
        LonghandId::Color => style.clone_color() == parent.clone_color(),
        LonghandId::Fill => absolute_paint(&svg.fill) && svg.fill == above.fill,
        LonghandId::Stroke => absolute_paint(&svg.stroke) && svg.stroke == above.stroke,
        LonghandId::FillOpacity => svg.fill_opacity == above.fill_opacity,
        LonghandId::FillRule => svg.fill_rule == above.fill_rule,
        LonghandId::StrokeWidth => svg.stroke_width == above.stroke_width,
        LonghandId::StrokeOpacity => svg.stroke_opacity == above.stroke_opacity,
        LonghandId::StrokeDasharray => svg.stroke_dasharray == above.stroke_dasharray,
        LonghandId::StrokeDashoffset => svg.stroke_dashoffset == above.stroke_dashoffset,
        LonghandId::StrokeLinecap => svg.stroke_linecap == above.stroke_linecap,
        LonghandId::StrokeLinejoin => svg.stroke_linejoin == above.stroke_linejoin,
        LonghandId::StrokeMiterlimit => svg.stroke_miterlimit == above.stroke_miterlimit,
        LonghandId::PaintOrder => svg.paint_order == above.paint_order,
        LonghandId::MarkerStart => svg.marker_start == above.marker_start,
        LonghandId::MarkerMid => svg.marker_mid == above.marker_mid,
        LonghandId::MarkerEnd => svg.marker_end == above.marker_end,
        LonghandId::TextAnchor => svg.text_anchor == above.text_anchor,
        LonghandId::Visibility => style.clone_visibility() == parent.clone_visibility(),
        LonghandId::FontFamily => style.clone_font_family() == parent.clone_font_family(),
        LonghandId::FontSize => style.clone_font_size() == parent.clone_font_size(),
        LonghandId::FontStyle => style.clone_font_style() == parent.clone_font_style(),
        LonghandId::FontWeight => style.clone_font_weight() == parent.clone_font_weight(),
        _ => false,
    }
}

/// Whether a paint's colors do not depend on currentColor.
fn absolute_paint(paint: &SVGPaint<Color, ComputedUrl>) -> bool {
    let kind = match &paint.kind {
        SVGPaintKind::Color(color) => color.is_absolute(),
        _ => true,
    };
    let fallback = match &paint.fallback {
        SVGPaintFallback::Color(color) => color.is_absolute(),
        _ => true,
    };
    kind && fallback
}

/// A computed paint as SVG paint syntax. A paint server without a copy draws
/// its fallback, or nothing.
fn paint(
    paint: &SVGPaint<Color, ComputedUrl>,
    color: &style::color::AbsoluteColor,
    keep_current_color: bool,
    server: Option<String>,
) -> String {
    let color_value = |value: &Color| {
        if keep_current_color && value.is_currentcolor() {
            "currentColor".to_owned()
        } else {
            svg_color(&value.resolve_to_absolute(color))
        }
    };
    let fallback = match &paint.fallback {
        SVGPaintFallback::None => Some("none".to_owned()),
        SVGPaintFallback::Color(value) => Some(color_value(value)),
        SVGPaintFallback::Unset => None,
    };
    match &paint.kind {
        SVGPaintKind::None => "none".to_owned(),
        SVGPaintKind::Color(value) => color_value(value),
        SVGPaintKind::PaintServer(_) => match (server, fallback) {
            (Some(server), Some(fallback)) => format!("url(#{server}) {fallback}"),
            (Some(server), None) => format!("url(#{server})"),
            (None, fallback) => fallback.unwrap_or_else(|| "none".to_owned()),
        },
        SVGPaintKind::ContextFill => "context-fill".to_owned(),
        SVGPaintKind::ContextStroke => "context-stroke".to_owned(),
    }
}

/// A computed marker as a `url()` naming its copy, or `none` without one.
fn marker(value: &UrlOrNone<ComputedUrl>, copy: Option<String>) -> String {
    match (value, copy) {
        (UrlOrNone::Url(_), Some(copy)) => format!("url(#{copy})"),
        _ => "none".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every written property has a value and a comparison: a property added
    /// to `WRITTEN` without either would silently drop.
    #[test]
    fn every_written_property_is_handled() {
        let style = ComputedValues::initial_values_with_font_override(
            style::properties::style_structs::Font::initial_values(),
        );
        let element = Element {
            style: &style,
            tag: "stop",
            color: style.clone_color(),
            keep_current_color: false,
        };
        for (name, id) in WRITTEN {
            let written = element.value(id, &|_| None);
            assert!(written.is_some() || id == LonghandId::Display, "{name}");
            let compared = inherited_unchanged(id, &style, &style);
            let not_inherited = matches!(
                id,
                LonghandId::Display | LonghandId::StopColor | LonghandId::StopOpacity
            );
            assert!(compared != not_inherited, "{name}");
        }
    }

    /// Each presentation attribute written from style is mapped into style,
    /// or the attribute would be lost.
    #[test]
    fn written_properties_are_presentation_attributes() {
        for (name, _) in WRITTEN {
            assert!(
                crate::svg_presentation::PRESENTATION_PROPERTIES.contains(&name),
                "{name}"
            );
        }
    }
}
