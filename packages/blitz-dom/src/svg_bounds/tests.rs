//! SVGZ inflation, parse reservations and retained costs of SVG documents.
use super::{MAX_NESTED_SVG_IMAGES, MAX_SVG_SOURCE_BYTES, SvgRefusal, parse_svg_image};
use crate::DecodedImageBudget;
use crate::node::SvgImageData;
use crate::util::default_svg_fonts;
use std::io::Write as _;

const OPEN: &str = "<svg xmlns='http://www.w3.org/2000/svg' width='4' height='4'>";

fn parse(source: &[u8]) -> Result<SvgImageData, SvgRefusal> {
    parse_svg_image(source, &default_svg_fonts(), &DecodedImageBudget::default())
}

fn cost(source: &str) -> usize {
    parse(source.as_bytes()).unwrap().retained_bytes
}

fn gzip(source: &[u8]) -> Vec<u8> {
    let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    gzip.write_all(source).unwrap();
    gzip.finish().unwrap()
}

/// `data` as a percent-encoded `data:` URL in an `<image>` of an SVG document.
fn nesting(mime: &str, data: &[u8]) -> String {
    let href: String = data.iter().map(|byte| format!("%{byte:02X}")).collect();
    format!("<image width='4' height='4' href='data:{mime},{href}'/>")
}

fn images(svg: &SvgImageData) -> usize {
    let mut count = 0;
    let mut groups = vec![svg.tree.root()];
    while let Some(group) = groups.pop() {
        for node in group.children() {
            match node {
                usvg::Node::Group(child) => groups.push(child),
                usvg::Node::Image(_) => count += 1,
                _ => {}
            }
        }
    }
    count
}

#[test]
fn svg_documents_are_admitted_by_their_size() {
    let rects = "<rect width='1' height='1'/>".repeat(10);
    let svg = parse(format!("{OPEN}{rects}</svg>").as_bytes()).unwrap();
    let cost = svg.retained_bytes;
    assert!(cost >= 10 * 1024, "{cost}");

    assert!(!DecodedImageBudget::with_limit(cost - 1).admit_svg(&svg));
    let budget = DecodedImageBudget::with_limit(cost);
    assert!(budget.admit_svg(&svg));
    assert_eq!(budget.retained_bytes(), cost);
    drop(svg);
    assert_eq!(budget.retained_bytes(), 0);
}

#[test]
fn costs_count_resources_outside_the_rendered_tree() {
    let points: String = (0..1000).map(|i| format!(" L{i} {i}")).collect();
    let clip = cost(&format!(
        "{OPEN}<clipPath id='c'><path d='M0 0{points}'/></clipPath><rect width='4' height='4' clip-path='url(#c)'/></svg>"
    ));
    assert!(clip >= 1000 * 16, "{clip}");
    let stops: String = (0..200)
        .map(|i| format!("<stop offset='{}'/>", i as f32 / 200.0))
        .collect();
    let gradient = cost(&format!(
        "{OPEN}<linearGradient id='g'>{stops}</linearGradient><rect width='4' height='4' fill='url(#g)'/></svg>"
    ));
    assert!(gradient >= 200 * 64, "{gradient}");
}

/// A nested raster's decode is charged on its own, once, rather than with the
/// tree for each paint.
#[test]
fn nested_rasters_are_charged_their_decode_once() {
    let png = crate::image_decode::solid_png(512, 512, [0, 0, 0, 255]);
    let budget = DecodedImageBudget::default();
    let source = format!("{OPEN}{}</svg>", nesting("image/png", &png));
    let svg = parse_svg_image(source.as_bytes(), &default_svg_fonts(), &budget).unwrap();
    assert!(svg.retained_bytes < 512 * 512 * 4, "{}", svg.retained_bytes);
    assert_eq!(budget.retained_bytes(), svg.retained_bytes + 512 * 512 * 4);
}

#[test]
fn svgz_inflates_within_a_bound() {
    assert!(parse(&gzip(format!("{OPEN}</svg>").as_bytes())).is_ok());

    let mut bomb = OPEN.as_bytes().to_vec();
    bomb.resize(MAX_SVG_SOURCE_BYTES as usize + 1, b' ');
    bomb.extend(b"</svg>");
    let bomb = gzip(&bomb);
    assert!(bomb.len() < 64 * 1024, "{}", bomb.len());
    assert!(matches!(parse(&bomb), Err(SvgRefusal::Invalid)));
    assert!(matches!(parse(&gzip(&bomb)), Err(SvgRefusal::Invalid)));
}

#[test]
fn nested_svgz_inflates_once_within_the_bound() {
    let inner = format!("{OPEN}<rect width='1' height='1'/></svg>");
    let once = parse(
        format!(
            "{OPEN}{}</svg>",
            nesting("image/svg+xml", &gzip(inner.as_bytes()))
        )
        .as_bytes(),
    )
    .unwrap();
    assert_eq!(images(&once), 1, "one layer of gzip inflates");
    // usvg would inflate a second layer itself, without a bound.
    let twice = gzip(&gzip(inner.as_bytes()));
    let twice =
        parse(format!("{OPEN}{}</svg>", nesting("image/svg+xml", &twice)).as_bytes()).unwrap();
    assert_eq!(images(&twice), 0);
}

#[test]
fn nested_documents_share_one_source_bound() {
    let half = MAX_SVG_SOURCE_BYTES as usize / 2 + 1;
    let mut inner = OPEN.as_bytes().to_vec();
    inner.resize(half, b' ');
    inner.extend(b"</svg>");
    let nested = gzip(&inner);
    let image = nesting("image/svg+xml", &nested);
    let svg = parse(format!("{OPEN}{image}</svg>").as_bytes()).unwrap();
    assert_eq!(images(&svg), 1);
    let svg = parse(format!("{OPEN}{image}{image}</svg>").as_bytes()).unwrap();
    assert_eq!(images(&svg), 1, "the second passes the shared bound");
}

#[test]
fn what_parsing_allocates_up_front_is_reserved() {
    let markup = format!("{OPEN}<!--{}--></svg>", "<".repeat(100_000));
    let budget = DecodedImageBudget::with_limit(1024 * 1024);
    let parsed = parse_svg_image(markup.as_bytes(), &default_svg_fonts(), &budget);
    assert!(matches!(parsed, Err(SvgRefusal::Budget)));
    assert!(
        budget.reserve(1024 * 1024).is_some(),
        "released after parsing"
    );
    assert!(parse(markup.as_bytes()).is_ok(), "within a larger budget");
}

const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";

/// usvg's default resolver reads `href` paths from the filesystem; page SVG
/// must not.
#[test]
fn svg_images_never_read_the_filesystem() {
    let decode = |text| data_url::forgiving_base64::decode_to_vec(text).unwrap();
    let path = std::env::temp_dir().join(format!("blitz-svg-{}.png", std::process::id()));
    std::fs::write(&path, decode(PNG.as_bytes())).unwrap();
    let svg = format!(
        "{OPEN}<image href='{}' width='4' height='4'/><image href='data:image/png;base64,{PNG}' width='4' height='4'/></svg>",
        path.display()
    );
    let parsed = parse(svg.as_bytes()).unwrap();
    let _ = std::fs::remove_file(&path);
    assert_eq!(images(&parsed), 1, "only the data: image loads");
}

/// Nested raster images are bounded by their headers before they decode:
/// sides past the decode limit, or images past the document's byte bound, are
/// dropped rather than decoded.
#[test]
fn nested_raster_images_are_bounded_by_their_headers() {
    use crate::image_decode::{declared_png, solid_png};
    let nested = |body: String| images(&parse(format!("{OPEN}{body}</svg>").as_bytes()).unwrap());
    let declared = |side: u32| nesting("image/png", &declared_png(side));
    assert_eq!(nested(declared(65_536)), 0, "a side past the decode limit");
    // Each 2048² image decodes to half the bound; equal bodies share one.
    let image = |grey: u8| nesting("image/png", &solid_png(2048, 2048, [grey, grey, grey, 255]));
    assert_eq!(nested(format!("{}{}{}", image(0), image(1), image(2))), 2);
    assert_eq!(nested(image(0).repeat(3)), 3);
}

#[test]
fn nested_images_share_one_bound() {
    let image = format!("<image href='data:image/png;base64,{PNG}' width='1' height='1'/>");
    let svg = format!("{OPEN}{}</svg>", image.repeat(40));
    assert_eq!(
        images(&parse(svg.as_bytes()).unwrap()),
        MAX_NESTED_SVG_IMAGES
    );
}

/// usvg parses nested documents itself, and refuses one whose root size is
/// not positive. Chrome draws it in its `<image>` box, so it is resized first.
#[test]
fn nested_documents_of_no_size_are_drawn() {
    let nested = b"<svg xmlns='http://www.w3.org/2000/svg' width='0' height='5'><rect width='4' height='5'/></svg>";
    for (mime, data) in [
        ("image/svg+xml", nested.to_vec()),
        ("text/plain", nested.to_vec()),
        ("image/svg+xml", gzip(nested)),
    ] {
        let svg = parse(format!("{OPEN}{}</svg>", nesting(mime, &data)).as_bytes()).unwrap();
        assert_eq!(images(&svg), 1, "{mime}");
    }
}

/// usvg draws a nested document with a zero `viewBox` as if it had none, and
/// fits a nested document's own size into its `<image>` box; Chrome draws
/// nothing for the first and lays the second out in the box (SUB-1606).
#[test]
fn nested_documents_with_a_zero_view_box_are_drawn() {
    let nested = b"<svg xmlns='http://www.w3.org/2000/svg' width='4' height='4' viewBox='0 0 0 4'><rect width='4' height='4'/></svg>";
    let svg =
        parse(format!("{OPEN}{}</svg>", nesting("image/svg+xml", nested)).as_bytes()).unwrap();
    assert_eq!(images(&svg), 1);
}
