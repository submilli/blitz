//! Border rectangles for non-atomic span decorations.

use crate::{BaseDocument, NodeId, font_metrics::first_available_or_fallback_font_metrics};
use parley::{InlineBoxKind, Line, PositionedInlineBox};
use stylo_taffy::convert::layout_length;
use taffy::{CoreStyle as _, Direction, ResolveOrZero};

pub(super) fn border_rect(
    doc: &BaseDocument,
    target: NodeId,
    inline: &PositionedInlineBox,
    root_width: f32,
    line: &Line<'_, crate::node::TextBrush>,
    scale: f32,
) -> Option<(f64, f64, f64, f64)> {
    let node = doc.get_node(NodeId::from_u64(inline.id))?;
    let styles = node.primary_styles()?;
    let font = styles.get_font();
    let metrics = first_available_or_fallback_font_metrics(
        &mut *doc.font_ctx().lock().ok()?,
        font,
        font.font_size.used_size.0.px(),
        scale,
    )?;
    let (mut x0, mut x1) = (inline.x, inline.x + inline.width);
    if node.id == target {
        // CSSOM rectangles include padding and border, but exclude the
        // target's margins. Descendant decorations consume target content.
        let style = node.layout_style();
        let margin = style
            .margin()
            .resolve_or_zero(Some(root_width), crate::layout::resolve_calc_value);
        let left =
            (inline.kind == InlineBoxKind::SpanStart) == (style.direction() == Direction::Ltr);
        if left {
            x0 += layout_length(margin.left) * scale;
        } else {
            x1 -= layout_length(margin.right) * scale;
        }
    }
    let baseline = line.metrics().baseline;
    Some((
        f64::from(x0.min(x1)),
        f64::from(baseline - metrics.ascent * scale),
        f64::from(x0.max(x1)),
        f64::from(baseline + metrics.descent * scale),
    ))
}
