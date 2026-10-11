//! Computed styles written as `style` attributes, for a parser that sees no
//! stylesheet.
//!
//! Only properties Blink paints with enter: those it applies through paint
//! property nodes (opacity, filters, clips, masks, blending) are left out, as
//! `PaintResourceSubtree` leaves them out.

use super::plan::Slot;
use crate::canvas_svg_filter::{CanvasSvgLimit, svg_color};
use style::properties::{ComputedValues, LonghandId, LonghandIdSet, PropertyDeclarationId};
use style::shared_lock::SharedRwLockReadGuard;
use style::values::computed::Color;
use style::values::computed::url::ComputedUrl;
use style::values::generics::svg::{SVGPaint, SVGPaintFallback, SVGPaintKind};
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
    /// Those that differ from its parent's, which the parser inherits (every
    /// property written is inherited, but for `display: none`).
    BelowParent(&'s ComputedValues),
    /// Only those its own rules declare: it inherits the rest from the
    /// `<use>` instancing it (Blink matches rules against the original
    /// element, and inherits through the instance tree).
    Declared,
}

/// The properties written, in order, as their presentation attributes are named.
const PROPERTIES: [(&str, LonghandId); 18] = [
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
    ("display", LonghandId::Display),
    ("visibility", LonghandId::Visibility),
    ("font-family", LonghandId::FontFamily),
    ("font-size", LonghandId::FontSize),
    ("font-style", LonghandId::FontStyle),
    ("font-weight", LonghandId::FontWeight),
];

/// Whether an attribute is a presentation attribute written from style.
pub(super) fn from_style(name: &str) -> bool {
    PROPERTIES.iter().any(|(property, _)| *property == name)
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

    /// The `style` attribute value for `style`. `paint_server` names the copy
    /// a `url()` paint is written as, if any.
    pub(super) fn write(
        &mut self,
        style: &ComputedValues,
        values: Values,
        paint_server: impl Fn(Slot) -> Option<String>,
    ) -> Result<String, CanvasSvgLimit> {
        let declared = match values {
            Values::Declared => Some(self.declared(style)?),
            Values::All | Values::BelowParent(_) => None,
        };
        let keep_current_color = matches!(values, Values::Declared);
        let color = style.clone_color();
        let mut out = String::new();
        for (name, id) in PROPERTIES {
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
            let value = match id {
                LonghandId::Color => svg_color(&color),
                LonghandId::Fill => paint(
                    &style.clone_fill(),
                    &color,
                    keep_current_color,
                    paint_server(Slot::Fill),
                ),
                LonghandId::Stroke => paint(
                    &style.clone_stroke(),
                    &color,
                    keep_current_color,
                    paint_server(Slot::Stroke),
                ),
                LonghandId::FillOpacity => style.clone_fill_opacity().to_css_string(),
                LonghandId::FillRule => style.clone_fill_rule().to_css_string(),
                LonghandId::StrokeWidth => style.clone_stroke_width().to_css_string(),
                LonghandId::StrokeOpacity => style.clone_stroke_opacity().to_css_string(),
                LonghandId::StrokeDasharray => style.clone_stroke_dasharray().to_css_string(),
                LonghandId::StrokeDashoffset => style.clone_stroke_dashoffset().to_css_string(),
                LonghandId::StrokeLinecap => style.clone_stroke_linecap().to_css_string(),
                LonghandId::StrokeLinejoin => style.clone_stroke_linejoin().to_css_string(),
                LonghandId::StrokeMiterlimit => style.clone_stroke_miterlimit().to_css_string(),
                // Only `none` matters to painting; other values lay out boxes.
                LonghandId::Display if style.clone_display().is_none() => "none".to_owned(),
                LonghandId::Display => continue,
                LonghandId::Visibility => style.clone_visibility().to_css_string(),
                LonghandId::FontFamily => style.clone_font_family().to_css_string(),
                LonghandId::FontSize => {
                    let size = style.clone_font_size().computed_size().px();
                    format!("{}px", size.clamp(0.0, MAX_FONT_SIZE))
                }
                LonghandId::FontStyle => style.clone_font_style().to_css_string(),
                LonghandId::FontWeight => style.clone_font_weight().value().to_string(),
                _ => continue,
            };
            out.push_str(name);
            out.push(':');
            out.push_str(&value);
            out.push(';');
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
            self.charge()?;
            let Some(source) = node.style_source() else {
                continue;
            };
            let Some(block) = source.get().try_read_with(&self.guard) else {
                continue;
            };
            let declarations = block.declarations();
            self.visits += declarations.len();
            if self.visits > MAX_DECLARATION_VISITS {
                return Err(CanvasSvgLimit);
            }
            for declaration in declarations {
                if let PropertyDeclarationId::Longhand(id) = declaration.id() {
                    declared.insert(id);
                }
            }
        }
        Ok(declared)
    }

    fn charge(&mut self) -> Result<(), CanvasSvgLimit> {
        self.visits += 1;
        if self.visits > MAX_DECLARATION_VISITS {
            return Err(CanvasSvgLimit);
        }
        Ok(())
    }
}

/// Whether the parser would inherit `id`'s value from `parent` unchanged.
/// Paints that use currentColor are written resolved, so they change with
/// `color`.
fn inherited_unchanged(id: LonghandId, style: &ComputedValues, parent: &ComputedValues) -> bool {
    match id {
        LonghandId::Color => style.clone_color() == parent.clone_color(),
        LonghandId::Fill => {
            let fill = style.clone_fill();
            absolute_paint(&fill) && fill == parent.clone_fill()
        }
        LonghandId::Stroke => {
            let stroke = style.clone_stroke();
            absolute_paint(&stroke) && stroke == parent.clone_stroke()
        }
        LonghandId::FillOpacity => style.clone_fill_opacity() == parent.clone_fill_opacity(),
        LonghandId::FillRule => style.clone_fill_rule() == parent.clone_fill_rule(),
        LonghandId::StrokeWidth => style.clone_stroke_width() == parent.clone_stroke_width(),
        LonghandId::StrokeOpacity => style.clone_stroke_opacity() == parent.clone_stroke_opacity(),
        LonghandId::StrokeDasharray => {
            style.clone_stroke_dasharray() == parent.clone_stroke_dasharray()
        }
        LonghandId::StrokeDashoffset => {
            style.clone_stroke_dashoffset() == parent.clone_stroke_dashoffset()
        }
        LonghandId::StrokeLinecap => style.clone_stroke_linecap() == parent.clone_stroke_linecap(),
        LonghandId::StrokeLinejoin => {
            style.clone_stroke_linejoin() == parent.clone_stroke_linejoin()
        }
        LonghandId::StrokeMiterlimit => {
            style.clone_stroke_miterlimit() == parent.clone_stroke_miterlimit()
        }
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
