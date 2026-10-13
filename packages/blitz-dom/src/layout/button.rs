//! Lay out buttons as the HTML standard's button layout describes, and as
//! Chrome's `LayoutButton` does: the button is a block container whose
//! content sits in one anonymous button content box, centered in the block
//! axis when it fits. <https://html.spec.whatwg.org/multipage/rendering.html#button-layout>
//!
//! A button whose `display` is flex or grid lays out its children as any
//! flex or grid container would, and takes none of this.

use style::values::specified::box_::{DisplayInside, DisplayOutside};
use taffy::{
    LayoutInput, LayoutOutput, NodeId as TaffyNodeId, OofCandidates, Point, RequestedAxis, RunMode,
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

fn content_box_of(doc: &BaseDocument, button: TaffyNodeId) -> Option<NodeId> {
    let children = doc.nodes[dom_node_id(button)].layout_children.borrow();
    let content_box = *children.as_ref()?.first()?;
    matches!(
        doc.nodes.get(content_box)?.data,
        NodeData::AnonymousBlock(_)
    )
    .then_some(content_box)
}
