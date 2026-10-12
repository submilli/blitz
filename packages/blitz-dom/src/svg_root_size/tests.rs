//! Root sizes of SVG images against offline Chrome 154
//! (`fixtures/local/image-state` `zeroSizes` and scratch probes recorded in
//! submilli-browser's SUB-1567 evidence).
use crate::node::SvgImageData;
use crate::svg_bounds::{SvgRefusal, parse_svg_image};
use crate::util::default_svg_fonts;

/// An SVG image whose root has `attributes`, holding a 4x5 rect.
fn root(attributes: &str) -> Result<SvgImageData, SvgRefusal> {
    let source = format!(
        "<svg xmlns='http://www.w3.org/2000/svg' {attributes}><rect width='4' height='5'/></svg>"
    );
    parse_svg_image(source.as_bytes(), &default_svg_fonts(), &Default::default())
}

fn natural(svg: &SvgImageData) -> (Option<f32>, Option<f32>) {
    (svg.intrinsic_width(), svg.intrinsic_height())
}

/// usvg refuses a root that resolves to an empty size; Chrome loads it with
/// that natural dimension zero, keeping its content for a box it is given.
#[test]
fn empty_root_sizes_are_natural_zeros() {
    let cases = [
        ("width='0' height='5'", (Some(0.0), Some(5.0)), (0.0, 5.0)),
        ("width='5' height='0'", (Some(5.0), Some(0.0)), (5.0, 0.0)),
        ("width='0' height='0'", (Some(0.0), Some(0.0)), (0.0, 0.0)),
        ("width='0'", (Some(0.0), None), (0.0, 150.0)),
        ("height='0'", (None, Some(0.0)), (300.0, 0.0)),
        ("width='0' viewBox='0 0 4 2'", (Some(0.0), None), (0.0, 0.0)),
        (
            "height='0' viewBox='0 0 4 2'",
            (None, Some(0.0)),
            (0.0, 0.0),
        ),
        (
            "width='0' height='50%' viewBox='0 0 4 2'",
            (Some(0.0), None),
            (0.0, 0.0),
        ),
        ("width='-5' height='5'", (Some(0.0), Some(5.0)), (0.0, 5.0)),
        (
            "width='0em' height='5mm'",
            (Some(0.0), Some(96.0 / 25.4 * 5.0)),
            (0.0, 18.897_638),
        ),
        // Positive, but nothing in usvg's single precision.
        (
            "width='1e-50' height='5'",
            (Some(0.0), Some(5.0)),
            (0.0, 5.0),
        ),
        // A percentage is no natural dimension, zero or not.
        ("width='0%' height='5'", (None, Some(5.0)), (300.0, 5.0)),
        // Spacing and quoting are the document's; entities resolve.
        (
            "width =  \"0\" height\t=\n'5'",
            (Some(0.0), Some(5.0)),
            (0.0, 5.0),
        ),
        (
            "width='&#48;' height='5'",
            (Some(0.0), Some(5.0)),
            (0.0, 5.0),
        ),
    ];
    for (attributes, expected, concrete) in cases {
        let svg = root(attributes).unwrap_or_else(|error| panic!("{attributes}: {error:?}"));
        assert_eq!(natural(&svg), expected, "{attributes}");
        let (width, height) = svg.concrete_size((300.0, 150.0));
        let close = (width - concrete.0).abs() < 1e-3 && (height - concrete.1).abs() < 1e-3;
        assert!(close, "{attributes}: {width}x{height}");
        assert!(
            svg.tree.root().has_children(),
            "{attributes} keeps its content"
        );
    }
}

/// The root dimension is the unprefixed attribute, as Chrome reads it:
/// usvg's prefixed ones in the SVG, XLink or XML namespace are removed
/// (Chrome: `s:width="0" height="5"` is 300x5, `xml:width="10" width="5"` 5).
#[test]
fn root_sizes_are_the_unprefixed_attributes() {
    let svg_namespace = "xmlns:s='http://www.w3.org/2000/svg'";
    let svg = root(&format!("{svg_namespace} s:width='0' height='5'")).unwrap();
    assert_eq!(natural(&svg), (None, Some(5.0)));
    assert_eq!(svg.concrete_size((300.0, 150.0)), (300.0, 5.0));
    let svg = root(&format!("{svg_namespace} s:width='7' height='5'")).unwrap();
    assert_eq!(svg.concrete_size((300.0, 150.0)), (300.0, 5.0));
    let svg = root("xml:width='10' width='5' height='5'").unwrap();
    assert_eq!(natural(&svg), (Some(5.0), Some(5.0)));
    assert_eq!(svg.tree.size().width(), 5.0, "usvg reads the same width");
    let svg = root("xmlns:x='urn:x' x:width='5' width='0' height='5'").unwrap();
    assert_eq!(natural(&svg), (Some(0.0), Some(5.0)));
    // A size beyond single precision is refused, not infinite.
    assert!(root("xml:width='10' width='1e39' height='10'").is_err());
    assert!(root("width='1e39' height='10'").is_err());
    assert!(root("width='4e36in' height='10'").is_err());
}

/// Hundreds of spaces around `=` misplace roxmltree's value range, not the
/// attribute's, which is what is replaced.
#[test]
fn spacing_beyond_value_ranges_resizes_the_root() {
    let space = " ".repeat(300);
    let svg = root(&format!("width{space}={space}'0' height='5'")).unwrap();
    assert_eq!(natural(&svg), (Some(0.0), Some(5.0)));
}

/// Absolute lengths are the declared ones, even where usvg sizes the tree to
/// its content (another dimension missing or a percentage, without a
/// `viewBox`). Chrome: `width="10"` is 10x150, `1in` 96, `5mm` 19 rounded.
#[test]
fn absolute_dimensions_are_declared_lengths() {
    let cases = [
        ("width='10'", (Some(10.0), None)),
        ("width='10' height='50%'", (Some(10.0), None)),
        ("height='7'", (None, Some(7.0))),
        ("width='1in' height='3pt'", (Some(96.0), Some(4.0))),
        ("width='1pc' height='1cm'", (Some(16.0), Some(96.0 / 2.54))),
        ("width='5mm'", (Some(96.0 / 25.4 * 5.0), None)),
        (
            "width='2em' height='5' font-size='10'",
            (Some(20.0), Some(5.0)),
        ),
        // Chrome: 32, 14 and 32. Font-relative lengths take usvg's
        // resolution: a 12px default font size, half an em for `ex`, and the
        // content's bounds when usvg sizes the tree to them (SUB-1605).
        ("width='2em' height='5'", (Some(24.0), Some(5.0))),
        ("width='2ex' height='5'", (Some(12.0), Some(5.0))),
        ("width='2em'", (Some(4.0), None)),
    ];
    for (attributes, expected) in cases {
        assert_eq!(
            natural(&root(attributes).unwrap()),
            expected,
            "{attributes}"
        );
    }
}

/// A zero `viewBox` only disables rendering: the image keeps its size.
#[test]
fn zero_view_boxes_keep_their_sizes() {
    let cases = [
        ("viewBox='0 0 0 10'", (300.0, 150.0)),
        ("viewBox='0 0 0 0'", (300.0, 150.0)),
        ("width='10' height='8' viewBox='0 0 0 10'", (10.0, 8.0)),
        ("width='10' viewBox='0 0 0 10'", (10.0, 150.0)),
    ];
    for (attributes, concrete) in cases {
        let svg = root(attributes).unwrap();
        assert!(svg.intrinsic_dimensions.degenerate_view_box, "{attributes}");
        assert_eq!(svg.viewbox_aspect_ratio(), None, "{attributes}");
        assert_eq!(svg.concrete_size((300.0, 150.0)), concrete, "{attributes}");
    }
}
