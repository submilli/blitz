//! Nested rasters decode once, within the bounds and the budget, and are
//! shared by every holder of the document.
use crate::DecodedImageBudget;
use crate::image_decode::{declared_png, solid_png};
use crate::node::SvgImageData;
use crate::svg_bounds::parse_svg_image;
use crate::util::default_svg_fonts;

const OPEN: &str = "<svg xmlns='http://www.w3.org/2000/svg' width='8' height='8'>";

/// `data` as a percent-encoded `data:` URL in an `<image>`.
fn image(mime: &str, data: &[u8]) -> String {
    let href: String = data.iter().map(|byte| format!("%{byte:02X}")).collect();
    format!("<image width='8' height='8' href='data:{mime},{href}'/>")
}

fn parse(body: &str, budget: &DecodedImageBudget) -> SvgImageData {
    let source = format!("{OPEN}{body}</svg>");
    parse_svg_image(source.as_bytes(), &default_svg_fonts(), budget).unwrap()
}

/// The image kinds of a tree, nested documents included.
fn kinds(tree: &usvg::Tree) -> Vec<usvg::ImageKind> {
    let mut kinds = Vec::new();
    let mut groups = vec![tree.root()];
    while let Some(group) = groups.pop() {
        for node in group.children() {
            match node {
                usvg::Node::Group(child) => groups.push(child),
                usvg::Node::Image(image) => {
                    if let usvg::ImageKind::SVG(nested) = image.kind() {
                        kinds.extend(self::kinds(nested));
                    }
                    kinds.push(image.kind().clone());
                }
                _ => {}
            }
        }
    }
    kinds
}

/// The pixel blob ids of the decodes of every raster in `svg`.
fn decodes(svg: &SvgImageData) -> Vec<u64> {
    kinds(&svg.tree)
        .iter()
        .filter(|kind| !matches!(kind, usvg::ImageKind::SVG(_)))
        .map(|kind| {
            let raster = svg.nested_rasters().decoded(kind).expect("a held decode");
            raster.data.id()
        })
        .collect()
}

#[test]
fn rasters_decode_once_when_parsed_and_clones_share_the_decode() {
    let budget = DecodedImageBudget::default();
    let png = solid_png(8, 8, [0, 255, 0, 255]);
    let svg = parse(&image("image/png", &png), &budget);
    let kind = &kinds(&svg.tree)[0];
    let raster = svg.nested_rasters().decoded(kind).unwrap();
    assert_eq!((raster.width, raster.height), (8, 8));
    assert_eq!(&raster.data.data()[..4], &[0, 255, 0, 255]);
    let decoded = 8 * 8 * 4;
    assert_eq!(budget.retained_bytes(), svg.retained_bytes + decoded);

    let clone = svg.clone();
    assert_eq!(decodes(&clone), decodes(&svg));
    assert_eq!(budget.retained_bytes(), svg.retained_bytes + decoded);
    drop((svg, clone));
    assert_eq!(budget.retained_bytes(), 0, "released with the document");
}

#[test]
fn equal_bodies_share_one_decode() {
    let budget = DecodedImageBudget::default();
    let png = solid_png(8, 8, [0, 0, 255, 255]);
    let svg = parse(&image("image/png", &png).repeat(3), &budget);
    let decodes = decodes(&svg);
    assert_eq!(decodes.len(), 3);
    assert!(decodes.iter().all(|id| *id == decodes[0]), "{decodes:?}");
    assert_eq!(svg.nested_rasters().decoded_bytes(), 8 * 8 * 4);
    assert_eq!(budget.retained_bytes(), svg.retained_bytes + 8 * 8 * 4);
}

#[test]
fn rasters_of_nested_documents_decode_with_the_document() {
    let png = solid_png(8, 8, [255, 0, 0, 255]);
    let inner = format!("{OPEN}{}</svg>", image("image/png", &png));
    let svg = parse(
        &image("image/svg+xml", inner.as_bytes()),
        &DecodedImageBudget::default(),
    );
    assert_eq!(decodes(&svg).len(), 1);
}

/// A raster whose header is readable but whose pixels are not keeps its place
/// and draws nothing.
#[test]
fn rasters_that_do_not_decode_have_no_decode() {
    let budget = DecodedImageBudget::default();
    let svg = parse(&image("image/png", &declared_png(16)), &budget);
    let kinds = kinds(&svg.tree);
    assert_eq!(kinds.len(), 1, "a header without pixels");
    assert!(svg.nested_rasters().decoded(&kinds[0]).is_none());
    assert_eq!(budget.retained_bytes(), svg.retained_bytes);
}

#[test]
fn rasters_past_the_budget_have_no_decode() {
    // A 1 MiB decode against a budget that admits the document but not it.
    let png = solid_png(512, 512, [0, 0, 0, 255]);
    let body = format!("{}<rect width='8' height='8'/>", image("image/png", &png));
    let budget = DecodedImageBudget::with_limit(512 * 1024);
    let svg = parse(&body, &budget);
    let kinds = kinds(&svg.tree);
    assert_eq!(kinds.len(), 1, "the image keeps its place");
    assert!(svg.nested_rasters().decoded(&kinds[0]).is_none());
    assert_eq!(budget.retained_bytes(), svg.retained_bytes);
}

/// Only the rasters the parsed tree keeps count against the document's
/// bound: a skipped image (without area) decodes nothing and leaves the bound
/// to the images that draw.
#[test]
fn skipped_images_leave_the_bound_to_kept_ones() {
    let large = |grey: u8| solid_png(4096, 2048, [grey, grey, grey, 255]);
    let href = |png: &[u8]| -> String { png.iter().map(|byte| format!("%{byte:02X}")).collect() };
    let skipped = format!(
        "<image width='0' height='8' href='data:image/png,{}'/>",
        href(&large(0))
    );
    let svg = parse(
        &format!("{skipped}{}", image("image/png", &large(1))),
        &DecodedImageBudget::default(),
    );
    assert_eq!(decodes(&svg).len(), 1, "the kept image decodes");
}

/// Trees parsed apart from Blitz's bounds have no decodes to draw.
#[test]
fn rasters_of_other_trees_have_no_decode() {
    let png = solid_png(8, 8, [0, 0, 0, 255]);
    let body = image("image/png", &png);
    let svg = parse(&body, &Default::default());
    let source = format!("{OPEN}{body}</svg>");
    let other = usvg::Tree::from_str(&source, &usvg::Options::default()).unwrap();
    assert!(svg.nested_rasters().decoded(&kinds(&other)[0]).is_none());
}

/// usvg resolves an image before skipping it (here, one without area), so
/// its decode is released with nothing holding its body.
#[test]
fn rasters_usvg_drops_release_their_decodes() {
    let budget = DecodedImageBudget::default();
    let png = solid_png(64, 64, [0, 0, 0, 255]);
    let href: String = png.iter().map(|byte| format!("%{byte:02X}")).collect();
    let body = format!("<image width='0' height='8' href='data:image/png,{href}'/>");
    let svg = parse(&body, &budget);
    assert!(kinds(&svg.tree).is_empty());
    assert_eq!(svg.nested_rasters().decoded_bytes(), 0);
    assert_eq!(budget.retained_bytes(), svg.retained_bytes, "nothing held");
}

/// A document parsed only to measure its geometry keeps its images, by their
/// headers, without decoding or charging them.
#[test]
fn measured_documents_decode_nothing() {
    let budget = DecodedImageBudget::default();
    let png = solid_png(64, 64, [0, 0, 0, 255]);
    let source = format!("{OPEN}{}</svg>", image("image/png", &png));
    let svg =
        crate::svg_bounds::parse_svg_geometry(source.as_bytes(), &default_svg_fonts(), &budget)
            .unwrap();
    let kinds = kinds(&svg.tree);
    assert_eq!(kinds.len(), 1, "the image keeps its place");
    assert!(svg.nested_rasters().decoded(&kinds[0]).is_none());
    assert_eq!(budget.retained_bytes(), svg.retained_bytes);
}

/// usvg makes some rasters itself (the colour bitmap glyphs of flattened
/// text) without its resolver; a tree parsed by default options stands in for
/// them, as no test font has bitmap strikes. They decode with the document.
#[test]
fn rasters_the_resolver_never_saw_decode_when_finished() {
    use super::{NestedRasterDecoder, RasterPurpose};
    let budget = DecodedImageBudget::default();
    let png = solid_png(8, 8, [0, 0, 0, 255]);
    let source = format!("{OPEN}{}</svg>", image("image/png", &png));
    let tree = usvg::Tree::from_str(&source, &usvg::Options::default()).unwrap();
    let rasters = NestedRasterDecoder::new(&budget, RasterPurpose::Paint).decode(&tree);
    let kinds = kinds(&tree);
    assert_eq!(rasters.decoded(&kinds[0]).map(|r| r.width), Some(8));
    assert_eq!(budget.retained_bytes(), 8 * 8 * 4);
}

/// Under the document's bound, earlier rasters in document order decode
/// first, whatever groups hold them.
#[test]
fn rasters_decode_in_document_order() {
    let large = |grey: u8| solid_png(4096, 2048, [grey, grey, grey, 255]);
    let (first, second) = (large(1), large(2));
    let body = format!(
        "<g>{}</g><g>{}</g>",
        image("image/png", &first),
        image("image/png", &second)
    );
    let svg = parse(&body, &DecodedImageBudget::default());
    let decoded = |png: &[u8]| {
        kinds(&svg.tree).iter().any(|kind| {
            matches!(kind, usvg::ImageKind::PNG(data) if data.as_slice() == png)
                && svg.nested_rasters().decoded(kind).is_some()
        })
    };
    assert!(decoded(&first), "the first image decodes");
    assert!(!decoded(&second), "the second passes the bound");
}
