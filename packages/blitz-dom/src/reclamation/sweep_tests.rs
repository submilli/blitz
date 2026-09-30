use super::*;
use crate::dom_events::{EventTargetId, ListenerOptions};
use crate::node::FlatParent;
use crate::{DocumentConfig, QualName, ns};

fn element(doc: &mut BaseDocument, name: &str) -> NodeId {
    doc.mutate()
        .try_create_element(QualName::new(None, ns!(html), name.into()), vec![])
        .unwrap()
}

#[test]
fn sweep_preserves_exposed_subtrees_and_recovers_admission_capacity() {
    let mut doc = BaseDocument::new(DocumentConfig::default());
    doc.node_limit = 65;
    let parent = element(&mut doc, "div");
    let child = element(&mut doc, "span");
    doc.mutate().append_children(parent, &[child]);
    let mut last_dead = None;
    for _ in 0..100 {
        for _ in 0..61 {
            let id = element(&mut doc, "div");
            assert_ne!(Some(id), last_dead);
            last_dead = Some(id);
        }
        assert_eq!(doc.node_count(), 65);
        assert_eq!(doc.reclaim_detached_nodes(&[child]).len(), 61);
        assert_eq!(doc.nodes[child].parent, Some(parent));
        assert_eq!(doc.node_count(), 4);
    }
    assert_eq!(doc.reclaim_detached_nodes(&[]).len(), 2);
    assert_eq!(doc.node_count(), 2);
}

#[test]
fn sweep_recomputes_pending_roots_and_prunes_listener_and_index_entries() {
    let mut doc = BaseDocument::new(DocumentConfig::default());
    let dead = element(&mut doc, "div");
    let live = element(&mut doc, "input");
    doc.nodes_to_id.insert("old".into(), HashSet::from([dead]));
    doc.controls_to_form.insert(live, dead);
    doc.scrollbar_activity.insert(dead, 0.0);
    doc.changed_nodes.insert(dead);
    let target = EventTargetId::Node(dead);
    doc.event_listeners
        .add(target, "x", 1, ListenerOptions::default());
    let listeners = doc.event_listeners.listeners(target, "x");
    // A snapshot taken before this pin cannot authorize collection.
    let _old_graph = doc.node_reachability();
    let pin = doc.resource_pin(dead);
    assert!(doc.reclaim_detached_nodes(&[live]).is_empty());
    drop(pin);
    assert_eq!(doc.reclaim_detached_nodes(&[live]), vec![dead]);
    assert!(listeners[0].removed.get());
    assert!(doc.event_listeners.listeners(target, "x").is_empty());
    assert!(doc.nodes_to_id.is_empty());
    assert!(doc.controls_to_form.is_empty());
    assert!(doc.scrollbar_activity.is_empty());
    assert!(!doc.changed_nodes.contains(&dead));
}

#[test]
fn sweep_scrubs_derived_links_without_resetting_geometry_or_scroll() {
    let mut doc = BaseDocument::new(DocumentConfig::default());
    let live = element(&mut doc, "div");
    let dead = element(&mut doc, "span");
    let node = &mut doc.nodes[live];
    node.layout_parent.set(Some(dead));
    node.oof_containing_block.set(Some(dead));
    node.flat_parent.set(FlatParent::Node(dead));
    *node.layout_children.get_mut() = Some([dead].into_iter().collect());
    *node.paint_children.get_mut() = Some([dead].into_iter().collect());
    node.composed_children = Some([dead].into_iter().collect());
    node.hoisted_children.get_mut().push(dead);
    *node.scroll_offset_mut() = crate::Point { x: 7.0, y: 9.0 };
    let layout = *node.final_layout();
    doc.set_viewport_scroll(crate::Point { x: 3.0, y: 4.0 });
    let generation = doc.nodes.geometry_generation();
    assert_eq!(doc.reclaim_detached_nodes(&[live]), vec![dead]);
    let node = &doc.nodes[live];
    assert_eq!(node.layout_parent.get(), None);
    assert_eq!(node.oof_containing_block.get(), None);
    assert_eq!(node.flat_parent.get(), FlatParent::Dom);
    assert!(node.layout_children.borrow().as_ref().unwrap().is_empty());
    assert!(node.paint_children.borrow().as_ref().unwrap().is_empty());
    assert!(node.composed_children.as_ref().unwrap().is_empty());
    assert!(node.hoisted_children.borrow().is_empty());
    assert_eq!(*node.final_layout(), layout);
    assert_eq!(*node.scroll_offset(), crate::Point { x: 7.0, y: 9.0 });
    assert_eq!(doc.viewport_scroll(), crate::Point { x: 3.0, y: 4.0 });
    assert!(doc.nodes.geometry_generation() > generation);
}

#[test]
fn unreachable_stylesheet_owner_changes_cssom_generation() {
    let mut doc = BaseDocument::new(DocumentConfig::default());
    let owner = element(&mut doc, "style");
    let sheet = doc.make_stylesheet("div { color: red }", style::stylesheets::Origin::Author);
    doc.add_stylesheet_for_node(sheet, owner);
    let generation = doc.stylesheet_generation();
    assert!(doc.node_has_stylesheet(owner));
    assert_eq!(doc.reclaim_detached_nodes(&[]), vec![owner]);
    assert!(!doc.node_has_stylesheet(owner));
    assert!(doc.stylesheet_generation() > generation);
}

#[test]
fn actual_inline_and_table_caches_are_safe_while_resolution_is_blocked() {
    use crate::node::SpecialElementData;
    let mut doc = BaseDocument::new(DocumentConfig {
        font_ctx: Some(crate::test_font_ctx()),
        ..Default::default()
    });
    let html = element(&mut doc, "html");
    let body = element(&mut doc, "body");
    let inline = element(&mut doc, "div");
    let span = element(&mut doc, "span");
    let text = doc.mutate().create_text_node("old text");
    let table = element(&mut doc, "table");
    let row = element(&mut doc, "tr");
    let cell = element(&mut doc, "td");
    let cell_text = doc.mutate().create_text_node("cell text");
    let root = doc.root_node_id;
    {
        let mut m = doc.mutate();
        m.append_children(span, &[text]);
        m.append_children(inline, &[span]);
        m.append_children(cell, &[cell_text]);
        m.append_children(row, &[cell]);
        m.append_children(table, &[row]);
        m.append_children(body, &[inline, table]);
        m.append_children(html, &[body]);
        m.append_children(root, &[html]);
    }
    doc.resolve(0.0);
    assert!(doc.nodes[inline].flags.is_inline_root());
    assert!(doc.nodes[table].flags.is_table_root());
    assert!(
        doc.nodes[inline]
            .element_data()
            .unwrap()
            .inline_layout_data
            .is_some()
    );
    assert!(matches!(
        doc.nodes[table].element_data().unwrap().special_data,
        SpecialElementData::TableRoot(_)
    ));
    doc.mutate().remove_node(span);
    doc.mutate().remove_node(row);
    doc.snapshot_node(span);
    let opaque = style::dom::TNode::opaque(&&doc.nodes[span]);
    assert!(doc.snapshots.contains_key(&opaque));
    let animation = style::animation::AnimationSetKey::new_for_non_pseudo(opaque);
    doc.animations
        .sets
        .write()
        .entry(animation.clone())
        .or_default();
    doc.pending_critical_resources.insert(123);
    let removed = doc.reclaim_detached_nodes(&[]);
    assert!(removed.contains(&span));
    assert!(removed.contains(&row));
    assert!(!doc.snapshots.contains_key(&opaque));
    assert!(!doc.animations.sets.read().contains_key(&animation));
    doc.resolve(0.0);
    // A blocked resolve does not rebuild these payloads. Render and layout
    // consumers must not be told that the discarded payload still exists.
    assert!(!doc.nodes[inline].flags.is_inline_root());
    assert!(!doc.nodes[table].flags.is_table_root());
    assert!(
        doc.nodes[inline]
            .element_data()
            .unwrap()
            .inline_layout_data
            .is_none()
    );
    assert!(!matches!(
        doc.nodes[table].element_data().unwrap().special_data,
        SpecialElementData::TableRoot(_)
    ));
    doc.pending_critical_resources.clear();
    doc.resolve(0.0);
    assert!(doc.get_client_bounding_rect(inline).is_some());
    assert!(doc.get_client_bounding_rect(table).is_some());
}

#[test]
fn anonymous_layout_borrows_are_scrubbed_before_reconstruction() {
    let mut doc = BaseDocument::new(DocumentConfig {
        font_ctx: Some(crate::test_font_ctx()),
        ..Default::default()
    });
    let html = element(&mut doc, "html");
    let body = element(&mut doc, "body");
    let block = element(&mut doc, "div");
    let span = element(&mut doc, "span");
    let text = doc.mutate().create_text_node("inline content");
    let root = doc.root_node_id;
    {
        let mut m = doc.mutate();
        m.append_children(span, &[text]);
        m.append_children(body, &[span, block]);
        m.append_children(html, &[body]);
        m.append_children(root, &[html]);
    }
    doc.resolve(0.0);
    let anonymous = doc.nodes[body].anonymous_blocks.clone();
    assert!(!anonymous.is_empty());
    assert!(
        anonymous
            .iter()
            .any(|&id| doc.nodes[id].children.contains(&span))
    );
    doc.mutate().remove_node(span);
    doc.node_limit = doc.node_count();
    assert!(doc.reclaim_detached_nodes(&[]).contains(&span));
    for &id in &anonymous {
        assert!(!doc.nodes[id].children.contains(&span));
        assert!(!doc.nodes[id].flags.is_inline_root());
    }
    doc.resolve(0.0);
    assert!(doc.get_client_bounding_rect(block).is_some());
    assert!(doc.node_count() <= doc.node_limit);
}
