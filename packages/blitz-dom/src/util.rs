use crate::node::{Node, NodeData};
use color::{AlphaColor, Srgb};
use keyboard_types::Modifiers;
use std::borrow::Cow;
use style::color::AbsoluteColor;

#[cfg(target_os = "macos")]
pub(crate) const ACTION_MOD: Modifiers = Modifiers::SUPER;
#[cfg(not(target_os = "macos"))]
pub(crate) const ACTION_MOD: Modifiers = Modifiers::CONTROL;

pub type Color = AlphaColor<Srgb>;

/// Decode raw font bytes, decompressing WOFF/WOFF2 if the `woff` feature is enabled.
/// Returns the original slice unchanged for TTF/OTF input, and also on decompression
/// failure. With the `woff` feature disabled, all input passes through unchanged.
pub fn decode_font_bytes(bytes: &[u8]) -> Cow<'_, [u8]> {
    if bytes.len() < 4 {
        return Cow::Borrowed(bytes);
    }
    match &bytes[0..4] {
        #[cfg(feature = "woff")]
        b"wOFF" => wuff::decompress_woff1(bytes)
            .map(Cow::Owned)
            .unwrap_or_else(|_| {
                #[cfg(feature = "tracing")]
                tracing::warn!("Failed to decompress woff1 font");
                Cow::Borrowed(bytes)
            }),
        #[cfg(feature = "woff")]
        b"wOF2" => wuff::decompress_woff2(bytes)
            .map(Cow::Owned)
            .unwrap_or_else(|_| {
                #[cfg(feature = "tracing")]
                tracing::warn!("Failed to decompress woff2 font");
                Cow::Borrowed(bytes)
            }),
        _ => Cow::Borrowed(bytes),
    }
}

#[cfg(feature = "svg")]
use std::sync::{Arc, LazyLock};
#[cfg(feature = "svg")]
use usvg::fontdb;

/// A document's fonts for SVG text; nothing without the `svg` feature.
#[cfg(feature = "svg")]
pub(crate) type SvgFonts = crate::SvgFontDb;
#[cfg(not(feature = "svg"))]
#[derive(Clone)]
pub(crate) struct SvgFonts;

/// SVG fonts for documents whose embedder supplies none: the system fonts
/// (loaded once per process), or none without the `system-fonts` feature.
#[cfg(feature = "svg")]
pub(crate) fn default_svg_fonts() -> crate::SvgFontDb {
    static FONT_DB: LazyLock<Arc<fontdb::Database>> = LazyLock::new(|| {
        #[allow(unused_mut)]
        let mut db = fontdb::Database::new();
        #[cfg(feature = "system-fonts")]
        db.load_system_fonts();
        Arc::new(db)
    });
    Arc::clone(&FONT_DB)
}

/// Which kind of CSS image layer list (`background-image` or `mask-image`) to
/// flush from style to dedicated storage on the node.
#[derive(Clone, Copy, Debug)]
pub enum ImageLayerKind {
    Background,
    Mask,
}

impl ImageLayerKind {
    pub fn image_type(self, idx: usize) -> ImageType {
        match self {
            Self::Background => ImageType::Background(idx),
            Self::Mask => ImageType::Mask(idx),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum ImageType {
    Image,
    Background(usize),
    Mask(usize),
    /// An SVG `feImage` element's `href`. Only built with the `svg` feature,
    /// which fetches and delivers these.
    FilterImage,
}

/// A point
#[derive(Clone, Debug, Copy, Eq, PartialEq)]
pub struct Point<T> {
    /// The x coordinate
    pub x: T,
    /// The y coordinate
    pub y: T,
}

impl Point<f64> {
    pub const ZERO: Self = Point { x: 0.0, y: 0.0 };
}

// Debug print an RcDom
pub fn walk_tree(indent: usize, node: &Node) {
    // Skip all-whitespace text nodes entirely
    if let NodeData::Text(data) = &node.data {
        if data
            .content
            .as_str_lossy()
            .chars()
            .all(|c| c.is_ascii_whitespace())
        {
            return;
        }
    }

    print!("{}", " ".repeat(indent));
    let id = node.id;
    match &node.data {
        NodeData::Document(_) => println!("#Document {id}"),

        NodeData::Text(data) => {
            if data
                .content
                .as_str_lossy()
                .chars()
                .all(|c| c.is_ascii_whitespace())
            {
                println!("{id} #text: <whitespace>");
            } else {
                let content = data.content.as_str_lossy().trim();
                if content.len() > 10 {
                    println!(
                        "#text {id}: {}...",
                        content
                            .split_at(content.char_indices().take(10).last().unwrap().0)
                            .0
                            .escape_default()
                    )
                } else {
                    println!(
                        "#text {id}: {}",
                        data.content.as_str_lossy().trim().escape_default()
                    )
                }
            }
        }

        NodeData::Comment { .. } => println!("<!-- COMMENT {id} -->"),
        NodeData::DocumentFragment => println!("#document-fragment {id}"),
        NodeData::Doctype { name, .. } => println!("<!DOCTYPE {name}> {id}"),
        NodeData::ProcessingInstruction { target, .. } => println!("<?{target}?> {id}"),

        NodeData::AnonymousBlock(_) => println!("{id} AnonymousBlock"),

        NodeData::Element(data) => {
            print!("<{} {id}", data.name.local);
            for attr in data.attrs.iter() {
                print!(" {}=\"{}\"", attr.name.local, attr.value);
            }
            if !node.children.is_empty() {
                println!(">");
            } else {
                println!("/>");
            }
        } // NodeData::Doctype {
          //     ref name,
          //     ref public_id,
          //     ref system_id,
          // } => println!("<!DOCTYPE {} \"{}\" \"{}\">", name, public_id, system_id),
          // NodeData::ProcessingInstruction { .. } => unreachable!(),
    }

    if !node.children.is_empty() {
        for child_id in node.children.iter() {
            walk_tree(indent + 2, node.with(*child_id));
        }

        if let NodeData::Element(data) = &node.data {
            println!("{}</{}>", " ".repeat(indent), data.name.local);
        }
    }
}

/// Images an SVG document may nest (including within nested SVG images).
#[cfg(feature = "svg")]
const MAX_NESTED_SVG_IMAGES: usize = 16;
/// Decoded bytes of the raster images one SVG document may nest. The renderer
/// decodes them itself, so their headers are checked against this up front.
#[cfg(feature = "svg")]
const MAX_NESTED_SVG_IMAGE_BYTES: u64 = 64 * 1024 * 1024;

/// Parse an SVG image. Page-supplied SVG never reaches the network or the
/// filesystem: nested images may only be `data:` URLs, a bounded number of them.
#[cfg(feature = "svg")]
pub(crate) fn parse_svg_image(
    source: &[u8],
    fonts: &crate::SvgFontDb,
) -> Result<crate::node::SvgImageData, usvg::Error> {
    crate::node::SvgImageData::from_data(source, &svg_options(fonts))
}

/// usvg options without ambient resource access. usvg's default string resolver
/// reads paths from the filesystem; nested SVG documents forward only the data
/// resolver, so its budgets cover every nesting level. Raster images stay
/// encoded until a renderer decodes them, so only those whose headers fit the
/// decode bounds and the shared byte budget are kept.
#[cfg(feature = "svg")]
pub(crate) fn svg_options(fonts: &crate::SvgFontDb) -> usvg::Options<'static> {
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    let nested = Arc::new(AtomicUsize::new(0));
    let bytes = Arc::new(AtomicU64::new(0));
    let decode = usvg::ImageHrefResolver::default_data_resolver();
    usvg::Options {
        fontdb: Arc::clone(fonts),
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: Box::new(move |mime, data, options| {
                if nested.fetch_add(1, Ordering::Relaxed) >= MAX_NESTED_SVG_IMAGES {
                    return None;
                }
                let kind = decode(mime, data, options)?;
                let encoded = match &kind {
                    usvg::ImageKind::JPEG(data)
                    | usvg::ImageKind::PNG(data)
                    | usvg::ImageKind::GIF(data)
                    | usvg::ImageKind::WEBP(data) => data,
                    usvg::ImageKind::SVG(_) => return Some(kind),
                };
                let decoded = crate::net::bounded_rgba_bytes(encoded)?;
                let total = bytes.fetch_add(decoded, Ordering::Relaxed) + decoded;
                (total <= MAX_NESTED_SVG_IMAGE_BYTES).then_some(kind)
            }),
            resolve_string: Box::new(|_, _| None),
        },
        ..Default::default()
    }
}

pub trait ToColorColor {
    /// Converts a color into the `AlphaColor<Srgb>` type from the `color` crate
    fn as_color_color(&self) -> Color;
}
impl ToColorColor for AbsoluteColor {
    fn as_color_color(&self) -> Color {
        Color::new(
            *self
                .to_color_space(style::color::ColorSpace::Srgb)
                .raw_components(),
        )
    }
}

#[cfg(all(test, feature = "svg"))]
mod svg_tests {
    use super::parse_svg_image;

    fn images(group: &usvg::Group) -> usize {
        group
            .children()
            .iter()
            .map(|node| match node {
                usvg::Node::Image(_) => 1,
                usvg::Node::Group(group) => images(group),
                _ => 0,
            })
            .sum()
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
            "<svg xmlns='http://www.w3.org/2000/svg' width='4' height='4'><image href='{}' width='4' height='4'/><image href='data:image/png;base64,{PNG}' width='4' height='4'/></svg>",
            path.display()
        );
        let parsed = parse_svg_image(svg.as_bytes(), &super::default_svg_fonts()).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(images(parsed.tree.root()), 1, "only the data: image loads");
    }

    /// The renderer decodes nested raster images itself, so their headers
    /// bound them first: sides past the decode limit, or images past the shared
    /// byte budget, are dropped rather than decoded.
    #[test]
    fn nested_raster_images_are_bounded_by_their_headers() {
        let image = |side: u32| {
            let png = crate::net::declared_png(side);
            let encoded =
                percent_encoding::percent_encode(&png, percent_encoding::NON_ALPHANUMERIC);
            format!("<image href='data:image/png,{encoded}' width='1' height='1'/>")
        };
        let parse = |body: String| {
            let svg = format!(
                "<svg xmlns='http://www.w3.org/2000/svg' width='4' height='4'>{body}</svg>"
            );
            let parsed = parse_svg_image(svg.as_bytes(), &super::default_svg_fonts()).unwrap();
            images(parsed.tree.root())
        };
        assert_eq!(parse(image(65_536)), 0, "a side past the decode limit");
        // Each 4096² image decodes to the whole 64 MiB budget.
        assert_eq!(parse(image(4096).repeat(3)), 1);
        assert_eq!(parse(image(16).repeat(3)), 3);
    }

    #[test]
    fn nested_images_share_one_budget() {
        let image = format!("<image href='data:image/png;base64,{PNG}' width='1' height='1'/>");
        let svg = format!(
            "<svg xmlns='http://www.w3.org/2000/svg' width='4' height='4'>{}</svg>",
            image.repeat(40)
        );
        let parsed = parse_svg_image(svg.as_bytes(), &super::default_svg_fonts()).unwrap();
        assert_eq!(images(parsed.tree.root()), super::MAX_NESTED_SVG_IMAGES);
    }

    #[test]
    fn missing_height_is_computed_from_width_and_viewbox_ratio() {
        let src = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100" width="200"><rect width="100%" height="100%" fill="green"/></svg>"#;
        let svg = parse_svg_image(src, &super::default_svg_fonts()).unwrap();
        assert_eq!(svg.intrinsic_width(), Some(200.0));
        assert_eq!(svg.intrinsic_height(), None);
        assert_eq!(svg.viewbox_aspect_ratio(), Some(1.0));
        assert_eq!(svg.tree.size().width(), 200.0);
        assert_eq!(svg.intrinsic_size(), (200.0, 200.0));
    }

    #[test]
    fn viewbox_only_has_no_intrinsic_dimensions() {
        let src = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 485 58"></svg>"#;
        let svg = parse_svg_image(src, &super::default_svg_fonts()).unwrap();
        assert_eq!(svg.intrinsic_width(), None);
        assert_eq!(svg.intrinsic_height(), None);
        // The aspect ratio is still available from the viewBox.
        assert!((svg.aspect_ratio() - (485.0 / 58.0)).abs() < 1e-3);
    }

    #[test]
    fn concrete_size_follows_the_default_sizing_algorithm() {
        let size = |root: &str| {
            let src = format!("<svg xmlns='http://www.w3.org/2000/svg' {root}/>");
            let svg = parse_svg_image(src.as_bytes(), &super::default_svg_fonts()).unwrap();
            svg.concrete_size((20.0, 16.0))
        };
        assert_eq!(size("width='8' height='6' viewBox='0 0 4 2'"), (8.0, 6.0));
        assert_eq!(size("width='10' viewBox='0 0 4 2'"), (10.0, 5.0));
        assert_eq!(size("height='4' viewBox='0 0 4 2'"), (8.0, 4.0));
        assert_eq!(size("width='10'"), (10.0, 16.0));
        assert_eq!(size("height='3'"), (20.0, 3.0));
        // Contained in the default: limited by its width, then its height.
        assert_eq!(size("viewBox='0 0 4 2'"), (20.0, 10.0));
        assert_eq!(size("viewBox='0 0 1 2'"), (8.0, 16.0));
        assert_eq!(size("width='50%' viewBox='0 0 4 2'"), (20.0, 10.0));
        // No ratio: the default, not usvg's 100x100 fallback.
        assert_eq!(size(""), (20.0, 16.0));
    }

    #[test]
    fn root_preserve_aspect_ratio_is_captured() {
        let parse = |root: &str| {
            let src = format!("<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 4 2' {root}/>");
            let svg = parse_svg_image(src.as_bytes(), &super::default_svg_fonts()).unwrap();
            svg.intrinsic_dimensions.preserve_aspect_ratio
        };
        let aspect = parse("preserveAspectRatio='xMinYMax slice'");
        assert_eq!(aspect.align, svgtypes::Align::XMinYMax);
        assert!(aspect.slice);
        assert_eq!(parse(""), svgtypes::AspectRatio::default());
        assert_eq!(
            parse("preserveAspectRatio='bogus'"),
            svgtypes::AspectRatio::default()
        );
    }

    #[test]
    fn absolute_dimensions_are_intrinsic() {
        let src = br#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="16" viewBox="0 0 48 32"></svg>"#;
        let svg = parse_svg_image(src, &super::default_svg_fonts()).unwrap();
        assert_eq!(svg.intrinsic_width(), Some(24.0));
        assert_eq!(svg.intrinsic_height(), Some(16.0));
    }

    #[test]
    fn percentage_dimensions_are_not_intrinsic() {
        let src = br#"<svg xmlns="http://www.w3.org/2000/svg" width="100%" height="50%" viewBox="0 0 200 100"></svg>"#;
        let svg = parse_svg_image(src, &super::default_svg_fonts()).unwrap();
        assert_eq!(svg.intrinsic_width(), None);
        assert_eq!(svg.intrinsic_height(), None);
    }

    #[test]
    fn unit_lengths_are_intrinsic() {
        let src = br#"<svg xmlns="http://www.w3.org/2000/svg" width="24px" height="1.5em" viewBox="0 0 48 32"></svg>"#;
        let svg = parse_svg_image(src, &super::default_svg_fonts()).unwrap();
        assert!(svg.intrinsic_width().is_some());
        assert!(svg.intrinsic_height().is_some());
    }

    #[test]
    fn non_numeric_dimensions_are_not_intrinsic() {
        let src = br#"<svg xmlns="http://www.w3.org/2000/svg" width="auto" height="foo" viewBox="0 0 200 100"></svg>"#;
        let svg = parse_svg_image(src, &super::default_svg_fonts()).unwrap();
        assert_eq!(svg.intrinsic_width(), None);
        assert_eq!(svg.intrinsic_height(), None);
    }
}

/// Creates an markup5ever::QualName.
/// Given a local name and an optional namespace
#[macro_export]
macro_rules! qual_name {
    ($local:tt $(, $ns:ident)?) => {
        $crate::QualName {
            prefix: None,
            ns: $crate::ns!($($ns)?),
            local: $crate::local_name!($local),
        }
    };
}
