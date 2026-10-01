//! Work still owned by the engine between exclusive embedder checkpoints.
use crate::custom_elements::CustomElementReaction;
use crate::events::DragMode;
use crate::layout::construct::ConstructionTaskData;
use crate::mutations::MutationRecord;
use crate::scrolling::{ScrollAnimationState, ScrollTarget};
use crate::{BaseDocument, NodeId};

pub(super) fn native_work(doc: &BaseDocument) -> Vec<NodeId> {
    let mut roots = vec![doc.root_node_id];
    roots.extend(&doc.connected_scripts);
    roots.extend(doc.pending_resource_nodes());
    roots.extend(
        doc.pending_images
            .values()
            .flat_map(|pending| &pending.waiters)
            .map(|&(id, _)| id),
    );
    roots.extend(doc.failed_image_inputs.keys());
    roots.extend(&doc.pending_style_image_nodes);
    roots.extend(doc.iframe_loads.keys());
    roots.extend(&doc.sub_document_nodes);
    #[cfg(feature = "custom-widget")]
    roots.extend(&doc.custom_widget_nodes);
    if let Some(log) = &doc.mutation_log {
        for entry in log {
            roots.extend(&entry.ancestors);
            mutation_roots(&entry.record, &mut roots);
        }
    }
    if let Some(reactions) = &doc.custom_element_reactions {
        for reaction in reactions {
            roots.push(match reaction {
                CustomElementReaction::Connected(id) | CustomElementReaction::Disconnected(id) => {
                    *id
                }
                CustomElementReaction::AttributeChanged { element, .. } => *element,
            });
        }
    }
    for task in &doc.deferred_construction_nodes {
        roots.push(task.node_id);
        match &task.data {
            ConstructionTaskData::InlineLayout(text) => {
                roots.extend(text.layout.styles().iter().map(|style| style.brush.id));
                roots.extend(
                    text.layout
                        .inline_boxes()
                        .iter()
                        .map(|item| NodeId::from_u64(item.id)),
                );
            }
        }
    }
    interaction_roots(doc, &mut roots);
    roots
}

fn mutation_roots(record: &MutationRecord, roots: &mut Vec<NodeId>) {
    roots.push(record.target());
    if let MutationRecord::ChildList {
        added,
        removed,
        previous_sibling,
        next_sibling,
        ..
    } = record
    {
        roots.extend(added);
        roots.extend(removed);
        roots.extend(previous_sibling);
        roots.extend(next_sibling);
    }
}

fn interaction_roots(doc: &BaseDocument, roots: &mut Vec<NodeId>) {
    roots.extend(
        [
            doc.hover_node_id,
            doc.hover_hit_node_id,
            doc.focus_node_id,
            doc.active_node_id,
            doc.mousedown_node_id,
            doc.text_selection.anchor.node_or_parent,
            doc.text_selection.focus.node_or_parent,
            doc.hovered_scrollbar.map(|bar| bar.node_id),
        ]
        .into_iter()
        .flatten(),
    );
    match &doc.drag_mode {
        DragMode::Panning(pan) => roots.push(pan.target),
        DragMode::ScrollbarDrag(drag) => roots.push(drag.scrollbar.node_id),
        DragMode::None | DragMode::Selecting => {}
    }
    match &doc.scroll_animation {
        ScrollAnimationState::Fling(fling) => roots.push(fling.target),
        ScrollAnimationState::ScrollTo(scroll) => {
            if let ScrollTarget::Node(id) = scroll.target {
                roots.push(id);
            }
        }
        ScrollAnimationState::None => {}
    }
}
