//! Containing blocks determine geometry; flat ancestry determines paint order.
use anyrender::{Paint, Scene, recording::RenderCommand};
use blitz_dom::{BaseDocument, DocumentConfig, NodeId, QualName, ns};

fn element(doc: &mut BaseDocument, parent: NodeId, tag: &str, css: &str) -> NodeId {
    let mut mutation = doc.mutate();
    let id = mutation.create_element(QualName::new(None, ns!(html), tag.into()), vec![]);
    mutation.append_children(parent, &[id]);
    mutation.set_attribute(id, QualName::new(None, ns!(), "style".into()), css);
    id
}

#[test]
fn fixed_background_precedes_later_positioned_content() {
    for clip in ["visible", "hidden"] {
        for z in ["auto", "0"] {
            for fixed_z in ["auto", "0", "-1", "1"] {
                let mut doc = BaseDocument::new(DocumentConfig::default());
                let root = doc.root_node().id;
                let html = element(&mut doc, root, "html", "margin:0;height:100%");
                let body = element(
                    &mut doc,
                    html,
                    "body",
                    &format!("margin:0;height:100%;overflow:{clip};background:white"),
                );
                let app = element(
                    &mut doc,
                    body,
                    "div",
                    &format!("position:relative;z-index:{z};height:100%"),
                );
                let wrapper = element(&mut doc, app, "div", "");
                element(
                    &mut doc,
                    wrapper,
                    "div",
                    &format!("position:fixed;z-index:{fixed_z};inset:0;background:blue"),
                );
                let content = element(
                    &mut doc,
                    app,
                    "div",
                    "position:relative;height:100%;background:red",
                );
                element(
                    &mut doc,
                    content,
                    "div",
                    "position:relative;width:10px;height:10px;background:green",
                );
                doc.resolve(0.0);
                let mut scene = Scene::new();
                super::paint_scene(&mut scene, &mut doc, 1.0, 800, 600, 0, 0);
                let fills: Vec<_> = scene
                    .commands
                    .iter()
                    .filter_map(|command| {
                        if let RenderCommand::Fill(fill) = command {
                            Some(&fill.brush)
                        } else {
                            None
                        }
                    })
                    .collect();
                let blue = fills
                    .iter()
                    .position(|brush| **brush == Paint::Solid(peniko::Color::from_rgb8(0, 0, 255)))
                    .unwrap();
                let red = fills
                    .iter()
                    .position(|brush| **brush == Paint::Solid(peniko::Color::from_rgb8(255, 0, 0)))
                    .unwrap();
                let green = fills
                    .iter()
                    .position(|brush| **brush == Paint::Solid(peniko::Color::from_rgb8(0, 128, 0)))
                    .unwrap();
                assert!(red < green, "clip={clip}, z={z}, fixed_z={fixed_z}");
                if fixed_z == "1" {
                    assert!(green < blue, "clip={clip}, z={z}, fixed_z={fixed_z}");
                } else {
                    assert!(blue < red, "clip={clip}, z={z}, fixed_z={fixed_z}");
                }
            }
        }
    }
}

#[test]
fn positioned_flex_items_keep_order_modified_paint_order() {
    for display in ["flex", "grid"] {
        let mut doc = BaseDocument::new(DocumentConfig::default());
        let root = doc.root_node().id;
        let html = element(&mut doc, root, "html", "margin:0");
        let body = element(
            &mut doc,
            html,
            "body",
            &format!("margin:0;display:{display}"),
        );
        element(
            &mut doc,
            body,
            "div",
            "position:relative;order:1;width:10px;height:10px;background:red",
        );
        element(
            &mut doc,
            body,
            "div",
            "position:relative;order:0;width:10px;height:10px;background:blue",
        );
        doc.resolve(0.0);
        let mut scene = Scene::new();
        super::paint_scene(&mut scene, &mut doc, 1.0, 800, 600, 0, 0);
        let colors: Vec<_> = scene
            .commands
            .iter()
            .filter_map(|command| {
                if let RenderCommand::Fill(fill) = command {
                    Some(fill.brush.clone())
                } else {
                    None
                }
            })
            .collect();
        let blue = colors
            .iter()
            .position(|brush| *brush == Paint::Solid(peniko::Color::from_rgb8(0, 0, 255)))
            .unwrap();
        let red = colors
            .iter()
            .position(|brush| *brush == Paint::Solid(peniko::Color::from_rgb8(255, 0, 0)))
            .unwrap();
        assert!(blue < red, "display={display}");
    }
}

#[test]
fn context_background_precedes_negative_z_children() {
    let mut doc = BaseDocument::new(DocumentConfig::default());
    let root = doc.root_node().id;
    let html = element(&mut doc, root, "html", "margin:0");
    let body = element(&mut doc, html, "body", "margin:0");
    let context = element(
        &mut doc,
        body,
        "div",
        "position:relative;z-index:0;width:100px;height:100px;background:red",
    );
    element(
        &mut doc,
        context,
        "div",
        "position:absolute;z-index:-1;width:100px;height:100px;background:blue",
    );
    doc.resolve(0.0);
    let mut scene = Scene::new();
    super::paint_scene(&mut scene, &mut doc, 1.0, 800, 600, 0, 0);
    let colors: Vec<_> = scene
        .commands
        .iter()
        .filter_map(|command| {
            if let RenderCommand::Fill(fill) = command {
                Some(fill.brush.clone())
            } else {
                None
            }
        })
        .collect();
    let blue = colors
        .iter()
        .position(|brush| *brush == Paint::Solid(peniko::Color::from_rgb8(0, 0, 255)))
        .unwrap();
    let red = colors
        .iter()
        .position(|brush| *brush == Paint::Solid(peniko::Color::from_rgb8(255, 0, 0)))
        .unwrap();
    assert!(red < blue);
}

#[test]
fn out_of_flow_flex_and_grid_children_ignore_order() {
    for display in ["flex", "grid"] {
        for position in ["absolute", "fixed"] {
            let mut doc = BaseDocument::new(DocumentConfig::default());
            let root = doc.root_node().id;
            let html = element(&mut doc, root, "html", "margin:0");
            let body = element(
                &mut doc,
                html,
                "body",
                &format!("margin:0;position:relative;display:{display}"),
            );
            element(
                &mut doc,
                body,
                "div",
                &format!(
                    "position:{position};order:1;inset:0;width:10px;height:10px;background:red"
                ),
            );
            element(
                &mut doc,
                body,
                "div",
                &format!(
                    "position:{position};order:0;inset:0;width:10px;height:10px;background:blue"
                ),
            );
            doc.resolve(0.0);
            let mut scene = Scene::new();
            super::paint_scene(&mut scene, &mut doc, 1.0, 800, 600, 0, 0);
            let colors: Vec<_> = scene
                .commands
                .iter()
                .filter_map(|command| {
                    if let RenderCommand::Fill(fill) = command {
                        Some(fill.brush.clone())
                    } else {
                        None
                    }
                })
                .collect();
            let blue = colors
                .iter()
                .position(|brush| *brush == Paint::Solid(peniko::Color::from_rgb8(0, 0, 255)))
                .unwrap();
            let red = colors
                .iter()
                .position(|brush| *brush == Paint::Solid(peniko::Color::from_rgb8(255, 0, 0)))
                .unwrap();
            assert!(red < blue, "display={display}, position={position}");
        }
    }
}

#[test]
fn positioned_flex_and_grid_pseudos_honor_order() {
    for display in ["flex", "grid"] {
        for pseudo in ["before", "after"] {
            let mut doc = BaseDocument::new(DocumentConfig::default());
            let root = doc.root_node().id;
            let html = element(&mut doc, root, "html", "margin:0");
            let sheet = element(&mut doc, html, "style", "");
            let css = format!(
                "body::{pseudo}{{content:'';position:relative;order:1;width:10px;height:10px;background:red}}"
            );
            {
                let mut mutation = doc.mutate();
                let text = mutation.create_text_node(&css);
                mutation.append_children(sheet, &[text]);
            }
            let body = element(
                &mut doc,
                html,
                "body",
                &format!("margin:0;display:{display}"),
            );
            element(
                &mut doc,
                body,
                "div",
                "position:relative;order:0;width:10px;height:10px;background:blue",
            );
            doc.resolve(0.0);
            let mut scene = Scene::new();
            super::paint_scene(&mut scene, &mut doc, 1.0, 800, 600, 0, 0);
            let colors: Vec<_> = scene
                .commands
                .iter()
                .filter_map(|command| {
                    if let RenderCommand::Fill(fill) = command {
                        Some(fill.brush.clone())
                    } else {
                        None
                    }
                })
                .collect();
            let blue = colors
                .iter()
                .position(|brush| *brush == Paint::Solid(peniko::Color::from_rgb8(0, 0, 255)))
                .unwrap();
            let red = colors
                .iter()
                .position(|brush| *brush == Paint::Solid(peniko::Color::from_rgb8(255, 0, 0)))
                .unwrap();
            assert!(blue < red, "display={display}, pseudo={pseudo}");
        }
    }
}

#[test]
fn contents_wrapped_flex_and_grid_items_honor_order() {
    for display in ["flex", "grid"] {
        let mut doc = BaseDocument::new(DocumentConfig::default());
        let root = doc.root_node().id;
        let html = element(&mut doc, root, "html", "margin:0");
        let body = element(
            &mut doc,
            html,
            "body",
            &format!("margin:0;display:{display}"),
        );
        let contents = element(&mut doc, body, "div", "display:contents");
        let contents = element(&mut doc, contents, "div", "display:contents");
        element(
            &mut doc,
            contents,
            "div",
            "position:relative;order:1;width:10px;height:10px;background:red",
        );
        element(
            &mut doc,
            body,
            "div",
            "position:relative;order:0;width:10px;height:10px;background:blue",
        );
        doc.resolve(0.0);
        let mut scene = Scene::new();
        super::paint_scene(&mut scene, &mut doc, 1.0, 800, 600, 0, 0);
        let colors: Vec<_> = scene
            .commands
            .iter()
            .filter_map(|command| {
                if let RenderCommand::Fill(fill) = command {
                    Some(fill.brush.clone())
                } else {
                    None
                }
            })
            .collect();
        let blue = colors
            .iter()
            .position(|brush| *brush == Paint::Solid(peniko::Color::from_rgb8(0, 0, 255)))
            .unwrap();
        let red = colors
            .iter()
            .position(|brush| *brush == Paint::Solid(peniko::Color::from_rgb8(255, 0, 0)))
            .unwrap();
        assert!(blue < red, "display={display}");
    }
}
