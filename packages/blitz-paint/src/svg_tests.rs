//! Scene-level regressions for inline SVG viewports.
use anyrender::{Paint, Scene, recording::RenderCommand};
use blitz_dom::{BaseDocument, DocumentConfig, NodeId, QualName, ns};

fn element(
    doc: &mut BaseDocument,
    parent: NodeId,
    name: QualName,
    attrs: &[(&str, &str)],
) -> NodeId {
    let mut mutation = doc.mutate();
    let id = mutation.create_element(name, vec![]);
    mutation.append_children(parent, &[id]);
    for (attr, value) in attrs {
        mutation.set_attribute(id, QualName::new(None, ns!(), (*attr).into()), *value);
    }
    id
}

/// An inline `<svg>` without dimensions or a `viewBox` lays out at the
/// default 300x150, and paints its user units as CSS pixels from its origin
/// rather than fitting its content into the box (Chrome).
#[test]
fn unsized_inline_svg_paints_at_its_own_scale_in_the_default_box() {
    let mut doc = BaseDocument::new(DocumentConfig::default());
    let root = doc.root_node().id;
    let html_name = |tag: &str| QualName::new(None, ns!(html), tag.into());
    let html = element(&mut doc, root, html_name("html"), &[("style", "margin:0")]);
    let body = element(&mut doc, html, html_name("body"), &[("style", "margin:0")]);
    let svg = element(
        &mut doc,
        body,
        QualName::new(None, ns!(svg), "svg".into()),
        &[("style", "display:block")],
    );
    let rect = [
        ("x", "60"),
        ("y", "30"),
        ("width", "50"),
        ("height", "50"),
        ("fill", "red"),
    ];
    element(
        &mut doc,
        svg,
        QualName::new(None, ns!(svg), "rect".into()),
        &rect,
    );
    doc.resolve(0.0);
    let size = doc.get_node(svg).unwrap().final_layout().size;
    assert_eq!((size.width, size.height), (300.0, 150.0));
    let mut scene = Scene::new();
    super::paint_scene(&mut scene, &mut doc, 1.0, 800, 600, 0, 0);
    let red = Paint::Solid(peniko::Color::from_rgb8(255, 0, 0));
    let fill = scene.commands.iter().find_map(|command| match command {
        RenderCommand::Fill(fill) if fill.brush == red => Some(fill),
        _ => None,
    });
    let fill = fill.expect("the rect paints");
    let placed = fill
        .transform
        .transform_rect_bbox(kurbo::Shape::bounding_box(&fill.shape));
    assert_eq!(
        placed,
        kurbo::Rect::new(60.0, 30.0, 110.0, 80.0),
        "{:?}",
        fill.transform
    );
}
