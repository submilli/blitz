//! Lay out buttons as the HTML standard's button layout describes, and as
//! Chrome's `LayoutButton` does: the button is a block container whose
//! content sits in one anonymous button content box, centered in the block
//! axis when it fits. <https://html.spec.whatwg.org/multipage/rendering.html#button-layout>
//!
//! A button whose `display` is flex or grid lays out its children as any
//! flex or grid container would, and takes none of this.

use style::values::specified::box_::{DisplayInside, DisplayOutside};
use taffy::{
    CoreStyle as _, LayoutInput, LayoutOutput, NodeId as TaffyNodeId, OofCandidates, Point,
    RequestedAxis, RunMode, Size,
};

use super::construct::LayoutChildren;
use super::content_block_extent;
use crate::node::{NodeFlags, NodeKind};
use crate::{BaseDocument, Node, NodeData, NodeId, dom_node_id};

/// Whether an element lays out its content in an anonymous button content
/// box: a `<button>`, or an `<input>` button, whose `display` is neither
/// flex nor grid. An inline button is laid out as an inline-block.
pub(super) fn has_content_box(node: &Node) -> bool {
    if !node.is_button() {
        return false;
    }
    let Some(display) = node.display_style() else {
        return false;
    };
    display.outside() != DisplayOutside::None
        && matches!(
            display.inside(),
            DisplayInside::Flow | DisplayInside::FlowRoot
        )
}

/// Whether a node is a button's anonymous content box, which establishes
/// an independent formatting context as `display: flow-root` does: its
/// children's margins and floats stay inside it.
pub(crate) fn is_content_box(node: &Node) -> bool {
    matches!(node.data, NodeData::AnonymousBlock(_))
        && node
            .layout_parent
            .get()
            .and_then(|parent| node.tree().get(parent))
            .is_some_and(|parent| parent.flags.is_button_root())
}

/// Collect a button's layout children: one anonymous block holding its
/// children and pseudo-elements, as Chrome's `LayoutButton` keeps its
/// content in one anonymous child. Even an empty button has the box.
///
/// Returns false, collecting nothing, when the document has no room for the
/// box; the button is then laid out as an ordinary block container.
pub(super) fn collect_content_box(
    doc: &mut BaseDocument,
    button: NodeId,
    out: &mut LayoutChildren,
) -> bool {
    if doc.check_node_allocation(1).is_err() {
        return false;
    }
    let content_box = out.create_anonymous_block(button, doc);
    let node = &doc.nodes[button];
    let content: Vec<NodeId> = node
        .before()
        .into_iter()
        .chain(node.flat_children().iter().copied())
        .chain(node.after())
        .filter(|&child| doc.nodes[child].data.kind() != NodeKind::Comment)
        .collect();
    doc.nodes[content_box].children.extend_from_slice(&content);
    doc.nodes[button].flags.insert(NodeFlags::IS_BUTTON_ROOT);
    true
}

/// Lay out a button: its content box as a block container's only child,
/// then moved to the middle of the button's content box when it fits there
/// (Chrome centers it with auto block margins, which keep overflowing
/// content at the start).
///
/// A button's baseline depends on where its content lands, so sizing it
/// lays the content out too: a short-circuited size would leave an inline
/// button on a different line height than its final layout.
pub(super) fn compute_button_layout(
    doc: &mut BaseDocument,
    button: TaffyNodeId,
    inputs: LayoutInput,
) -> LayoutOutput {
    if inputs.run_mode != RunMode::PerformLayout && inputs.axis == RequestedAxis::Horizontal {
        return taffy::compute_block_layout(doc, button, inputs, None);
    }
    let perform = LayoutInput {
        run_mode: RunMode::PerformLayout,
        ..inputs
    };
    let mut output = taffy::compute_block_layout(doc, button, perform, None);
    if inputs.run_mode != RunMode::PerformLayout {
        // Only a performed layout hands its out-of-flow descendants on.
        output.oof_candidates = OofCandidates::NONE;
    }
    let Some(content_box) = content_box_of(doc, button) else {
        return output;
    };
    let (inner_top, inner_height) = content_block_extent(
        &doc.nodes[dom_node_id(button)].layout_style(),
        inputs.parent_size.width,
        output.size.height,
    );
    let inner_height = inner_height.max(0.0);
    let content_height = doc.nodes[content_box].unrounded_layout().size.height;
    let offset = ((inner_height - content_height) / 2.0).max(0.0);
    if offset > 0.0 {
        doc.nodes[content_box].unrounded_layout_mut().location.y += offset;
        output.oof_candidates.translate(Point { x: 0.0, y: offset });
        output.scrollable_overflow_rect.top += offset;
        output.scrollable_overflow_rect.bottom += offset;
    }

    // Chrome's button with no line has its baseline at its content box's
    // block-end edge, rather than an inline-block's margin edge.
    let content_baseline = output.baselines.first.map(|first| first + offset);
    output.baselines.first = Some(content_baseline.unwrap_or(inner_top + inner_height));
    output
}

/// Lay out a button's content box. Percentages skip anonymous boxes and
/// resolve against the nearest non-anonymous ancestor, here the button's
/// content box (<https://www.w3.org/TR/CSS21/visuren.html#anonymous-block-level>).
/// When a child's block size needs that basis and the button's height is
/// definite, the box is laid out at that height, then given the height its
/// content takes, for the button to center. Otherwise it is laid out at its
/// auto height, as Taffy computes it.
pub(super) fn compute_content_box_layout(
    doc: &mut BaseDocument,
    content_box: TaffyNodeId,
    inputs: LayoutInput,
) -> LayoutOutput {
    let basis = inputs
        .parent_size
        .height
        .filter(|_| inputs.known_dimensions.height.is_none())
        // Only a performed layout places the children the extent is read from.
        .filter(|_| inputs.run_mode == RunMode::PerformLayout)
        .filter(|_| has_percentage_block_size_child(doc, dom_node_id(content_box)));
    let Some(height) = basis else {
        return lay_out_content(doc, content_box, inputs);
    };
    let at_button_height = LayoutInput {
        known_dimensions: Size {
            height: Some(height),
            ..inputs.known_dimensions
        },
        known_dimensions_are_definite: Size {
            height: true,
            ..inputs.known_dimensions_are_definite
        },
        ..inputs
    };
    let mut output = lay_out_content(doc, content_box, at_button_height);
    output.size.height = content_extent(doc, dom_node_id(content_box), height);
    output
}

fn lay_out_content(
    doc: &mut BaseDocument,
    content_box: TaffyNodeId,
    inputs: LayoutInput,
) -> LayoutOutput {
    // A flow root: no outer block formatting context reaches it.
    let id = dom_node_id(content_box);
    if doc.nodes[id].flags.is_inline_root() {
        doc.compute_inline_layout(id, inputs, None)
    } else {
        taffy::compute_block_layout(doc, content_box, inputs, None)
    }
}

/// Whether an in-flow child of the content box has a percentage `height`,
/// `min-height` or `max-height`, which would resolve against the button.
fn has_percentage_block_size_child(doc: &BaseDocument, content_box: NodeId) -> bool {
    let children = doc.nodes[content_box].layout_children.borrow();
    children
        .iter()
        .flatten()
        .filter_map(|&child| doc.nodes.get(child))
        .filter(|child| is_in_flow_box(child))
        .any(|child| {
            let style = child.layout_style();
            [
                style.size().height.into_raw(),
                style.min_size().height.into_raw(),
                style.max_size().height.into_raw(),
            ]
            .into_iter()
            .any(|height| height.uses_percentage())
        })
}

/// The height a content box laid out at `basis` takes as its auto height
/// would: for an inline root, its lines; otherwise its last in-flow child's
/// bottom margin edge, before any relative offset. Floats extend either, as
/// the box is a flow root.
fn content_extent(doc: &BaseDocument, content_box: NodeId, basis: f32) -> f32 {
    let node = &doc.nodes[content_box];
    let children = node.layout_children.borrow();
    let children: Vec<&Node> = children
        .iter()
        .flatten()
        .filter_map(|&child| doc.nodes.get(child))
        .filter(|child| is_in_flow_box(child))
        .collect();
    let bottom = |child: &Node| {
        let layout = child.unrounded_layout();
        // An empty box that its margins collapse through is placed past
        // them already.
        let collapses_through = layout.size.height == 0.0
            && layout.border.top
                + layout.border.bottom
                + layout.padding.top
                + layout.padding.bottom
                == 0.0;
        let margin = if collapses_through {
            0.0
        } else {
            layout.margin.bottom
        };
        layout.location.y + layout.size.height + margin
    };
    let floats = children
        .iter()
        .filter(|child| is_floated(child))
        .map(|child| bottom(child))
        .fold(0.0, f32::max);
    let flow = if node.flags.is_inline_root() {
        node.element_data()
            .and_then(|element| element.inline_layout_data.as_ref())
            .map_or(0.0, |text| {
                super::inline::content_height(&text.layout) / text.layout.scale()
            })
    } else {
        children
            .iter()
            .rfind(|child| !is_floated(child))
            .map_or(0.0, |child| bottom(child) - relative_offset(child, basis))
    };
    flow.max(floats)
}

/// A child that generates a box in the content box's flow, or floats in it:
/// not `display: none` (which Blitz keeps among layout children) and not
/// absolutely positioned.
fn is_in_flow_box(child: &Node) -> bool {
    child.primary_styles().is_some()
        && child.taffy_display() != taffy::Display::None
        && !child.taffy_position().is_out_of_flow()
}

fn is_floated(child: &Node) -> bool {
    child
        .primary_styles()
        .is_some_and(|style| style.clone_float().is_floating())
}

/// The block-axis offset `position: relative` moved a child by, which its
/// parent's auto height does not count.
fn relative_offset(child: &Node, basis: f32) -> f32 {
    let style = child.layout_style();
    if style.position() != taffy::Position::Relative {
        return 0.0;
    }
    let inset = style.inset();
    let resolve = |inset: taffy::LengthPercentageAuto| {
        inset.resolve_to_option(basis, super::resolve_calc_value)
    };
    resolve(inset.top)
        .or(resolve(inset.bottom).map(|bottom| -bottom))
        .unwrap_or(0.0)
}

fn content_box_of(doc: &BaseDocument, button: TaffyNodeId) -> Option<NodeId> {
    let children = doc.nodes[dom_node_id(button)].layout_children.borrow();
    let content_box = *children.as_ref()?.first()?;
    matches!(
        doc.nodes.get(content_box)?.data,
        NodeData::AnonymousBlock(_)
    )
    .then_some(content_box)
}
