//! A select paints the content its layout holds: a drop-down its selected
//! option's label and its arrow, a list box its rows and their selection.
use anyrender::{Paint, Scene, recording::RenderCommand};
use blitz_dom::{BaseDocument, DocumentConfig, NodeId, QualName, ns};
use kurbo::{Point, Shape};

fn element(doc: &mut BaseDocument, parent: NodeId, tag: &str, attrs: &[(&str, &str)]) -> NodeId {
    let mut mutation = doc.mutate();
    let id = mutation.create_element(QualName::new(None, ns!(html), tag.into()), vec![]);
    mutation.append_children(parent, &[id]);
    for (name, value) in attrs {
        mutation.set_attribute(id, QualName::new(None, ns!(), (*name).into()), *value);
    }
    id
}

fn option(doc: &mut BaseDocument, select: NodeId, label: &str, attrs: &[(&str, &str)]) {
    let option = element(doc, select, "option", attrs);
    let mut mutation = doc.mutate();
    let text = mutation.create_text_node(label);
    mutation.append_children(option, &[text]);
}

/// A document whose body holds one select at (10, 20), the options' labels
/// and attributes given: its scene and the select's border box width.
fn painted(select_attrs: &[(&str, &str)], options: &[(&str, &[(&str, &str)])]) -> (Scene, f64) {
    let mut doc = BaseDocument::new(DocumentConfig::default());
    *doc.font_ctx().lock().unwrap() = blitz_dom::build_single_font_ctx(include_bytes!(
        "../../blitz-dom/assets/test-fonts/LiberationSans-Regular.ttf"
    ));
    let root = doc.root_node().id;
    let html = element(&mut doc, root, "html", &[]);
    let body = element(&mut doc, html, "body", &[("style", "margin:0")]);
    let mut attrs = vec![(
        "style",
        "position:absolute;left:10px;top:20px;font-family:Arial",
    )];
    attrs.extend_from_slice(select_attrs);
    let select = element(&mut doc, body, "select", &attrs);
    for (label, attrs) in options {
        option(&mut doc, select, label, attrs);
    }
    doc.resolve(0.0);
    let mut scene = Scene::new();
    super::paint_scene(&mut scene, &mut doc, 1.0, 800, 600, 0, 0);
    let width = doc
        .get_client_bounding_rect(select)
        .expect("the select is laid out")
        .width;
    (scene, width)
}

/// Each glyph run's glyph count and its first glyph's position.
fn glyph_runs(scene: &Scene) -> Vec<(usize, Point)> {
    scene
        .commands
        .iter()
        .filter_map(|command| match command {
            RenderCommand::GlyphRun(run) => {
                let first = run.glyphs.first()?;
                let origin = run.transform * Point::new(f64::from(first.x), f64::from(first.y));
                Some((run.glyphs.len(), origin))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn a_drop_down_paints_its_selected_label_and_its_arrow() {
    // Chrome 154: the label starts 4px into the content box, and its
    // baseline is a pixel below the line's top plus the font's ascent (12px
    // at 13.333333px).
    let (scene, width) = painted(&[], &[("one", &[]), ("second", &[("selected", "")])]);
    let runs = glyph_runs(&scene);
    assert_eq!(runs.len(), 1, "{runs:?}");
    let (glyphs, origin) = runs[0];
    assert_eq!(glyphs, "second".len());
    assert!((origin.x - 15.0).abs() < 1.0, "{origin:?}");
    assert!((origin.y - 34.0).abs() < 0.01, "{origin:?}");
    // The chevron: the one stroke smaller than the button part (the
    // border is stroked too).
    let arrow = scene.commands.iter().find_map(|command| match command {
        RenderCommand::Stroke(stroke) => {
            let bounds = stroke
                .transform
                .transform_rect_bbox(stroke.shape.bounding_box());
            (bounds.width() < 16.0).then_some(bounds)
        }
        _ => None,
    });
    // In the 16px button part at the end of the content box.
    let content_right = 10.0 + width - 1.0;
    let arrow = arrow.expect("the arrow is stroked");
    assert!(
        arrow.x1 < content_right && arrow.x0 > content_right - 16.0,
        "{arrow:?}"
    );
}

#[test]
fn a_drop_down_without_a_selection_paints_no_label() {
    let (scene, _) = painted(&[], &[]);
    assert!(glyph_runs(&scene).is_empty());
    let (scene, _) = painted(&[], &[("", &[])]);
    assert!(glyph_runs(&scene).is_empty());
}

#[test]
fn a_list_box_paints_its_rows_and_highlights_the_selected_ones() {
    let (scene, _) = painted(
        &[("size", "2"), ("multiple", "")],
        &[
            ("ab", &[("selected", "")]),
            ("cde", &[]),
            ("hidden past the box", &[]),
        ],
    );
    // Rows are 17px tall (1.2em and a pixel of padding), their labels 2px
    // from the start edge; the third row is below the box.
    let runs = glyph_runs(&scene);
    assert_eq!(runs.len(), 2, "{runs:?}");
    assert_eq!(runs[0].0, 2);
    assert_eq!(runs[1].0, 3);
    assert!((runs[0].1.x - 13.0).abs() < 1.0, "{runs:?}");
    assert!((runs[1].1.y - runs[0].1.y - 17.0).abs() < 0.01, "{runs:?}");
    let selected: Vec<_> = scene
        .commands
        .iter()
        .filter_map(|command| match command {
            RenderCommand::Fill(fill)
                if fill.brush == Paint::Solid(super::render::select::SELECTED_ROW) =>
            {
                Some(
                    fill.transform
                        .transform_rect_bbox(fill.shape.bounding_box()),
                )
            }
            _ => None,
        })
        .collect();
    assert_eq!(selected.len(), 1, "{selected:?}");
    assert_eq!(selected[0].y0, 21.0);
    assert_eq!(selected[0].height(), 17.0);
}
