//! Floats inside inline content: where each goes as lines are broken
//! (CSS 2.1 §9.5.1), and the band of floats each line sits beside.

use blitz_traits::node_id::NodeId;
use parley::{BoxBreakData, BreakerState, Layout, LineBreakData, YieldData};
use std::collections::HashSet;
use taffy::{
    BlockContext, BlockItemStyle as _, Clear, CoreStyle as _, Float, LayoutInput,
    LayoutPartialTree as _, OofCandidates, ResolveOrZero as _,
};

use super::content_height;
use crate::BaseDocument;
use crate::layout::resolve_calc_value;
use crate::node::TextBrush;

/// What placing floats beside a container's lines needs from the container.
pub(super) struct FloatContainer {
    /// The width lines are broken at beside no float, scaled.
    pub(super) width: f32,
    pub(super) scale: f32,
    pub(super) padding_border: taffy::Rect<f32>,
    /// The inputs a float is laid out with.
    pub(super) float_inputs: LayoutInput,
}

impl BaseDocument {
    /// Break `layout`'s lines beside the floats of `block_ctx`, placing each
    /// float the content holds as the lines reach it.
    pub(super) fn break_lines_beside_floats(
        &mut self,
        layout: &mut Layout<TextBrush>,
        block_ctx: &mut BlockContext<'_>,
        container: &FloatContainer,
        oof_candidates: &mut OofCandidates,
    ) {
        let scale = container.scale;
        let mut floats = LineFloats::default();
        let mut breaker = layout.break_lines();
        let state = breaker.state_mut();
        state.set_layout_max_advance(container.width);
        let initial_slot = block_ctx.find_content_slot(0.0, Clear::None, None);
        floats.enter(state, &initial_slot, scale);

        while let Some(yield_data) = breaker.break_next() {
            match yield_data {
                YieldData::LineBreak(line) => {
                    let max_advance = breaker.state().line_max_advance();
                    if let Some(lower) = floats.slot_below(block_ctx, &line, max_advance, scale)
                        && breaker.revert()
                    {
                        floats.retry_in(breaker.state_mut(), &lower, scale);
                        continue;
                    }
                    let state = breaker.state_mut();

                    // Floats that did not fit beside the line go below it.
                    let deferred = floats.commit_line();
                    let placed_deferred = !deferred.is_empty();
                    let min_y = state.line_y() as f32 / scale;
                    self.place_inline_floats(block_ctx, deferred, min_y, container, oof_candidates);

                    if floats.beside_float() || placed_deferred {
                        floats.enter_below(block_ctx, state, scale);
                    } else {
                        state.set_line_x(0.0);
                        state.set_line_max_advance(container.width);
                    }
                }
                YieldData::MaxHeightExceeded(_) => {}
                YieldData::InlineBoxBreak(float_break) => self.meet_inline_float(
                    &mut floats,
                    breaker.state_mut(),
                    &float_break,
                    block_ctx,
                    container,
                    oof_candidates,
                ),
            }
        }
        breaker.finish();

        // A float that did not fit beside the last line goes below it.
        let min_y = content_height(layout) / scale;
        let deferred = floats.commit_line();
        self.place_inline_floats(block_ctx, deferred, min_y, container, oof_candidates);
    }

    /// Lay out the float a line met, and place it beside the line if it fits
    /// there; otherwise it waits below the line.
    fn meet_inline_float(
        &mut self,
        floats: &mut LineFloats,
        state: &mut BreakerState,
        float_break: &BoxBreakData,
        block_ctx: &mut BlockContext<'_>,
        container: &FloatContainer,
        oof_candidates: &mut OofCandidates,
    ) {
        let node_id = NodeId::from_u64(float_break.inline_box_id);
        if !floats.meet(node_id) {
            state.append_inline_box_to_line(float_break.advance, 0.0, 0.0, true);
            return;
        }
        let float = self.lay_out_inline_float(node_id, container.float_inputs);
        let float_advance =
            (float.output.size.width + float.margin.sum_axes().width) * container.scale;

        // A float that does not fit beside the line's content is placed below
        // the line (CSS 2.1 §9.5.1), as Chrome does once the line is complete.
        let fits = float_break.advance <= 0.0
            || float_break.advance + float_advance <= state.line_max_advance();
        if !fits {
            floats.defer(float);
            state.append_inline_box_to_line(float_break.advance, 0.0, 0.0, true);
            return;
        }

        floats.place_after(float_break.advance);
        let min_y = state.line_y() as f32 / container.scale;
        self.place_inline_floats(block_ctx, vec![float], min_y, container, oof_candidates);
        floats.enter_below(block_ctx, state, container.scale);
        state.append_inline_box_to_line(float_break.advance, 0.0, 0.0, true);
    }

    fn lay_out_inline_float(&mut self, node_id: NodeId, inputs: LayoutInput) -> DeferredFloat {
        let (direction, clear, margin) = {
            let style = self.nodes[node_id].layout_style();
            let direction = match style.float() {
                Float::Left => taffy::FloatDirection::Left,
                Float::Right => taffy::FloatDirection::Right,
                Float::None => {
                    unreachable!("only floats break lines at their box (`break_on_box`)")
                }
            };
            let margin = style
                .margin()
                .resolve_or_zero(inputs.parent_size, resolve_calc_value);
            (direction, style.clear(), margin)
        };
        let output = self.compute_child_layout(crate::taffy_node_id(node_id), inputs);
        DeferredFloat {
            node_id,
            output,
            margin,
            direction,
            clear,
        }
    }

    /// Place floats at or below `min_y` in the block formatting context.
    fn place_inline_floats(
        &mut self,
        block_ctx: &mut BlockContext<'_>,
        floats: Vec<DeferredFloat>,
        min_y: f32,
        container: &FloatContainer,
        oof_candidates: &mut OofCandidates,
    ) {
        for mut float in floats {
            // Note: `pos` is content-box relative
            let pos = block_ctx.place_floated_box(
                float.output.size + float.margin.sum_axes(),
                min_y,
                float.direction,
                float.clear,
                false,
            );
            let location = taffy::Point {
                x: pos.x + float.margin.left + container.padding_border.left,
                y: pos.y + float.margin.top + container.padding_border.top,
            };
            let layout = self.nodes[float.node_id].unrounded_layout_mut();
            layout.size = float.output.size;
            layout.location = location;

            // Translate anchors from item-relative to container-relative
            // coordinates and collect candidates bubbled from the float's subtree
            if !float.output.oof_candidates.is_empty() {
                float.output.oof_candidates.translate(location);
                oof_candidates.append(&mut float.output.oof_candidates);
            }
        }
    }
}

/// A float laid out at the point of the inline content where it occurs,
/// waiting to be placed.
struct DeferredFloat {
    node_id: NodeId,
    output: taffy::LayoutOutput,
    margin: taffy::Rect<f32>,
    direction: taffy::FloatDirection,
    clear: Clear,
}

/// The floats met while breaking lines, and the float band the current line
/// sits beside.
#[derive(Default)]
struct LineFloats {
    /// The float segment beside the current line; `None` beside no float.
    segment: Option<usize>,
    /// A float placed after content on the current line commits the line to
    /// its position.
    after_content: bool,
    /// Floats that did not fit beside the current line, placed below it.
    deferred: Vec<DeferredFloat>,
    /// Floats already placed or deferred, which a line laid out again passes.
    met: HashSet<NodeId>,
    /// How many times the current line moved to a lower band.
    retries: u32,
}

/// Each retry lays the line out again, so a page with many float bands
/// would cost bands × items per line; past this many bands the line moves
/// below all floats at once.
const MAX_LINE_RETRIES: u32 = 32;

impl LineFloats {
    /// Where a line too wide for the space beside floats moves: below them
    /// (CSS 2.1 §9.5), unless a float placed after its content committed it
    /// to this position. `None` keeps the line.
    fn slot_below(
        &self,
        block_ctx: &BlockContext<'_>,
        line: &LineBreakData,
        max_advance: f32,
        scale: f32,
    ) -> Option<taffy::ContentSlot> {
        let segment = self.segment?;
        if content_fits(line, max_advance) || self.after_content {
            return None;
        }
        let min_y = line.line_y_start as f32 / scale;
        let mut lower = block_ctx.find_content_slot(min_y, Clear::None, Some(segment));
        if lower.y <= min_y || self.retries >= MAX_LINE_RETRIES {
            // The last band of floats, or too many: below all of them.
            lower = block_ctx.find_content_slot(min_y, Clear::Both, None);
        }
        (lower.y > min_y).then_some(lower)
    }

    /// Lay the current line out again in the lower `slot`.
    fn retry_in(&mut self, state: &mut BreakerState, slot: &taffy::ContentSlot, scale: f32) {
        self.retries += 1;
        self.enter(state, slot, scale);
    }

    /// The current line is final: start counting for the next one, and take
    /// the floats that wait below this one.
    fn commit_line(&mut self) -> Vec<DeferredFloat> {
        self.after_content = false;
        self.retries = 0;
        std::mem::take(&mut self.deferred)
    }

    /// Note a float the line met. `false` if an earlier layout of the same
    /// line already met it.
    fn meet(&mut self, float: NodeId) -> bool {
        self.met.insert(float)
    }

    /// Note a float placed beside the line, `advance` into its content.
    fn place_after(&mut self, advance: f32) {
        self.after_content |= advance > 0.0;
    }

    fn defer(&mut self, float: DeferredFloat) {
        self.deferred.push(float);
    }

    fn beside_float(&self) -> bool {
        self.segment.is_some()
    }

    /// Start the next line in the content slot at the current line's y.
    fn enter_below(&mut self, block_ctx: &BlockContext<'_>, state: &mut BreakerState, scale: f32) {
        let min_y = state.line_y() / scale as f64;
        let slot = block_ctx.find_content_slot(min_y as f32, Clear::None, None);
        self.enter(state, &slot, scale);
    }

    /// Start the next line in `slot`.
    fn enter(&mut self, state: &mut BreakerState, slot: &taffy::ContentSlot, scale: f32) {
        self.segment = slot.segment_id;
        state.set_line_max_advance(slot.width * scale);
        state.set_line_x(slot.x * scale);
        state.set_line_y((slot.y * scale) as f64);
    }
}

/// Whether `line`'s content fits in `max_advance`. White space hanging past
/// the line's end may overflow the floats (CSS Text 4 §4.3.2). Preserved
/// white space at a forced break hangs only by its overflow, so the content
/// can measure `max_advance` give or take rounding: a relative tolerance far
/// below a pixel keeps such a line beside the floats.
fn content_fits(line: &LineBreakData, max_advance: f32) -> bool {
    let content = line.advance - line.hanging_advance;
    content - max_advance <= max_advance.abs().max(1.0) * 1e-5
}
