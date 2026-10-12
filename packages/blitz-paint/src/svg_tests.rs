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
/// default 300x150, and paints its user units as CSS pixels from its origin,
/// at the device scale and clipped to its box, rather than fitting its content
/// into the box (Chrome).
#[test]
fn unsized_inline_svg_paints_at_its_own_scale_in_the_default_box() {
    for scale in [1.0, 2.0] {
        let mut doc = BaseDocument::new(DocumentConfig::default());
        let root = doc.root_node().id;
        let html_name = |tag: &str| QualName::new(None, ns!(html), tag.into());
        let html = element(&mut doc, root, html_name("html"), &[("style", "margin:0")]);
        let body = element(&mut doc, html, html_name("body"), &[("style", "margin:0")]);
        let svg_name = QualName::new(None, ns!(svg), "svg".into());
        let svg = element(&mut doc, body, svg_name, &[("style", "display:block")]);
        let rect = [
            ("x", "60"),
            ("y", "30"),
            ("width", "500"),
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
        super::paint_scene(&mut scene, &mut doc, scale, 800, 600, 0, 0);
        let red = Paint::Solid(peniko::Color::from_rgb8(255, 0, 0));
        let fill = scene.commands.iter().find_map(|command| match command {
            RenderCommand::Fill(fill) if fill.brush == red => Some(fill),
            _ => None,
        });
        let fill = fill.expect("the rect paints");
        let bounds = kurbo::Shape::bounding_box(&fill.shape);
        let placed = fill.transform.transform_rect_bbox(bounds);
        let expected = kurbo::Rect::new(60.0, 30.0, 560.0, 80.0);
        assert_eq!(placed, expected.scale_from_origin(scale), "{scale}");
        let clip = scene.commands.iter().find_map(|command| match command {
            RenderCommand::PushClipLayer(clip) => Some(clip),
            _ => None,
        });
        let clip = clip.expect("the content box clips");
        let clipped = clip
            .transform
            .transform_rect_bbox(kurbo::Shape::bounding_box(&clip.clip));
        let content = kurbo::Rect::new(0.0, 0.0, 300.0, 150.0);
        assert_eq!(clipped, content.scale_from_origin(scale), "{scale}");
    }
}

/// Answers every request with an SVG document.
struct SvgProvider(String);
impl blitz_traits::net::NetProvider for SvgProvider {
    fn fetch(
        &self,
        _: usize,
        request: blitz_traits::net::Request,
        handler: Box<dyn blitz_traits::net::NetHandler>,
    ) {
        handler.bytes(request.url.to_string(), self.0.clone().into_bytes().into());
    }
}

/// What a scene paints in red, and the clips it pushes, as device rectangles.
#[derive(Debug, Default)]
struct Painted {
    red: Vec<kurbo::Rect>,
    clips: Vec<kurbo::Rect>,
}

/// A document whose body holds a `tag` element with `attrs`, its images
/// answered with `svg`, painted at scale 1.
fn paint(svg: &str, tag: &str, attrs: &[(&str, &str)]) -> Painted {
    let mut doc = BaseDocument::new(DocumentConfig {
        base_url: Some("https://page.test/".into()),
        net_provider: Some(std::sync::Arc::new(SvgProvider(svg.to_owned()))),
        ..Default::default()
    });
    let root = doc.root_node().id;
    let html_name = |tag: &str| QualName::new(None, ns!(html), tag.into());
    let root_element = element(&mut doc, root, html_name("html"), &[("style", "margin:0")]);
    let body = element(
        &mut doc,
        root_element,
        html_name("body"),
        &[("style", "margin:0")],
    );
    element(&mut doc, body, html_name(tag), attrs);
    doc.resolve(0.0);
    doc.handle_messages();
    doc.resolve(0.0);
    let mut scene = Scene::new();
    super::paint_scene(&mut scene, &mut doc, 1.0, 100, 100, 0, 0);
    let red = Paint::Solid(peniko::Color::from_rgb8(255, 0, 0));
    let mut painted = Painted::default();
    for command in &scene.commands {
        match command {
            RenderCommand::Fill(fill) if fill.brush == red => {
                let bounds = kurbo::Shape::bounding_box(&fill.shape);
                painted.red.push(fill.transform.transform_rect_bbox(bounds));
            }
            RenderCommand::PushClipLayer(clip) => {
                let bounds = kurbo::Shape::bounding_box(&clip.clip);
                painted
                    .clips
                    .push(clip.transform.transform_rect_bbox(bounds));
            }
            _ => {}
        }
    }
    painted
}

/// An SVG image whose root has `attributes`, holding a 4x5 red rect.
fn root(attributes: &str) -> String {
    format!(
        "<svg xmlns='http://www.w3.org/2000/svg' {attributes}><rect width='4' height='5' fill='red'/></svg>"
    )
}

/// Where a 40x40 background of `svg` sized by `size` paints red.
fn background(svg: &str, size: &str) -> Vec<kurbo::Rect> {
    background_painted(svg, size).red
}

fn background_painted(svg: &str, size: &str) -> Painted {
    let style =
        format!("width:40px;height:40px;background:url(a.svg) no-repeat;background-size:{size}");
    paint(svg, "div", &[("style", &style)])
}

/// An SVG background without a `viewBox` scales its user units to the tile
/// only along an axis with a positive natural dimension, and not at all with
/// a zero one; a zero natural width sizes the tile to nothing (Chrome,
/// screenshots against plain boxes).
#[test]
fn svg_backgrounds_without_a_view_box_scale_like_chrome() {
    let rect = |w, h| vec![kurbo::Rect::new(0.0, 0.0, w, h)];
    assert_eq!(background(&root("width='10'"), "auto"), rect(4.0, 5.0));
    assert_eq!(
        background(&root("width='10'"), "40px 40px"),
        rect(16.0, 5.0)
    );
    assert_eq!(
        background(&root("height='5'"), "40px 40px"),
        rect(4.0, 40.0)
    );
    assert_eq!(
        background(&root("width='8' height='10'"), "40px 40px"),
        rect(20.0, 20.0)
    );
    assert_eq!(background(&root("width='0' height='5'"), "auto"), vec![]);
    assert_eq!(
        background(&root("width='0' height='5'"), "40px 40px"),
        rect(4.0, 5.0)
    );
    assert_eq!(
        background(&root("width='5' height='0'"), "40px 40px"),
        rect(4.0, 5.0)
    );
    let view_box = root("width='0' viewBox='0 0 4 5'");
    assert_eq!(background(&view_box, "auto"), vec![]);
    assert!(!background(&view_box, "40px 40px").is_empty());
}

/// Content beyond the tile is clipped to it, as replaced content is to its
/// box; a tiny natural width scales enormously without panicking.
#[test]
fn svg_backgrounds_clip_to_their_tile() {
    let large = "<svg xmlns='http://www.w3.org/2000/svg' width='10' height='10'>\
        <rect width='40' height='40' fill='red'/></svg>";
    let painted = background_painted(large, "auto");
    assert_eq!(painted.red, vec![kurbo::Rect::new(0.0, 0.0, 40.0, 40.0)]);
    assert!(
        painted
            .clips
            .contains(&kurbo::Rect::new(0.0, 0.0, 10.0, 10.0)),
        "{painted:?}"
    );
    background(&root("width='1e-44' height='5'"), "40px 40px");
}

/// An `<img>` of an SVG whose root size is zero paints its content in a box
/// it is given; a zero `viewBox` paints nothing (Chrome).
#[test]
fn svg_images_without_area_paint_in_a_given_box() {
    let image = |svg: &str| {
        let style = ("style", "display:block;width:40px;height:40px");
        paint(svg, "img", &[("src", "a.svg"), style]).red
    };
    assert_eq!(
        image(&root("width='0' height='5'")),
        vec![kurbo::Rect::new(0.0, 0.0, 4.0, 5.0)]
    );
    assert_eq!(
        image(&root("width='10' height='8' viewBox='0 0 0 10'")),
        vec![]
    );
}

/// Chrome fits an SVG background's `viewBox` into the tile with its
/// `preserveAspectRatio` (a 4x2 `viewBox` in a 40px tile is 40x20 at y=10,
/// for 8x10 and 0x5 roots alike); here the root's own fit is stretched to
/// the tile (SUB-1606).
#[test]
fn svg_backgrounds_with_a_view_box_stretch_to_the_tile() {
    let content = "<rect width='4' height='2' fill='red'/>";
    let sized = format!(
        "<svg xmlns='http://www.w3.org/2000/svg' width='8' height='10' viewBox='0 0 4 2'>{content}</svg>"
    );
    assert_eq!(
        background(&sized, "40px 40px"),
        vec![kurbo::Rect::new(0.0, 12.0, 40.0, 28.0)]
    );
}
