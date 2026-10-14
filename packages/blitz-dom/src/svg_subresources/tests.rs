//! Which images an SVG image's `<image>` elements load, at every nesting
//! level, and leaving out those not loaded.
use super::{BodyDigest, MAX_IMAGE_HREFS, NestedImages, SubresourceRecorder};
use crate::DecodedImageBudget;
use crate::node::SvgImageData;
use crate::svg_bounds::{parse_svg_image, parse_svg_image_showing};
use crate::util::default_svg_fonts;
use std::collections::HashSet;
use std::io::Cursor;
use std::sync::Arc;

fn png_body(blue: u8) -> Vec<u8> {
    let mut bytes = Cursor::new(Vec::new());
    image::RgbaImage::from_pixel(1, 1, image::Rgba([0, 0, blue, 255]))
        .write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap();
    bytes.into_inner()
}

fn data_url(mime: &str, body: &[u8]) -> String {
    let encoded = percent_encoding::percent_encode(body, percent_encoding::NON_ALPHANUMERIC);
    format!("data:{mime},{encoded}")
}

fn svg(content: &str) -> String {
    format!(
        "<svg xmlns='http://www.w3.org/2000/svg' xmlns:xlink='http://www.w3.org/1999/xlink' width='4' height='4'>{content}</svg>"
    )
}

fn parse(source: &str) -> SvgImageData {
    parse_svg_image(
        source.as_bytes(),
        &default_svg_fonts(),
        &DecodedImageBudget::default(),
    )
    .unwrap()
}

fn bodies(svg: &SvgImageData) -> HashSet<BodyDigest> {
    svg.subresources.bodies().collect()
}

#[test]
fn image_elements_with_an_href_delay_the_load() {
    let png = data_url("image/png", &png_body(1));
    for (content, delays) in [
        (format!("<image href='{png}' width='4' height='4'/>"), true),
        ("<image href='https://elsewhere.test/a.png'/>".into(), true),
        (
            format!("<image style='display:none' xlink:href='{png}'/>"),
            true,
        ),
        ("<image width='4' height='4'/>".into(), false),
        (
            format!(
                "<filter id='f'><feImage href='{png}'/></filter><rect width='4' height='4' filter='url(#f)'/>"
            ),
            false,
        ),
    ] {
        let svg = parse(&svg(&content));
        assert_eq!(svg.subresources.delays_load, delays, "{content}");
    }
}

#[test]
fn image_bodies_are_found_at_every_nesting_level() {
    let (outer, inner) = (png_body(1), png_body(2));
    let nested_svg = svg(&format!(
        "<image xlink:href='{}' width='4' height='4'/>",
        data_url("image/png", &inner)
    ));
    let source = svg(&format!(
        "<image href='{}' width='2' height='4'/><image x='2' href='{}' width='2' height='4'/>\
         <image style='display:none' href='{}'/>\
         <filter id='f'><feImage href='{}'/></filter><rect width='4' height='4' filter='url(#f)'/>",
        data_url("image/png", &outer),
        data_url("image/svg+xml", nested_svg.as_bytes()),
        data_url("image/png", &png_body(3)),
        data_url("image/png", &png_body(4)),
    ));
    let hidden = png_body(3);
    let expected = [&outer[..], nested_svg.as_bytes(), &inner[..], &hidden[..]];
    // The hidden image loads too; the feImage is no `<image>`.
    let expected = expected.map(BodyDigest::of);
    assert_eq!(bodies(&parse(&source)), HashSet::from(expected));
}

#[test]
fn only_loaded_images_show_when_asked() {
    let body = png_body(1);
    let source = svg(&format!(
        "<rect width='4' height='4' fill='lime'/><image href='{}' width='4' height='4'/>",
        data_url("image/png", &body)
    ));
    let parse_showing = |loaded: HashSet<BodyDigest>| {
        let shown = NestedImages::Loaded(Arc::new(loaded));
        let (fonts, budget) = (default_svg_fonts(), DecodedImageBudget::default());
        parse_svg_image_showing(source.as_bytes(), &fonts, &budget, shown).unwrap()
    };
    let unloaded = parse_showing(HashSet::new());
    assert!(unloaded.subresources.omits_images);
    assert_eq!(images(unloaded.tree.root()), 0);
    let loaded = parse_showing(HashSet::from([BodyDigest::of(&body)]));
    assert!(!loaded.subresources.omits_images);
    assert_eq!(images(loaded.tree.root()), 1);
    assert!(!parse(&source).subresources.omits_images);
}

#[test]
fn image_hrefs_are_remembered_within_a_bound() {
    let images: String = (0..MAX_IMAGE_HREFS + 8)
        .map(|index| format!("<image href='data:text/plain,{index}'/>"))
        .collect();
    let options = usvg::roxmltree::ParsingOptions::default();
    let source = svg(&images);
    let doc = usvg::roxmltree::Document::parse_with_options(&source, options).unwrap();
    let recorder = SubresourceRecorder::new(NestedImages::All);
    recorder.scan_document(&doc);
    assert_eq!(recorder.lock().image_hrefs.len(), MAX_IMAGE_HREFS);
    let subresources = recorder.finish();
    assert!(subresources.delays_load);
    // Kept with the image, so charged to the decoded image budget with it.
    assert_eq!(subresources.retained_bytes(), MAX_IMAGE_HREFS * 8);
}

/// The images a tree draws.
fn images(group: &usvg::Group) -> usize {
    group
        .children()
        .iter()
        .map(|node| match node {
            usvg::Node::Group(group) => images(group),
            usvg::Node::Image(_) => 1,
            _ => 0,
        })
        .sum()
}

#[test]
fn uppercase_and_malformed_data_images_wait_until_loaded() {
    let png = data_url("image/png", &png_body(37)).replacen("data:", "DATA:", 1);
    for href in [png.as_str(), "DATA:image/png;base64,!!!", "DATA:malformed"] {
        let source = svg(&format!("<image href='{href}' width='4' height='4'/>"));
        let parse = |loaded| {
            parse_svg_image_showing(
                source.as_bytes(),
                &default_svg_fonts(),
                &Default::default(),
                NestedImages::Loaded(Arc::new(loaded)),
            )
            .unwrap()
        };
        let unloaded = parse(HashSet::new());
        assert!(unloaded.subresources.omits_images, "{href}");
        assert_eq!(images(unloaded.tree.root()), 0);
        let digest = BodyDigest::of_data_url(href).unwrap();
        let loaded = parse(HashSet::from([digest]));
        assert!(!loaded.subresources.omits_images);
        assert_eq!(images(loaded.tree.root()), 1);
    }
}
