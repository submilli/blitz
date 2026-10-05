//! Scene-level regression for transformed and clipped native child documents.
use anyrender::{Paint, Scene, recording::RenderCommand};
use blitz_dom::{BaseDocument, DocumentConfig, NodeId, QualName, ns};
use kurbo::{Affine, Shape};

fn element(doc: &mut BaseDocument, parent: NodeId, tag: &str, css: &str) -> NodeId {
    let mut mutation = doc.mutate();
    let id = mutation.create_element(QualName::new(None, ns!(html), tag.into()), vec![]);
    mutation.append_children(parent, &[id]);
    mutation.set_attribute(id, QualName::new(None, ns!(), "style".into()), css);
    id
}

fn document(css: &str) -> (BaseDocument, NodeId) {
    let mut doc = BaseDocument::new(DocumentConfig::default());
    doc.embedder_loads_iframes = true;
    let root = doc.root_node().id;
    let html = element(&mut doc, root, "html", "margin:0;padding:0");
    let body = element(&mut doc, html, "body", css);
    (doc, body)
}

#[test]
fn child_scene_uses_full_transform_and_content_viewport_clip() {
    let (mut parent, body) = document("margin:0;background:white");
    let host = element(
        &mut parent,
        body,
        "iframe",
        "position:absolute;left:20px;top:30px;width:100px;height:80px;border:5px solid black;padding:3px;transform:scale(2);transform-origin:0 0",
    );
    let (child, _) = document("margin:0;background:red;height:1000px");
    parent.set_sub_document(host, Box::new(child));
    parent.resolve(0.0);
    let mut scene = Scene::new();
    super::paint_scene(&mut scene, &mut parent, 1.0, 800, 600, 0, 0);
    let transform = Affine::translate((36.0, 46.0)) * Affine::scale(2.0);
    assert!(scene.commands.iter().any(|command| matches!(command,
        RenderCommand::Fill(fill) if fill.brush == Paint::Solid(peniko::Color::from_rgb8(255, 0, 0)) && fill.transform == transform)));
    assert!(scene.commands.iter().any(|command| matches!(command,
        RenderCommand::PushClipLayer(clip) if clip.transform == transform && clip.clip.bounding_box() == kurbo::Rect::new(0.0, 0.0, 100.0, 80.0))));
    let pushed = scene
        .commands
        .iter()
        .filter(|command| {
            matches!(
                command,
                RenderCommand::PushClipLayer(_) | RenderCommand::PushLayer(_)
            )
        })
        .count();
    let popped = scene
        .commands
        .iter()
        .filter(|command| matches!(command, RenderCommand::PopLayer))
        .count();
    assert_eq!(pushed, popped);
}
