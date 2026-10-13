//! A button paints as Chrome's themed button unless the page styles its
//! border or background, when it paints them as any box does.
use anyrender::{Paint, Scene, recording::RenderCommand};
use blitz_dom::{BaseDocument, DocumentConfig, NodeId, QualName, ns};
use kurbo::{Rect, Shape};

use super::render::button::{BORDER, FILL};

/// Paint a document whose body holds one 40x20 button at (10, 20), with
/// the given style added to its own.
fn painted(style: &str) -> Scene {
    let mut doc = BaseDocument::new(DocumentConfig::default());
    *doc.font_ctx().lock().unwrap() = blitz_dom::build_single_font_ctx(include_bytes!(
        "../../blitz-dom/assets/test-fonts/LiberationSans-Regular.ttf"
    ));
    let root = doc.root_node().id;
    let html = element(&mut doc, root, "html", "");
    let body = element(&mut doc, html, "body", "margin:0");
    let style = format!("position:absolute;left:10px;top:20px;width:40px;height:20px;{style}");
    let button = element(&mut doc, body, "button", &style);
    let mut mutation = doc.mutate();
    let text = mutation.create_text_node("x");
    mutation.append_children(button, &[text]);
    drop(mutation);
    doc.resolve(0.0);
    let mut scene = Scene::new();
    super::paint_scene(&mut scene, &mut doc, 1.0, 800, 600, 0, 0);
    scene
}

fn element(doc: &mut BaseDocument, parent: NodeId, tag: &str, style: &str) -> NodeId {
    let mut mutation = doc.mutate();
    let id = mutation.create_element(QualName::new(None, ns!(html), tag.into()), vec![]);
    mutation.append_children(parent, &[id]);
    if !style.is_empty() {
        mutation.set_attribute(id, QualName::new(None, ns!(), "style".into()), style);
    }
    id
}

/// The bounds of every fill and stroke in a solid color.
fn painted_in(scene: &Scene, color: peniko::Color) -> Vec<Rect> {
    scene
        .commands
        .iter()
        .filter_map(|command| match command {
            RenderCommand::Fill(fill) if fill.brush == Paint::Solid(color) => Some(
                fill.transform
                    .transform_rect_bbox(fill.shape.bounding_box()),
            ),
            RenderCommand::Stroke(stroke) if stroke.brush == Paint::Solid(color) => Some(
                stroke
                    .transform
                    .transform_rect_bbox(stroke.shape.bounding_box()),
            ),
            _ => None,
        })
        .collect()
}

#[test]
fn a_button_paints_chrome_theme_in_its_border_box() {
    let scene = painted("");
    let border_box = Rect::new(10.0, 20.0, 50.0, 40.0);
    assert_eq!(painted_in(&scene, FILL), [border_box]);
    // The 1px border is stroked along its middle, half a pixel in.
    assert_eq!(painted_in(&scene, BORDER), [border_box.inset(-0.5)]);
    // Nothing of the computed `2px outset black` border.
    let black = peniko::Color::from_rgba8(0, 0, 0, 255);
    assert!(
        scene.commands.iter().all(|command| !matches!(
            command,
            RenderCommand::Fill(fill) if fill.brush == Paint::Solid(black)
        )),
        "the outset border is painted"
    );
}

#[test]
fn a_styled_button_paints_its_own_border_and_background() {
    let red = peniko::Color::from_rgba8(255, 0, 0, 255);
    for style in ["border:3px solid red", "background:red"] {
        let scene = painted(style);
        assert!(painted_in(&scene, BORDER).is_empty(), "{style}");
        assert!(!painted_in(&scene, red).is_empty(), "{style}");
    }
}
