use super::*;
use crate::{DocumentConfig, QualName, ns};

fn element(doc: &mut BaseDocument, name: &str) -> NodeId {
    doc.mutate()
        .try_create_element(QualName::new(None, ns!(html), name.into()), vec![])
        .unwrap()
}

#[test]
fn detached_descendant_keeps_ancestors_siblings_templates_and_shadows() {
    let mut doc = BaseDocument::new(DocumentConfig::default());
    let parent = element(&mut doc, "div");
    let child = element(&mut doc, "span");
    let sibling = element(&mut doc, "template");
    let contents = doc.mutate().try_ensure_template_contents(sibling).unwrap();
    let shadow = doc
        .mutate()
        .attach_shadow(child, crate::shadow::ShadowRootInit::default())
        .unwrap();
    doc.mutate().append_children(parent, &[child, sibling]);
    let unreferenced = element(&mut doc, "div");
    let graph = doc.node_reachability();
    let retained = graph.retained_nodes(&[contents]);
    for id in [parent, child, sibling, contents, shadow, doc.root_node_id] {
        assert!(retained.contains(&id), "{id:?}");
    }
    assert!(!retained.contains(&unreferenced));
    assert_eq!(
        graph.retained_nodes(&[]),
        HashSet::from([doc.root_node_id, doc.template_document(doc.root_node_id)])
    );
}

#[test]
fn pending_work_keeps_detached_nodes_until_drained() {
    let mut doc = BaseDocument::new(DocumentConfig::default());
    let target = element(&mut doc, "div");
    let child = element(&mut doc, "b");
    doc.mutate().append_children(target, &[child]);
    doc.set_mutation_recording(true);
    doc.mutate().remove_node(child);
    assert!(doc.node_reachability().retained_nodes(&[]).contains(&child));
    doc.take_logged_mutations();
    assert!(!doc.node_reachability().retained_nodes(&[]).contains(&child));
    let pin = doc.resource_pin(child);
    assert!(doc.node_reachability().retained_nodes(&[]).contains(&child));
    drop(pin);
    assert!(!doc.node_reachability().retained_nodes(&[]).contains(&child));
}

#[test]
fn stale_slots_and_layout_backlinks_do_not_keep_detached_nodes() {
    let mut doc = BaseDocument::new(DocumentConfig::default());
    let old = element(&mut doc, "div");
    doc.mutate().remove_and_drop_node(old);
    let new = element(&mut doc, "span");
    doc.nodes[new].layout_parent.set(Some(doc.root_node_id));
    let graph = doc.node_reachability();
    assert_ne!(old, new);
    assert!(!graph.retained_nodes(&[old]).contains(&new));
    assert!(graph.retained_nodes(&[new]).contains(&new));
}

#[test]
fn deep_detached_graph_walk_is_iterative() {
    let mut doc = BaseDocument::new(DocumentConfig::default());
    let first = element(&mut doc, "template");
    let mut last = first;
    for _ in 0..5_000 {
        let fragment = doc.mutate().try_ensure_template_contents(last).unwrap();
        let next = element(&mut doc, "template");
        doc.mutate().append_children(fragment, &[next]);
        last = next;
    }
    let graph = doc.node_reachability();
    assert_eq!(graph.retained_nodes(&[last]).len(), doc.node_count());
    assert_eq!(graph.retained_nodes(&[]).len(), 2);
}

#[test]
fn queued_reactions_and_images_release_their_native_roots() {
    use crate::custom_elements::CustomElementReaction;
    let mut doc = BaseDocument::new(DocumentConfig::default());
    let reaction = element(&mut doc, "x-widget");
    let image = element(&mut doc, "img");
    let background = element(&mut doc, "div");
    doc.set_custom_element_reactions(true);
    doc.record_custom_element_reaction(CustomElementReaction::Disconnected(reaction));
    doc.pending_images.insert(
        "fixture".into(),
        crate::image_request::PendingImage::new(0, image, crate::util::ImageType::Image),
    );
    doc.pending_style_image_nodes.push(background);
    let retained = doc.node_reachability().retained_nodes(&[]);
    for id in [reaction, image, background] {
        assert!(retained.contains(&id));
    }
    doc.take_custom_element_reactions();
    doc.pending_images.clear();
    doc.pending_style_image_nodes.clear();
    assert_eq!(
        doc.node_reachability().retained_nodes(&[]),
        HashSet::from([doc.root_node_id, doc.template_document(doc.root_node_id)])
    );
}

#[test]
fn frame_load_and_scroll_animation_release_their_native_roots() {
    use crate::scrolling::{FlingState, ScrollAnimationState};
    let mut doc = BaseDocument::new(DocumentConfig::default());
    let frame = element(&mut doc, "iframe");
    let scroller = element(&mut doc, "div");
    doc.iframe_loads.insert(
        frame,
        crate::iframe::IframeLoad {
            request_id: None,
            abort_controller: Default::default(),
        },
    );
    doc.scroll_animation = ScrollAnimationState::Fling(FlingState {
        target: scroller,
        last_seen_time: 0.0,
        x_velocity: 1.0,
        y_velocity: 0.0,
    });
    let retained = doc.node_reachability().retained_nodes(&[]);
    assert!(retained.contains(&frame));
    assert!(retained.contains(&scroller));
    doc.iframe_loads.clear();
    doc.scroll_animation = ScrollAnimationState::None;
    assert_eq!(
        doc.node_reachability().retained_nodes(&[]),
        HashSet::from([doc.root_node_id, doc.template_document(doc.root_node_id)])
    );
}

#[test]
fn deferred_inline_layout_keeps_brush_and_box_nodes_until_drained() {
    use crate::layout::construct::{ConstructionTask, ConstructionTaskData};
    use crate::node::{TextBrush, TextLayout};
    let mut doc = BaseDocument::new(DocumentConfig::default());
    let root = element(&mut doc, "div");
    let span = element(&mut doc, "span");
    let inline_box = element(&mut doc, "img");
    let mut text = TextLayout::new();
    let mut fonts = crate::test_font_ctx();
    let mut context = parley::LayoutContext::new();
    let style = parley::TextStyle {
        brush: TextBrush::from_id(span),
        ..Default::default()
    };
    let mut builder = context.tree_builder(&mut fonts, 1.0, true, &style);
    builder.push_text("text");
    builder.push_inline_box(parley::InlineBox {
        id: inline_box.as_u64(),
        kind: parley::InlineBoxKind::InFlow,
        index: 0,
        width: 1.0,
        height: 1.0,
        baseline: None,
    });
    text.text = builder.build_into(&mut text.layout);
    doc.deferred_construction_nodes.push(ConstructionTask {
        node_id: root,
        data: ConstructionTaskData::InlineLayout(Box::new(text)),
    });
    let retained = doc.node_reachability().retained_nodes(&[]);
    for id in [root, span, inline_box] {
        assert!(retained.contains(&id), "{id:?}");
    }
    doc.deferred_construction_nodes.clear();
    assert_eq!(
        doc.node_reachability().retained_nodes(&[]),
        HashSet::from([doc.root_node_id, doc.template_document(doc.root_node_id)])
    );
}
