//! Synchronous style reads flush loaded rules without delivering pending resources.
use crate::net::{ResourceHandler, StylesheetHandler};
use crate::{BaseDocument, DocumentConfig, NodeId, QualName, ns};
use blitz_traits::net::{Bytes, NetHandler};

fn element(doc: &mut BaseDocument, parent: NodeId, tag: &str) -> NodeId {
    let mut mutation = doc.mutate();
    let node = mutation.create_element(QualName::new(None, ns!(html), tag.into()), vec![]);
    mutation.append_children(parent, &[node]);
    node
}

#[test]
fn synchronous_style_flush_updates_loaded_rules_without_delivering_pending_sheet() {
    let mut doc = BaseDocument::new(DocumentConfig::default());
    let root = doc.root_node().id;
    let html = element(&mut doc, root, "html");
    let head = element(&mut doc, html, "head");
    let style = element(&mut doc, head, "style");
    doc.mutate().set_text_content(style, "div{width:10px}");
    let body = element(&mut doc, html, "body");
    let target = element(&mut doc, body, "div");
    doc.resolve(0.0);
    assert_eq!(doc.resolved_style_value(target, "width"), "10px");

    let link = element(&mut doc, head, "link");
    let url = url::Url::parse("https://example.test/pending.css").unwrap();
    let handler = ResourceHandler::new(
        doc.tx.clone(),
        doc.id(),
        Some(doc.resource_pin(link)),
        doc.shell_provider.clone(),
        StylesheetHandler {
            source_url: url.clone(),
            guard: doc.guard.clone(),
            net_provider: doc.net_provider.clone(),
            abort_signal: None,
        },
    );
    doc.pending_critical_resources.insert(handler.request_id());
    Box::new(handler).bytes(url.to_string(), Bytes::from_static(b"div{width:30px}"));
    doc.mutate().set_text_content(style, "div{width:20px}");

    doc.flush_style_and_layout(0.0);
    assert_eq!(doc.resolved_style_value(target, "width"), "20px");
    assert!(doc.has_pending_critical_resources());
    assert!(doc.retain_stylesheet(link).is_none());

    doc.resolve(0.0);
    assert!(!doc.has_pending_critical_resources());
    assert_eq!(doc.resolved_style_value(target, "width"), "30px");
}

#[test]
fn embedded_document_viewport_excludes_host_border_and_padding() {
    let mut parent = BaseDocument::new(DocumentConfig::default());
    parent.embedder_loads_iframes = true;
    let root = parent.root_node().id;
    let html = element(&mut parent, root, "html");
    let body = element(&mut parent, html, "body");
    let frame = element(&mut parent, body, "iframe");
    parent.mutate().set_attribute(
        frame,
        QualName::new(None, ns!(), "style".into()),
        "width:100px;height:80px;border:5px solid;padding:3px",
    );
    parent.set_sub_document(
        frame,
        Box::new(BaseDocument::new(DocumentConfig::default())),
    );
    parent.resolve(0.0);
    assert_eq!(
        parent.subdoc(frame).unwrap().inner().viewport().window_size,
        (100, 80)
    );
    let mut viewport = parent.viewport().clone();
    viewport.hidpi_scale = 2.0;
    parent.set_viewport(viewport);
    parent.resolve(0.0);
    assert_eq!(
        parent.subdoc(frame).unwrap().inner().viewport().window_size,
        (200, 160)
    );
}

#[test]
fn physical_pointer_input_focuses_buttons_and_keeps_disabled_controls_unfocused() {
    use crate::{EventDriver, NoopEventHandler};
    use blitz_traits::events::UiEvent;
    use keyboard_types::Modifiers;
    let mut doc = BaseDocument::new(DocumentConfig::default());
    let root = doc.root_node().id;
    let html = element(&mut doc, root, "html");
    let body = element(&mut doc, html, "body");
    let button = element(&mut doc, body, "button");
    doc.mutate().set_text_content(button, "Focus me");
    doc.resolve(0.0);
    let pointer = doc
        .get_node(button)
        .unwrap()
        .synthetic_click_event_data(Modifiers::empty());
    EventDriver::new(&mut doc, NoopEventHandler).handle_ui_event(UiEvent::PointerDown(pointer));
    assert_eq!(doc.get_focussed_node_id(), Some(button));
    let span = element(&mut doc, button, "span");
    doc.mutate().set_attribute(
        span,
        QualName::new(None, ns!(), "style".into()),
        "display:block;width:80px;height:30px",
    );
    doc.clear_focus();
    doc.resolve(0.0);
    assert!(doc.begin_focus_transition(Some(span)).is_none());
    let pointer = doc
        .get_node(span)
        .unwrap()
        .synthetic_click_event_data(Modifiers::empty());
    EventDriver::new(&mut doc, NoopEventHandler).handle_ui_event(UiEvent::PointerDown(pointer));
    assert_eq!(doc.get_focussed_node_id(), Some(button));
    let disabled = element(&mut doc, body, "button");
    doc.mutate().set_text_content(disabled, "Disabled");
    doc.mutate()
        .set_attribute(disabled, QualName::new(None, ns!(), "disabled".into()), "");
    doc.resolve(0.0);
    let pointer = doc
        .get_node(disabled)
        .unwrap()
        .synthetic_click_event_data(Modifiers::empty());
    EventDriver::new(&mut doc, NoopEventHandler).handle_ui_event(UiEvent::PointerDown(pointer));
    assert_ne!(doc.get_focussed_node_id(), Some(disabled));
}

/// A chain of `depth` `div`s under `parent`; returns them, outermost first.
fn div_chain(doc: &mut BaseDocument, parent: NodeId, depth: usize) -> Vec<NodeId> {
    let mut divs = Vec::with_capacity(depth);
    let mut node = parent;
    for _ in 0..depth {
        node = element(doc, node, "div");
        divs.push(node);
    }
    divs
}

#[test]
fn a_frame_chain_shares_one_box_depth_limit() {
    // A subdocument is painted within its parent's paint recursion, so its
    // boxes continue the depth of its iframe's.
    std::thread::Builder::new()
        .stack_size(64 << 20)
        .spawn(|| {
            let mut parent = BaseDocument::new(DocumentConfig::default());
            parent.embedder_loads_iframes = true;
            let root = parent.root_node().id;
            let html = element(&mut parent, root, "html");
            let body = element(&mut parent, html, "body");
            let divs = div_chain(&mut parent, body, 1_000);
            let frame = element(&mut parent, *divs.last().unwrap(), "iframe");

            let mut child = BaseDocument::new(DocumentConfig::default());
            let child_root = child.root_node().id;
            let child_html = element(&mut child, child_root, "html");
            let child_body = element(&mut child, child_html, "body");
            let child_divs = div_chain(&mut child, child_body, 100);
            parent.set_sub_document(frame, Box::new(child));
            parent.resolve(0.0);

            let child = parent.subdoc(frame).unwrap().inner();
            // html, body and the divs put the iframe's box 1,003 deep.
            assert_eq!(child.box_depth_base, 1_004);
            let has_box = |id: NodeId| child.get_node(id).unwrap().layout_parent.get().is_some();
            assert!(has_box(child_divs[10]));
            assert!(!has_box(child_divs[50]));
        })
        .unwrap()
        .join()
        .unwrap();
}
