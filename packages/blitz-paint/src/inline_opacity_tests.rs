use anyrender::{PaintScene, Scene, recording::RenderCommand};
use blitz_dom::{BaseDocument, DocumentConfig, NodeId, QualName, ns};
use kurbo::{Affine, Rect};
use peniko::{Color, Fill};

fn element(doc: &mut BaseDocument, parent: NodeId, tag: &str, style: &str) -> NodeId {
    let mut mutation = doc.mutate();
    let id = mutation.create_element(QualName::new(None, ns!(html), tag.into()), vec![]);
    mutation.append_children(parent, &[id]);
    mutation.set_attribute(id, QualName::new(None, ns!(), "style".into()), style);
    id
}

fn text(doc: &mut BaseDocument, parent: NodeId, value: &str) {
    let mut mutation = doc.mutate();
    let text = mutation.create_text_node(value);
    mutation.append_children(parent, &[text]);
}

fn document(opacity: &str) -> (BaseDocument, NodeId, NodeId) {
    let mut doc = BaseDocument::new(DocumentConfig::default());
    *doc.font_ctx().lock().unwrap() = blitz_dom::build_single_font_ctx(include_bytes!(
        "../../blitz-dom/assets/test-fonts/LiberationSans-Regular.ttf"
    ));
    let root = doc.root_node().id;
    let html = element(&mut doc, root, "html", "");
    let body = element(
        &mut doc,
        html,
        "body",
        "margin:0;width:160px;font:32px Arial",
    );
    let outer = element(&mut doc, body, "span", opacity);
    let inner = element(
        &mut doc,
        outer,
        "span",
        "background:red;color:black;text-decoration:underline",
    );
    text(&mut doc, inner, "MMMM");
    (doc, outer, inner)
}

fn pixels(doc: &mut BaseDocument) -> Vec<u8> {
    doc.resolve(0.0);
    anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |scene| {
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                Color::WHITE,
                None,
                &Rect::new(0.0, 0.0, 160.0, 80.0),
            );
            super::paint_scene(scene, doc, 1.0, 160, 80, 0, 0);
        },
        160,
        80,
    )
}

#[test]
fn zero_opacity_hides_inline_ink_without_removing_layout() {
    let (mut doc, outer, _) = document("opacity:0");
    let hidden = pixels(&mut doc);
    assert!(
        hidden
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| *p == [255, 255, 255, 255])
    );
    let hidden_width = doc.get_client_bounding_rect(outer).unwrap().width;
    doc.mutate().set_attribute(
        outer,
        QualName::new(None, ns!(), "style".into()),
        "opacity:1",
    );
    let visible = pixels(&mut doc);
    assert!(visible.as_chunks::<4>().0.iter().any(|p| p[1] == 0));
    assert_eq!(
        hidden_width,
        doc.get_client_bounding_rect(outer).unwrap().width
    );
    assert!(hidden_width > 60.0);
}

#[test]
fn opacity_composites_background_glyphs_and_decoration_together() {
    let (mut doc, _, _) = document("opacity:0.5");
    let rgba = pixels(&mut doc);
    // Painting each primitive with alpha .5 would make text/background overlap
    // darker than this. Group composition keeps every channel >= 127 on white.
    assert!(
        rgba.as_chunks::<4>()
            .0
            .iter()
            .all(|p| p[0] >= 127 && p[1] >= 127 && p[2] >= 127)
    );
    assert!(rgba.as_chunks::<4>().0.iter().any(|p| p[1] <= 128));
}

#[test]
fn nested_inline_opacity_is_applied_once_per_element() {
    let (mut doc, _, inner) = document("opacity:0.5");
    doc.mutate().set_attribute(
        inner,
        QualName::new(None, ns!(), "style".into()),
        "opacity:0.5;background:red",
    );
    let rgba = pixels(&mut doc);
    assert!(
        rgba.as_chunks::<4>()
            .0
            .iter()
            .all(|p| p[0] >= 190 && p[1] >= 190 && p[2] >= 190)
    );
    assert!(rgba.as_chunks::<4>().0.iter().any(|p| p[1] <= 192));
}

#[test]
fn atomic_descendants_share_the_non_atomic_inline_group() {
    let (mut doc, outer, _) = document("opacity:0.5");
    element(
        &mut doc,
        outer,
        "span",
        "display:inline-block;width:30px;height:30px;background:blue;margin-left:-30px",
    );
    let rgba = pixels(&mut doc);
    assert!(
        rgba.as_chunks::<4>()
            .0
            .iter()
            .all(|p| p[0] >= 127 && p[1] >= 127 && p[2] >= 127)
    );
    assert!(
        rgba.as_chunks::<4>()
            .0
            .iter()
            .any(|p| p[2] >= 254 && p[0] <= 128)
    );
}

#[test]
fn one_inline_group_spans_all_line_and_font_runs() {
    let (mut doc, outer, _) = document("opacity:0.5");
    element(&mut doc, outer, "br", "");
    text(&mut doc, outer, "second line");
    doc.resolve(0.0);
    let mut scene = Scene::new();
    super::paint_scene(&mut scene, &mut doc, 1.0, 160, 80, 0, 0);
    let alphas: Vec<_> = scene
        .commands
        .iter()
        .filter_map(|c| match c {
            RenderCommand::PushLayer(layer) => Some(layer.alpha),
            _ => None,
        })
        .collect();
    assert_eq!(alphas, [0.5]);
    assert!(
        scene
            .commands
            .iter()
            .filter(|c| matches!(c, RenderCommand::GlyphRun(_)))
            .count()
            >= 2
    );
}

#[test]
fn an_opacity_context_paints_above_overlapping_ordinary_inline_content() {
    let (mut doc, outer, _) = document("opacity:.5;background:red");
    let body = doc.get_node(outer).unwrap().parent.unwrap();
    let sibling = element(&mut doc, body, "span", "color:blue;margin-left:-60px");
    text(&mut doc, sibling, "MMMM");
    let rgba = pixels(&mut doc);
    // The translucent red context paints above blue glyphs, so a blue-only
    // pixel below its opaque red background cannot survive unchanged.
    assert!(
        rgba.as_chunks::<4>()
            .0
            .iter()
            .enumerate()
            .filter(|(i, _)| i % 160 < 60)
            .all(|(_, p)| *p != [0, 0, 255, 255])
    );
}

#[test]
fn positioned_descendants_cannot_escape_zero_inline_opacity() {
    let (mut doc, outer, _) = document("opacity:0");
    element(
        &mut doc,
        outer,
        "span",
        "position:absolute;z-index:1;left:10px;top:10px;width:30px;height:30px;background:blue",
    );
    assert!(
        pixels(&mut doc)
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| *p == [255, 255, 255, 255])
    );
}

#[test]
fn positioned_descendants_share_fractional_inline_opacity_as_one_group() {
    let (mut doc, outer, _) = document("opacity:.5");
    for (z, color, left) in [(1, "red", 10), (2, "blue", 20)] {
        element(
            &mut doc,
            outer,
            "span",
            &format!(
                "position:absolute;z-index:{z};left:{left}px;top:10px;width:30px;height:30px;background:{color}"
            ),
        );
    }
    let rgba = pixels(&mut doc);
    assert!(
        rgba.as_chunks::<4>()
            .0
            .iter()
            .all(|p| p[0] >= 127 && p[1] >= 127 && p[2] >= 127)
    );
    assert!(
        rgba.as_chunks::<4>()
            .0
            .iter()
            .any(|p| p[0] <= 128 && p[2] == 255)
    );
}

#[test]
fn generated_positioned_content_cannot_escape_zero_inline_opacity() {
    let (mut doc, outer, _) = document("opacity:0");
    doc.mutate()
        .set_attribute(outer, QualName::new(None, ns!(), "id".into()), "owner");
    let root = doc.root_node().id;
    let html = doc.get_node(root).unwrap().children[0];
    let sheet = element(&mut doc, html, "style", "");
    text(
        &mut doc,
        sheet,
        "#owner::before{content:'M';position:absolute;z-index:1;left:10px;top:10px;width:30px;height:30px;background:blue}",
    );
    let rgba = pixels(&mut doc);
    assert!(doc.get_node(outer).unwrap().before().is_some());
    assert!(
        rgba.as_chunks::<4>()
            .0
            .iter()
            .all(|p| *p == [255, 255, 255, 255])
    );
}

#[test]
fn positive_z_siblings_paint_above_inline_opacity_contexts() {
    let (mut doc, outer, _) = document("opacity:.5;background:red");
    let body = doc.get_node(outer).unwrap().parent.unwrap();
    doc.mutate().set_attribute(
        body,
        QualName::new(None, ns!(), "style".into()),
        "margin:0;width:160px;font:32px Arial;opacity:.8",
    );
    element(
        &mut doc,
        body,
        "span",
        "position:absolute;z-index:1;left:10px;top:10px;width:30px;height:30px;background:blue",
    );
    let rgba = pixels(&mut doc);
    let pixel = &rgba[(20 * 160 + 20) * 4..][..4];
    assert!(
        pixel[0] <= 52 && pixel[1] <= 52 && pixel[2] == 255,
        "{pixel:?}"
    );
}

#[test]
fn later_positioned_auto_siblings_paint_above_inline_opacity_contexts() {
    let (mut doc, outer, _) = document("opacity:.5;background:red");
    let body = doc.get_node(outer).unwrap().parent.unwrap();
    doc.mutate().set_attribute(
        body,
        QualName::new(None, ns!(), "style".into()),
        "margin:0;width:160px;font:32px Arial;opacity:.8",
    );
    element(
        &mut doc,
        body,
        "span",
        "position:relative;display:inline-block;width:50px;height:32px;background:blue;margin-left:-70px",
    );
    let rgba = pixels(&mut doc);
    let pixel = &rgba[(20 * 160 + 50) * 4..][..4];
    assert!(
        pixel[0] <= 52 && pixel[1] <= 52 && pixel[2] == 255,
        "{pixel:?}"
    );
}

#[test]
fn negative_z_descendant_stays_below_inline_owner_ink() {
    let (mut doc, outer, inner) = document("opacity:.5;background:red;color:black");
    // Keep the positioned child in the same laid-out stacking context so this
    // independently checks ordering within the group, not cross-root capture.
    let body = doc.get_node(outer).unwrap().parent.unwrap();
    doc.mutate().set_attribute(
        body,
        QualName::new(None, ns!(), "style".into()),
        "margin:0;width:160px;font:32px Arial;opacity:.8;position:relative",
    );
    doc.mutate().remove_node(inner);
    text(&mut doc, outer, "MMMM");
    element(
        &mut doc,
        outer,
        "span",
        "position:absolute;z-index:-1;left:0;top:0;width:160px;height:40px;background:blue",
    );
    let rgba = pixels(&mut doc);
    let row = &rgba[15 * 160 * 4..16 * 160 * 4];
    // Chrome keeps owner background and text above negative-z descendants.
    // The blue child remains visible beyond the inline owner's background.
    assert!(row.as_chunks::<4>().0.iter().any(|p| p[0] >= 152
        && p[0] <= 154
        && p[1] >= 152
        && p[1] <= 154
        && p[2] == 255));
    assert!(row.as_chunks::<4>().0.iter().any(|p| p[0] >= 152
        && p[0] <= 154
        && p[1] >= 152
        && p[1] <= 154
        && p[2] >= 152
        && p[2] <= 154));
    assert!(
        row.as_chunks::<4>()
            .0
            .iter()
            .any(|p| p[0] == 255 && p[1] < 200 && p[2] < 200)
    );
}

#[test]
fn positioned_auto_ancestor_does_not_trap_descendant_stacking_contexts() {
    let (mut doc, outer, _) = document("opacity:.5");
    let body = doc.get_node(outer).unwrap().parent.unwrap();
    let ancestor = element(
        &mut doc,
        body,
        "div",
        "position:absolute;left:80px;top:0;width:60px;height:60px;background:green",
    );
    element(
        &mut doc,
        ancestor,
        "div",
        "position:absolute;z-index:10;left:0;top:0;width:60px;height:60px;background:red",
    );
    element(
        &mut doc,
        body,
        "div",
        "position:absolute;z-index:5;left:80px;top:0;width:60px;height:60px;background:blue",
    );
    let rgba = pixels(&mut doc);
    assert_eq!(&rgba[(20 * 160 + 100) * 4..][..4], &[255, 0, 0, 255]);
}

#[test]
fn later_static_atomic_opacity_context_paints_above_inline_opacity() {
    let (mut doc, outer, _) = document("opacity:.5;background:red");
    let body = doc.get_node(outer).unwrap().parent.unwrap();
    element(
        &mut doc,
        body,
        "span",
        "display:inline-block;vertical-align:top;opacity:.5;background:blue;width:60px;height:35px;margin-left:-60px",
    );
    let rgba = pixels(&mut doc);
    assert!(rgba.as_chunks::<4>().0.contains(&[127, 64, 192, 255]));
    assert!(!rgba.as_chunks::<4>().0.contains(&[192, 64, 127, 255]));
}

#[test]
fn mask_only_ancestor_composes_after_hoisted_inline_descendants() {
    let (mut doc, outer, _) = document("opacity:1");
    let body = doc.get_node(outer).unwrap().parent.unwrap();
    let ancestor = element(
        &mut doc,
        body,
        "div",
        "mask-image:linear-gradient(to right,black 50%,transparent 50%);width:80px;height:40px",
    );
    let owner = element(&mut doc, ancestor, "span", "opacity:.5");
    text(&mut doc, owner, "M");
    element(
        &mut doc,
        owner,
        "span",
        "position:absolute;z-index:1;left:0;top:0;width:160px;height:80px;background:blue",
    );
    let rgba = pixels(&mut doc);
    // The absolute box is hoisted out of the formatting root, but remains
    // inside its ancestor's mask, including the transparent right half.
    assert_eq!(&rgba[(60 * 160 + 60) * 4..][..4], &[255, 255, 255, 255]);
}

#[test]
fn ancestor_filter_wraps_hoisted_inline_opacity_once() {
    let (mut doc, outer, _) = document("opacity:0");
    let body = doc.get_node(outer).unwrap().parent.unwrap();
    let ancestor = element(
        &mut doc,
        body,
        "div",
        "filter:grayscale(1);width:60px;height:40px",
    );
    let owner = element(&mut doc, ancestor, "span", "opacity:.5");
    for (z, color, left) in [(1, "red", 10), (2, "blue", 20)] {
        element(
            &mut doc,
            owner,
            "span",
            &format!(
                "position:absolute;z-index:{z};left:{left}px;top:10px;width:30px;height:25px;background:{color}"
            ),
        );
    }
    doc.resolve(0.0);
    let mut scene = Scene::new();
    super::paint_scene(&mut scene, &mut doc, 1.0, 160, 80, 0, 0);
    // The CPU adapter currently disables DOM filters. Verify the portable
    // scene contract independently: both boxes share one owner opacity layer
    // inside exactly one ancestor filter, rather than filtering each fragment.
    let mut stack = Vec::new();
    let mut filters = 0;
    let mut red_seen = false;
    let mut blue_seen = false;
    for command in scene.commands {
        match command {
            RenderCommand::PushLayer(layer) => {
                if layer.filter.is_some() {
                    filters += 1;
                }
                stack.push((layer.filter.is_some(), layer.alpha));
            }
            RenderCommand::PushClipLayer(_) => stack.push((false, 1.0)),
            RenderCommand::PopLayer => {
                stack.pop();
            }
            RenderCommand::Fill(fill) if matches!(fill.brush, anyrender::Paint::Solid(color) if color == Color::from_rgb8(255,0,0) || color == Color::from_rgb8(0,0,255)) =>
            {
                assert_eq!(stack.iter().filter(|(filter, _)| *filter).count(), 1);
                assert_eq!(stack.iter().filter(|(_, alpha)| *alpha == 0.5).count(), 1);
                red_seen |= matches!(fill.brush, anyrender::Paint::Solid(color) if color == Color::from_rgb8(255,0,0));
                blue_seen |= matches!(fill.brush, anyrender::Paint::Solid(color) if color == Color::from_rgb8(0,0,255));
            }
            _ => {}
        }
    }
    assert_eq!(filters, 1);
    assert!(red_seen && blue_seen);
}

#[test]
fn containing_block_overflow_clips_hoisted_inline_descendant() {
    let (mut doc, outer, _) = document("opacity:1");
    let body = doc.get_node(outer).unwrap().parent.unwrap();
    let ancestor = element(
        &mut doc,
        body,
        "div",
        "position:relative;overflow:hidden;width:25px;height:40px",
    );
    let owner = element(&mut doc, ancestor, "span", "opacity:.5");
    element(
        &mut doc,
        owner,
        "span",
        "position:absolute;z-index:1;left:10px;top:10px;width:30px;height:25px;background:blue",
    );
    let rgba = pixels(&mut doc);
    assert_eq!(&rgba[(60 * 160 + 20) * 4..][..4], &[127, 127, 255, 255]);
    assert_eq!(&rgba[(60 * 160 + 35) * 4..][..4], &[255, 255, 255, 255]);
}
