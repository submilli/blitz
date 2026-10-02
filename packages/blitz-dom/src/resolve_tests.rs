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
