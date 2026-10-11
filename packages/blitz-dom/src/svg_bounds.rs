//! Parsing the SVG documents a page supplies, as images or inline `<svg>`,
//! within bounds: how far SVGZ may inflate, what nested images may add, what
//! the XML parser allocates up front, and what a parsed document costs to
//! keep. What parsing expands a source to (DTD entities, `<use>` copies,
//! flattened text) is not yet reserved before it is allocated.
use crate::DecodedImageBudget;
use crate::image_budget::Reservation;
use crate::node::SvgImageData;
use crate::svg_subresources::{BodyDigest, NestedImages, SubresourceRecorder};
use std::sync::Arc;

/// The SVG source an SVGZ image may inflate to, and that the documents one SVG
/// image nests may total. A few hundred kilobytes of gzip inflate a
/// thousandfold; usvg's own `decompress_svgz` has no bound. Uncompressed
/// responses are bounded by the host's response limit, which this matches.
pub(crate) const MAX_SVG_SOURCE_BYTES: u64 = 16 * 1024 * 1024;

/// Images an SVG document may nest (including within nested SVG images).
const MAX_NESTED_SVG_IMAGES: usize = 16;

/// Decoded bytes of the raster images one SVG document may nest. The renderer
/// decodes them itself, so their headers are checked against this up front.
/// The decoded image budget charges them twice (each paint decodes and copies
/// them), so this is a quarter of it.
const MAX_NESTED_SVG_IMAGE_BYTES: u64 = 32 * 1024 * 1024;

/// Why [`parse_svg_image`] produced no SVG document.
#[derive(Debug)]
pub(crate) enum SvgRefusal {
    /// Parsing or keeping it would pass the decoded image budget.
    Budget,
    /// The data is not an SVG document usvg accepts. Its error is dropped:
    /// parser errors can quote the document.
    Invalid,
}

/// Parse an SVG image, charged to `budget`. Page-supplied SVG never reaches the
/// network or the filesystem: nested images may only be `data:` URLs, a
/// bounded number of them. SVGZ inflates within its bound, and what parsing
/// allocates up front is reserved while it runs, for nested documents too.
pub(crate) fn parse_svg_image(
    source: &[u8],
    fonts: &crate::SvgFontDb,
    budget: &DecodedImageBudget,
) -> Result<SvgImageData, SvgRefusal> {
    parse_svg_image_showing(source, fonts, budget, NestedImages::All)
}

/// [`parse_svg_image`], showing only the `shown` images of its `<image>`
/// elements (see [`crate::svg_subresources`]).
pub(crate) fn parse_svg_image_showing(
    source: &[u8],
    fonts: &crate::SvgFontDb,
    budget: &DecodedImageBudget,
    shown: NestedImages,
) -> Result<SvgImageData, SvgRefusal> {
    let inflated;
    let source = if is_svgz(source) {
        inflated = inflate_svgz_once(source).ok_or(SvgRefusal::Invalid)?;
        inflated.as_slice()
    } else {
        source
    };
    let svg = {
        let _parsing = reserve_parse(budget, source).ok_or(SvgRefusal::Budget)?;
        let options = |recorder: &SubresourceRecorder| svg_options(fonts, budget, recorder);
        SvgImageData::parse_recording(source, shown, options).map_err(|_| SvgRefusal::Invalid)?
    };
    if !budget.admit_svg(&svg) {
        return Err(SvgRefusal::Budget);
    }
    Ok(svg)
}

/// usvg options without ambient resource access. usvg's default string resolver
/// reads paths from the filesystem; nested SVG documents forward only the data
/// resolver, so its bounds cover every nesting level: the number of nested
/// images, their source bytes, what parsing one allocates up front, and the
/// decoded size of rasters, which stay encoded until a renderer decodes them.
/// The resolver refuses, and notes as omitted, the `<image>` bodies `recorder`
/// does not admit, and scans each nested document with it.
fn svg_options(
    fonts: &crate::SvgFontDb,
    budget: &DecodedImageBudget,
    recorder: &SubresourceRecorder,
) -> usvg::Options<'static> {
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    let nested = Arc::new(AtomicUsize::new(0));
    let sources = Arc::new(AtomicU64::new(0));
    let bytes = Arc::new(AtomicU64::new(0));
    let budget = budget.clone();
    let recorder = recorder.clone();
    let decode = usvg::ImageHrefResolver::default_data_resolver();
    usvg::Options {
        fontdb: Arc::clone(fonts),
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: Box::new(move |mime, data, options| {
                let body = BodyDigest::of(&data);
                if !recorder.admits(body)
                    || nested.fetch_add(1, Ordering::Relaxed) >= MAX_NESTED_SVG_IMAGES
                {
                    return None;
                }
                // usvg would inflate a nested SVGZ document without a bound.
                let data = if is_svgz(&data) {
                    Arc::new(inflate_svgz_once(&data)?)
                } else {
                    data
                };
                let size = data.len() as u64;
                if sources.fetch_add(size, Ordering::Relaxed) + size > MAX_SVG_SOURCE_BYTES {
                    return None;
                }
                let kind = {
                    let _parsing = reserve_parse(&budget, &data)?;
                    // Before usvg parses a nested document and resolves its
                    // images, so they are known to be `<image>` elements.
                    recorder.scan_nested(&data);
                    decode(mime, data, options)?
                };
                let encoded = match &kind {
                    usvg::ImageKind::JPEG(data)
                    | usvg::ImageKind::PNG(data)
                    | usvg::ImageKind::GIF(data)
                    | usvg::ImageKind::WEBP(data) => data,
                    usvg::ImageKind::SVG(_) => return Some(kind),
                };
                let decoded = crate::image_decode::bounded_rgba_bytes(encoded)?;
                let total = bytes.fetch_add(decoded, Ordering::Relaxed) + decoded;
                (total <= MAX_NESTED_SVG_IMAGE_BYTES).then_some(kind)
            }),
            resolve_string: Box::new(|_, _| None),
        },
        ..Default::default()
    }
}

/// Whether `data` starts with the gzip magic bytes, as usvg detects SVGZ.
fn is_svgz(data: &[u8]) -> bool {
    data.starts_with(&[0x1f, 0x8b])
}

/// Inflate SVGZ `data` within [`MAX_SVG_SOURCE_BYTES`], stopping before it
/// allocates more. `None` when it is malformed, too large, or gzip again: usvg
/// would inflate a further layer itself, without a bound.
fn inflate_svgz_once(data: &[u8]) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut inflated = Vec::new();
    flate2::read::GzDecoder::new(data)
        .take(MAX_SVG_SOURCE_BYTES + 1)
        .read_to_end(&mut inflated)
        .ok()?;
    let fits = inflated.len() as u64 <= MAX_SVG_SOURCE_BYTES;
    (fits && !is_svgz(&inflated)).then_some(inflated)
}

/// Bytes roxmltree allocates up front for each `<` (a node) and each `=` (an
/// attribute) in a source, before parsing it, with room for usvg's own
/// intermediate tree of the same nodes.
const PARSE_BYTES_PER_NODE: usize = 192;
const PARSE_BYTES_PER_ATTRIBUTE: usize = 128;

/// Reserve from `budget` what parsing `source` allocates before any limit of
/// the parser applies; held while it parses. `None` when it does not fit.
fn reserve_parse(budget: &DecodedImageBudget, source: &[u8]) -> Option<Reservation> {
    let count = |byte: u8| source.iter().filter(|&&b| b == byte).count();
    let nodes = count(b'<').saturating_mul(PARSE_BYTES_PER_NODE);
    let attributes = count(b'=').saturating_mul(PARSE_BYTES_PER_ATTRIBUTE);
    budget.reserve(nodes.saturating_add(attributes))
}

/// What retaining a parsed SVG document costs, estimated from its nodes
/// (including clip paths, masks, patterns, filters and gradient stops), path
/// points, flattened text and nested documents. A nested raster stays encoded
/// in the tree, but the renderer decodes it and copies the decode on every
/// paint, so it costs twice its decoded size.
pub(crate) fn retained_bytes(tree: &usvg::Tree) -> usize {
    const BYTES_PER_NODE: usize = 1024;
    const BYTES_PER_POINT: usize = 16;
    const BYTES_PER_STOP: usize = 64;
    let mut bytes = 0usize;
    let mut add = |more: usize| bytes = bytes.saturating_add(more);
    let mut trees = vec![tree];
    let mut groups: Vec<&usvg::Group> = Vec::new();
    while let Some(tree) = trees.pop() {
        add(BYTES_PER_NODE);
        groups.push(tree.root());
        groups.extend(tree.clip_paths().iter().map(|clip| clip.root()));
        groups.extend(tree.masks().iter().map(|mask| mask.root()));
        groups.extend(tree.patterns().iter().map(|pattern| pattern.root()));
        for primitive in tree.filters().iter().flat_map(|filter| filter.primitives()) {
            add(BYTES_PER_NODE);
            if let usvg::filter::Kind::Image(image) = primitive.kind() {
                groups.push(image.root());
            }
        }
        let linear = tree.linear_gradients().iter().map(|g| g.stops().len());
        let radial = tree.radial_gradients().iter().map(|g| g.stops().len());
        for stops in linear.chain(radial) {
            add(BYTES_PER_NODE.saturating_add(stops.saturating_mul(BYTES_PER_STOP)));
        }
        while let Some(group) = groups.pop() {
            for node in group.children() {
                add(BYTES_PER_NODE);
                match node {
                    usvg::Node::Group(child) => groups.push(child),
                    usvg::Node::Path(path) => {
                        add(path.data().points().len().saturating_mul(BYTES_PER_POINT));
                    }
                    usvg::Node::Text(text) => groups.push(text.flattened()),
                    usvg::Node::Image(image) => match image.kind() {
                        usvg::ImageKind::SVG(nested) => trees.push(nested),
                        usvg::ImageKind::JPEG(data)
                        | usvg::ImageKind::PNG(data)
                        | usvg::ImageKind::GIF(data)
                        | usvg::ImageKind::WEBP(data) => add(nested_raster_bytes(data)),
                    },
                }
            }
        }
    }
    bytes
}

/// A nested raster's decode and its renderer copy; its encoded size when its
/// header is unreadable, as the renderer then fails to decode it.
fn nested_raster_bytes(encoded: &[u8]) -> usize {
    crate::image_decode::bounded_rgba_bytes(encoded)
        .and_then(|rgba| usize::try_from(rgba).ok())
        .map_or(encoded.len(), |rgba| rgba.saturating_mul(2))
}

#[cfg(test)]
mod tests;
