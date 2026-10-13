//! Lay out a `<select>`'s content as Chrome's user-agent shadow tree does,
//! with Chrome 154's metrics (macOS): a drop-down is as wide as its
//! longest option label plus its button part, and one line tall; a list box
//! is as wide as its widest row plus a scrollbar, and `size` rows tall.

use super::resolve_calc_value;
use crate::font_metrics::{RoundedFontMetrics, first_available_or_fallback_font_metrics};
use crate::node::{
    DropDownLayout, ListBoxLayout, ListBoxRowLayout, SelectContent, SelectLayout, TextBrush,
};
use crate::select::{ListBoxRow, RowKind, SelectDisplay};
use crate::{BaseDocument, NodeId, stylo_to_parley};
use parley::{FontContext, LayoutContext};
use std::sync::{Mutex, MutexGuard};
use style::Atom;
use style::computed_values::direction::T as Direction;
use style::properties::ComputedValues;
use style::servo_arc::Arc;
use style::values::computed::{Length, NonNegativeLengthPercentage, TextTransform};
use taffy::ResolveOrZero as _;

// Chrome's fixed metrics, in CSS px before `zoom`.

/// A drop-down's label sits this far from the content box's start edge.
const DROP_DOWN_LABEL_INSET: f32 = 4.0;
/// The button part at a drop-down's end edge, arrow included.
const DROP_DOWN_BUTTON_WIDTH: f32 = 16.0;
/// A drop-down's label line has a pixel of padding above and below.
const DROP_DOWN_LINE_PADDING: f32 = 1.0;
/// The scrollbar a list box reserves at its end edge.
const LIST_BOX_SCROLLBAR_WIDTH: f32 = 15.0;
/// A list box row's minimum height, in ems of its font (Chrome's
/// `option { min-block-size: 1.2em }`).
const ROW_MIN_HEIGHT_EM: f32 = 1.2;
/// An option group label's padding: start and end, and below.
const GROUP_LABEL_PADDING_INLINE: f32 = 2.0;
const GROUP_LABEL_PADDING_BOTTOM: f32 = 1.0;
/// The bottom padding of a row in a list box without rows.
const EMPTY_ROW_PADDING_BOTTOM: f32 = 1.0;
/// Chrome measures a grouped option in a drop-down with this indent.
const GROUPED_OPTION_INDENT: &str = "    ";
/// Chrome's largest layout length (`LayoutUnit::Max`), which bounds a list
/// box of many rows.
const MAX_LAYOUT_LENGTH: f32 = 33_554_432.0;
/// Chrome's layout lengths are in 64ths of a pixel.
const LAYOUT_UNITS_PER_PX: f32 = 64.0;

/// Lay out a select's content into its element data, or clear it for a
/// node that is not a select.
pub(super) fn build_select_layout(doc: &mut BaseDocument, select: NodeId) {
    let plan = plan(doc, select);
    let scale = doc.viewport.scale();
    let layout = plan.map(|plan| {
        let mut font_ctx = lock_fonts(&doc.font_ctx);
        let mut shaper = Shaper {
            font_ctx: &mut font_ctx,
            layout_ctx: &mut doc.layout_ctx,
            scale,
        };
        plan.lay_out(&mut shaper)
    });
    if let Some(element) = doc.nodes[select].element_data_mut() {
        element.select_layout = layout.map(Box::new);
    }
}

/// Size a select from its content, and give a drop-down its label's
/// baseline.
pub(super) fn leaf_layout(
    inputs: taffy::LayoutInput,
    style: &impl taffy::CoreStyle<CustomIdent = Atom>,
    select: &SelectLayout,
) -> taffy::LayoutOutput {
    let mut output = taffy::compute_leaf_layout(inputs, style, resolve_calc_value, |_, _| {
        select.content_size
    });
    // Chrome's layout lengths stop at its largest, padding and border
    // included.
    output.size.width = output.size.width.min(MAX_LAYOUT_LENGTH);
    output.size.height = output.size.height.min(MAX_LAYOUT_LENGTH);
    let padding = style
        .padding()
        .resolve_or_zero(inputs.parent_size.width, resolve_calc_value);
    let border = style
        .border()
        .resolve_or_zero(inputs.parent_size.width, resolve_calc_value);
    let top = padding.top + border.top;
    let content_height = output.size.height - top - padding.bottom - border.bottom;
    // Within the box: huge author padding must not carry the baseline to
    // infinity, where line breaking could never place the box.
    let baseline = select
        .content_baseline(content_height)
        .map(|baseline| top + baseline)
        .filter(|baseline| baseline.is_finite())
        .map(|baseline| baseline.clamp(0.0, output.size.height));
    output.baselines = taffy::Baselines {
        first: baseline,
        last: baseline,
    };
    output
}

/// What a select's layout needs from the document: the labels and their
/// styles.
enum Plan {
    DropDown(DropDownPlan),
    ListBox(ListBoxPlan),
}

struct DropDownPlan {
    select: NodeId,
    style: Arc<ComputedValues>,
    metrics: RoundedFontMetrics,
    /// Every option's label as Chrome measures it.
    measured: Vec<String>,
    selected: Option<String>,
}

struct ListBoxPlan {
    size: u32,
    metrics: RoundedFontMetrics,
    /// `zoom`'s scale of Chrome's fixed metrics.
    zoom: f32,
    rows: Vec<RowPlan>,
}

struct RowPlan {
    element: NodeId,
    style: Arc<ComputedValues>,
    label: String,
    metrics: RoundedFontMetrics,
    padding: Padding,
    rtl: bool,
    selected: bool,
}

struct Padding {
    top: f32,
    bottom: f32,
    left: f32,
    right: f32,
}

fn plan(doc: &BaseDocument, select: NodeId) -> Option<Plan> {
    let display = doc.select_display(select)?;
    let style = doc.nodes[select]
        .primary_styles()
        .map(|style| Arc::clone(&style))?;
    let metrics = font_metrics(doc, &style);
    Some(match display {
        SelectDisplay::DropDown => Plan::DropDown(drop_down_plan(doc, select, style, metrics)),
        SelectDisplay::ListBox { size } => {
            let zoom = style.effective_zoom.zoom(1.0);
            let rows = doc
                .list_box_rows(select)
                .into_iter()
                .map(|row| row_plan(doc, row, &style, zoom))
                .collect();
            Plan::ListBox(ListBoxPlan {
                size,
                metrics,
                zoom,
                rows,
            })
        }
    })
}

/// Chrome's `MenuListSelectType::UpdateOptionsWidth`: every option counts,
/// shown or not, a grouped one with its indent.
fn drop_down_plan(
    doc: &BaseDocument,
    select: NodeId,
    style: Arc<ComputedValues>,
    metrics: RoundedFontMetrics,
) -> DropDownPlan {
    let measured = doc
        .select_options(select)
        .into_iter()
        .map(|option| {
            let label = doc.option_display_label(option);
            if doc.option_is_grouped(option) {
                format!("{GROUPED_OPTION_INDENT}{label}")
            } else {
                label
            }
        })
        .collect();
    let selected = doc
        .selected_option(select)
        .map(|option| doc.option_display_label(option))
        .filter(|label| !label.is_empty());
    DropDownPlan {
        select,
        style,
        metrics,
        measured,
        selected,
    }
}

/// A row in its element's style (the select's when it has none): an
/// option with its own padding, a group label with Chrome's.
fn row_plan(
    doc: &BaseDocument,
    row: ListBoxRow,
    select_style: &Arc<ComputedValues>,
    zoom: f32,
) -> RowPlan {
    let style = doc.nodes[row.element]
        .primary_styles()
        .map_or_else(|| select_style.clone(), |row_style| Arc::clone(&row_style));
    let padding = match row.kind {
        RowKind::Option => {
            let padding = style.get_padding();
            Padding {
                top: px(&padding.padding_top),
                bottom: px(&padding.padding_bottom),
                left: px(&padding.padding_left),
                right: px(&padding.padding_right),
            }
        }
        RowKind::GroupLabel => Padding {
            top: 0.0,
            bottom: zoom * GROUP_LABEL_PADDING_BOTTOM,
            left: zoom * GROUP_LABEL_PADDING_INLINE,
            right: zoom * GROUP_LABEL_PADDING_INLINE,
        },
    };
    RowPlan {
        element: row.element,
        metrics: font_metrics(doc, &style),
        style,
        label: doc.list_box_row_label(row),
        padding,
        rtl: select_style.clone_direction() == Direction::Rtl,
        selected: row.kind == RowKind::Option && doc.option_selectedness(row.element),
    }
}

impl Plan {
    fn lay_out(self, shaper: &mut Shaper<'_>) -> SelectLayout {
        match self {
            Plan::DropDown(plan) => drop_down(shaper, plan),
            Plan::ListBox(plan) => list_box(shaper, plan),
        }
    }
}

/// A drop-down: its widest label rounded up to a whole pixel, as Chrome
/// does, beside its button part; one line of the font's height and a
/// pixel of padding above and below.
fn drop_down(shaper: &mut Shaper<'_>, plan: DropDownPlan) -> SelectLayout {
    let DropDownPlan {
        select,
        style,
        metrics,
        measured,
        selected,
    } = plan;
    let widest = measured
        .iter()
        .map(|label| text_width(&shaper.shape(select, &style, label), label))
        .fold(0.0, f32::max);
    let label = selected.map(|label| Box::new(shaper.shape(select, &style, &label)));
    let zoom = |length: f32| style.effective_zoom.zoom(length);
    let line_padding = zoom(DROP_DOWN_LINE_PADDING);
    let line_height = metrics.ascent + metrics.descent + 2.0 * line_padding;
    let label_inset = zoom(DROP_DOWN_LABEL_INSET);
    let button_width = zoom(DROP_DOWN_BUTTON_WIDTH);
    SelectLayout {
        content_size: taffy::Size {
            width: (widest.ceil() + label_inset + button_width).min(MAX_LAYOUT_LENGTH),
            height: line_height,
        },
        content: SelectContent::DropDown(DropDownLayout {
            label,
            line_height,
            baseline: line_padding + metrics.ascent,
            label_inset,
            button_width,
        }),
    }
}

/// Chrome's list box: rows as blocks of their own font and padding, at
/// least 1.2em tall; `size` rows as tall as the tallest; and a scrollbar.
fn list_box(shaper: &mut Shaper<'_>, plan: ListBoxPlan) -> SelectLayout {
    let ListBoxPlan {
        size,
        metrics,
        zoom,
        rows: plans,
    } = plan;
    let mut rows = Vec::with_capacity(plans.len());
    let mut widest: f32 = 0.0;
    let mut tallest: f32 = 0.0;
    let mut top = 0.0;
    for plan in plans {
        let label = shaper.shape(plan.element, &plan.style, &plan.label);
        let font_size = plan.style.get_font().font_size.used_size.0.px();
        let line_height = plan.metrics.ascent + plan.metrics.descent + plan.metrics.leading;
        let padding = &plan.padding;
        let height = line_height.max(snap_down(ROW_MIN_HEIGHT_EM * font_size))
            + padding.top
            + padding.bottom;
        widest = widest.max(text_width(&label, &plan.label) + padding.left + padding.right);
        tallest = tallest.max(height);
        rows.push(ListBoxRowLayout {
            element: plan.element,
            label: Box::new(label),
            top,
            height,
            inset: if plan.rtl {
                padding.right
            } else {
                padding.left
            },
            baseline: padding.top + plan.metrics.leading / 2.0 + plan.metrics.ascent,
            selected: plan.selected,
        });
        top += height;
    }
    if rows.is_empty() {
        tallest = metrics.ascent + metrics.descent + zoom * EMPTY_ROW_PADDING_BOTTOM;
    }
    let height = (size as f32 * tallest).min(MAX_LAYOUT_LENGTH);
    // Rows below the box are never drawn: it does not scroll.
    rows.retain(|row| row.top < height);
    let scrollbar_width = zoom * LIST_BOX_SCROLLBAR_WIDTH;
    SelectLayout {
        content_size: taffy::Size {
            width: (widest + scrollbar_width).min(MAX_LAYOUT_LENGTH),
            height,
        },
        content: SelectContent::ListBox(ListBoxLayout {
            rows,
            scrollbar_width,
        }),
    }
}

struct Shaper<'a> {
    font_ctx: &'a mut FontContext,
    layout_ctx: &'a mut LayoutContext<TextBrush>,
    scale: f32,
}

impl Shaper<'_> {
    /// A label on one line in an element's style, its white space kept:
    /// labels arrive collapsed, and an indent must stay.
    fn shape(
        &mut self,
        element: NodeId,
        style: &ComputedValues,
        text: &str,
    ) -> parley::Layout<TextBrush> {
        let mut parley_style = stylo_to_parley::style(element, style);
        parley_style.white_space_collapse = parley::WhiteSpaceCollapse::Preserve;
        parley_style.text_wrap_mode = parley::TextWrapMode::NoWrap;
        parley_style.line_height = parley::LineHeight::MetricsRelative(1.0);
        let mut builder =
            self.layout_ctx
                .tree_builder(self.font_ctx, self.scale, true, &parley_style);
        // Case transforms apply as they do to inline text.
        match style.clone_text_transform() & TextTransform::CASE_TRANSFORMS {
            TextTransform::UPPERCASE => builder.push_text(&text.to_uppercase()),
            TextTransform::LOWERCASE => builder.push_text(&text.to_lowercase()),
            _ => builder.push_text(text),
        }
        let mut layout = builder.build().0;
        layout.break_all_lines(None);
        layout
    }
}

/// The advance of a one-line layout of `text`, in CSS px. An empty line
/// has none, though Parley gives it a space's.
fn text_width(layout: &parley::Layout<TextBrush>, text: &str) -> f32 {
    if text.is_empty() {
        return 0.0;
    }
    layout.full_width() / layout.scale()
}

fn font_metrics(doc: &BaseDocument, style: &ComputedValues) -> RoundedFontMetrics {
    let font = style.get_font();
    let font_size = font.font_size.used_size.0.px();
    first_available_or_fallback_font_metrics(
        &mut lock_fonts(&doc.font_ctx),
        font,
        font_size,
        doc.viewport.scale(),
    )
    .unwrap_or(RoundedFontMetrics {
        // No font at all: proportions of a typical sans-serif.
        ascent: (font_size * 0.9).round(),
        descent: (font_size * 0.2).round(),
        leading: 0.0,
    })
}

fn lock_fonts(font_ctx: &Mutex<FontContext>) -> MutexGuard<'_, FontContext> {
    font_ctx
        .lock()
        .expect("the font context lock is not poisoned: layout never panics holding it")
}

fn px(length: &NonNegativeLengthPercentage) -> f32 {
    length.0.resolve(Length::new(0.0)).px()
}

/// Round down to Chrome's layout unit. A length a hair below a unit, as
/// 1.2em of the 13.333333px control font is in `f32`, is that unit: Chrome
/// computes it as exactly 16px.
fn snap_down(length: f32) -> f32 {
    const SLACK: f32 = 1e-3;
    ((length * LAYOUT_UNITS_PER_PX) + SLACK).floor() / LAYOUT_UNITS_PER_PX
}
