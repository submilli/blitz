//! Protected inline glyphs and hoisted descendants never reach the scene.
use anyrender::{Scene, recording::RenderCommand};
use blitz_dom::{BaseDocument, DocumentConfig, NodeId, QualName, ns};
fn element(doc: &mut BaseDocument, parent: NodeId, tag: &str, style: &str) -> NodeId {
    let mut m = doc.mutate();
    let id = m.create_element(QualName::new(None, ns!(html), tag.into()), vec![]);
    m.append_children(parent, &[id]);
    m.set_attribute(id, QualName::new(None, ns!(), "style".into()), style);
    id
}
fn glyphs(masked: bool, display: &str) -> usize {
    let mut doc = BaseDocument::new(DocumentConfig::default());
    *doc.font_ctx().lock().unwrap() = blitz_dom::build_single_font_ctx(include_bytes!(
        "../../blitz-dom/assets/test-fonts/LiberationSans-Regular.ttf"
    ));
    let root = doc.root_node().id;
    let html = element(&mut doc, root, "html", "");
    let body = element(&mut doc, html, "body", "");
    let span = element(&mut doc, body, "span", &format!("display:{display}"));
    let child = element(&mut doc, span, "span", "");
    let text = doc.mutate().create_text_node("private-value");
    doc.mutate().append_children(child, &[text]);
    let positioned = element(
        &mut doc,
        span,
        "span",
        "position:absolute;left:40px;top:40px",
    );
    let text = doc.mutate().create_text_node("private-value");
    doc.mutate().append_children(positioned, &[text]);
    if masked {
        assert!(doc.mark_credential_field(span));
    }
    doc.resolve(0.0);
    let mut scene = Scene::new();
    super::paint_scene(&mut scene, &mut doc, 1.0, 800, 600, 0, 0);
    scene
        .commands
        .iter()
        .filter(|c| matches!(c, RenderCommand::GlyphRun(_)))
        .count()
}
#[test]
fn protected_inline_and_contents_subtrees_suppress_all_glyphs() {
    for display in ["inline", "contents", "block"] {
        assert!(glyphs(false, display) > 0, "{display}");
        assert_eq!(glyphs(true, display), 0, "{display}");
    }
}
