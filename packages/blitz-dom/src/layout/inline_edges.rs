//! Non-atomic inline decorations, measured against the containing block.

use crate::{Node, font_metrics::first_available_or_fallback_font_metrics, node::TextBrush};
use parley::{FontContext, InlineBox, InlineBoxKind, TreeBuilder};
use stylo_taffy::convert::layout_length;
use taffy::{
    CoreStyle as _, Direction, LengthPercentage, LengthPercentageAuto, ResolveOrZero,
    prelude::TaffyZero,
};

/// Insert a potentially nonzero horizontal edge. Undecorated spans remain
/// transparent to whitespace processing and shaping.
pub(super) fn push_edge(builder: &mut TreeBuilder<TextBrush>, node: &Node, start: bool) {
    let style = node.layout_style();
    let left = start == (style.direction() == Direction::Ltr);
    let margin = style.margin();
    let padding = style.padding();
    let border = style.border();
    let (margin, padding, border) = if left {
        (margin.left, padding.left, border.left)
    } else {
        (margin.right, padding.right, border.right)
    };
    if (margin == LengthPercentageAuto::ZERO || margin.is_auto())
        && padding == LengthPercentage::ZERO
        && border == LengthPercentage::ZERO
    {
        return;
    }
    builder.push_inline_box(InlineBox {
        id: node.id.as_u64(),
        kind: if start {
            InlineBoxKind::SpanStart
        } else {
            InlineBoxKind::SpanEnd
        },
        index: 0,
        // Percentages require the containing block's width, available at layout.
        width: 0.0,
        height: 0.0,
        baseline: None,
    });
}

/// CSS 2.1 §8.3/8.4: percentages use the containing block's inline size.
/// Decorations contribute signed width but no line height or atomic box.
pub(super) fn measure_edge(
    node: &Node,
    kind: InlineBoxKind,
    parent_width: Option<f32>,
    scale: f32,
) -> f32 {
    let style = node.layout_style();
    let padding = style
        .padding()
        .resolve_or_zero(parent_width, super::resolve_calc_value);
    let border = style
        .border()
        .resolve_or_zero(parent_width, super::resolve_calc_value);
    let margin = style
        .margin()
        .resolve_or_zero(parent_width, super::resolve_calc_value);
    let left = (kind == InlineBoxKind::SpanStart) == (style.direction() == Direction::Ltr);
    let (margin, padding, border) = if left {
        (margin.left, padding.left, border.left)
    } else {
        (margin.right, padding.right, border.right)
    };
    // Percentage resolution can overflow first; bound each component before
    // summing, and bound the scaled result to the engine's layout range.
    layout_length((layout_length(margin) + layout_length(padding) + layout_length(border)) * scale)
}

/// A decorated empty span or a span around an atomic child still contributes
/// its font and line height; padding and borders do not change those extents.
pub(super) fn font_box(
    node: &Node,
    root: &Node,
    font_ctx: &mut FontContext,
    scale: f32,
) -> (f32, Option<f32>) {
    let Some(style) = node.primary_styles() else {
        return (0.0, None);
    };
    let root_height = root
        .primary_styles()
        .map(|style| super::construct::resolve_line_height(font_ctx, &style, scale))
        .unwrap_or(0.0);
    let height = layout_length(
        super::construct::resolve_line_height(font_ctx, &style, scale).max(root_height) * scale,
    )
    .max(0.0);
    let font = style.get_font();
    let metrics = first_available_or_fallback_font_metrics(
        font_ctx,
        font,
        font.font_size.used_size.0.px(),
        scale,
    );
    let baseline = metrics.map(|metrics| {
        let ascent = metrics.ascent * scale;
        let descent = metrics.descent * scale;
        ascent + ((height - ascent - descent) / 2.0).floor()
    });
    (height, baseline)
}
