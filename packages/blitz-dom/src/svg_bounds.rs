//! Parsing the SVG documents a page supplies, as images or inline `<svg>`,
//! within bounds: how far SVGZ may inflate, what nested images may add, what
//! the XML parser allocates as it expands the source, what usvg allocates to
//! build and convert the parsed document, and what that document costs to
//! keep. Each is reserved from the decoded image budget before it is
//! allocated (see [`crate::svg_expansion`]).
use crate::DecodedImageBudget;
use crate::image_budget::Reservation;
use crate::node::{SvgImageData, SvgParseCollectors};
use crate::svg_expansion::{
    ExpandedSource, MAX_NESTING, conversion, expand_entities, measure_bytes, measure_scan_bytes,
};
use crate::svg_nested_rasters::{NestedRasterDecoder, RasterPurpose};
use crate::svg_subresources::{BodyDigest, NestedImages, SubresourceRecorder};
use std::sync::{Arc, Mutex, PoisonError};
use usvg::roxmltree;

/// The SVG source an SVGZ image may inflate to, and that the documents one SVG
/// image nests may total. A few hundred kilobytes of gzip inflate a
/// thousandfold; usvg's own `decompress_svgz` has no bound. Uncompressed
/// responses are bounded by the host's response limit, which this matches.
pub(crate) const MAX_SVG_SOURCE_BYTES: u64 = 16 * 1024 * 1024;

/// Images an SVG document may nest (including within nested SVG images).
const MAX_NESTED_SVG_IMAGES: usize = 16;

/// Matching repeated across converted copies, independently of the preflight's
/// own allowance. A small XML tree may instantiate a large styled subtree.
const MAX_SVG_CONVERSION_WORK: usize = 2_000_000;

/// Why [`parse_svg_image`] produced no SVG document.
#[derive(Debug)]
pub(crate) enum SvgRefusal {
    /// Parsing or keeping it would pass the decoded image budget.
    Budget,
    /// The data is not an SVG document usvg accepts. Its error is dropped:
    /// parser errors can quote the document.
    Invalid,
    /// The document expands past what may be built: its entities amplify it
    /// past libxml2's bound, or its elements and references nest deeper than
    /// usvg may recurse or form a cycle.
    Expansion,
}

/// Parse an SVG image, charged to `budget`. Page-supplied SVG never reaches the
/// network or the filesystem: nested images may only be `data:` URLs, a
/// bounded number of them, and nested rasters are decoded once, charged to
/// `budget` (see [`crate::svg_nested_rasters`]). SVGZ inflates within its
/// bound, and what parsing allocates up front is reserved while it runs, for
/// nested documents too.
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
    parse(source, fonts, budget, shown, RasterPurpose::Paint)
}

/// [`parse_svg_image`] for a document whose geometry is measured and never
/// painted: its nested rasters are not decoded.
pub(crate) fn parse_svg_geometry(
    source: &[u8],
    fonts: &crate::SvgFontDb,
    budget: &DecodedImageBudget,
) -> Result<SvgImageData, SvgRefusal> {
    parse(
        source,
        fonts,
        budget,
        NestedImages::All,
        RasterPurpose::Measure,
    )
}

fn parse(
    source: &[u8],
    fonts: &crate::SvgFontDb,
    budget: &DecodedImageBudget,
    shown: NestedImages,
    purpose: RasterPurpose,
) -> Result<SvgImageData, SvgRefusal> {
    let inflated;
    let source = if is_svgz(source) {
        inflated = inflate_svgz_once(source).ok_or(SvgRefusal::Invalid)?;
        inflated.as_slice()
    } else {
        source
    };
    let text = std::str::from_utf8(source).map_err(|_| SvgRefusal::Invalid)?;
    let expanded = expand_entities(text).ok_or(SvgRefusal::Expansion)?;
    let svg = {
        let _parsing = reserve_parse(budget, text, expanded).ok_or(SvgRefusal::Budget)?;
        let collectors = SvgParseCollectors::new(shown, budget, purpose);
        let options = svg_options(fonts, budget, &collectors);
        SvgImageData::parse_recording(source, &options, &collectors)?
    };
    if !budget.admit_svg(&svg) {
        return Err(SvgRefusal::Budget);
    }
    Ok(svg)
}

/// usvg options without ambient resource access. usvg's default string resolver
/// reads paths from the filesystem; nested SVG documents forward only the data
/// resolver, so its bounds cover every nesting level: the number of nested
/// images, their source bytes, what parsing and converting one allocates, the
/// levels it nests, and the rasters' headers (they decode once the tree is
/// parsed). The resolver refuses, and notes as omitted, the `<image>` bodies
/// the recorder of `collectors` does not admit, and scans each nested
/// document with it.
fn svg_options(
    fonts: &crate::SvgFontDb,
    budget: &DecodedImageBudget,
    collectors: &SvgParseCollectors,
) -> usvg::Options<'static> {
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    let nested = Arc::new(AtomicUsize::new(0));
    let sources = Arc::new(AtomicU64::new(0));
    let budget = budget.clone();
    let recorder = collectors.recorder.clone();
    let conversions = collectors.conversions.clone();
    let rasters = collectors.rasters.clone();
    let decode = usvg::ImageHrefResolver::default_data_resolver();
    usvg::Options {
        fontdb: fonts.database.clone(),
        font_resolver: fonts.resolver(),
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_image: Some(Box::new(move |image| rasters.resolve(image))),
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
                if matches!(
                    mime,
                    "image/png" | "image/jpeg" | "image/jpg" | "image/gif" | "image/webp"
                ) {
                    let kind =
                        decode(mime, data, options).unwrap_or_else(NestedRasterDecoder::broken);
                    return if kind.raster_size().is_none() {
                        Some(NestedRasterDecoder::broken())
                    } else {
                        NestedRasterDecoder::admits(&kind).then_some(kind)
                    };
                }
                if mime == "text/plain" && image::guess_format(&data).is_ok() {
                    let kind =
                        decode(mime, data, options).unwrap_or_else(NestedRasterDecoder::broken);
                    return if kind.raster_size().is_none() {
                        Some(NestedRasterDecoder::broken())
                    } else {
                        NestedRasterDecoder::admits(&kind).then_some(kind)
                    };
                }
                if !matches!(mime, "image/svg+xml" | "text/plain") {
                    return Some(NestedRasterDecoder::broken());
                }
                let text = match std::str::from_utf8(&data) {
                    Ok(text) => text,
                    Err(_) => return Some(NestedRasterDecoder::broken()),
                };
                let _held = match prepare_nested(text, &budget, &recorder, &conversions) {
                    Ok(held) => held,
                    Err(NestedFailure::Invalid) => return Some(NestedRasterDecoder::broken()),
                    Err(NestedFailure::Refused) => return None,
                };
                decode_nested_svg(mime, &data, options)
            }),
            resolve_string: Box::new(|_, _| None),
        },
        ..Default::default()
    }
}

/// Preserve resource refusals across the nested-image resolver. A malformed
/// image may retry Chrome's root-size fallback or show a broken icon; a work
/// limit must neither retry conversion nor become that placeholder.
fn decode_nested_svg(mime: &str, data: &[u8], options: &usvg::Options) -> Option<usvg::ImageKind> {
    match usvg::Tree::from_data_nested(data, options) {
        Ok(tree) => Some(usvg::ImageKind::SVG(tree)),
        Err(usvg::Error::TextPathWorkLimitReached) => None,
        Err(_) => {
            let Some(resized) = crate::svg_root_size::nested_chrome_root_size(mime, data) else {
                return Some(NestedRasterDecoder::broken());
            };
            match usvg::Tree::from_data_nested(&resized, options) {
                Ok(tree) => Some(usvg::ImageKind::SVG(tree)),
                Err(usvg::Error::TextPathWorkLimitReached) => None,
                Err(_) => Some(NestedRasterDecoder::broken()),
            }
        }
    }
}

/// Bound a nested document as [`parse`] bounds an image: refuse amplifying
/// entities, reserve what parsing holds (returned, to hold while usvg parses
/// and converts it), scan its images with `recorder`, and reserve what converting it
/// allocates until the image is parsed. The levels it nests count while
/// the returned guard lives. A resource refusal never becomes a broken icon.
fn prepare_nested(
    text: &str,
    budget: &DecodedImageBudget,
    recorder: &SubresourceRecorder,
    conversions: &ConversionReservations,
) -> Result<(Reservation, Levels), NestedFailure> {
    let expanded = expand_entities(text).ok_or(NestedFailure::Refused)?;
    let parsing = reserve_parse(budget, text, expanded).ok_or(NestedFailure::Refused)?;
    let doc = crate::svg_root_size::parse_xml(text).map_err(|_| NestedFailure::Invalid)?;
    // Before usvg parses the document and resolves its images, so they are
    // known to be `<image>` elements.
    recorder.scan_nested(&doc);
    let levels = conversions
        .reserve(&doc)
        .map_err(|_| NestedFailure::Refused)?;
    Ok((parsing, levels))
}

enum NestedFailure {
    Invalid,
    Refused,
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

/// Reserve from `budget` what parsing `text` allocates: up front for its
/// markup, and as it expands its entities, for the nodes, attributes and
/// text they add (see [`crate::svg_expansion`]). Held while it parses.
/// `None` when it does not fit.
fn reserve_parse(
    budget: &DecodedImageBudget,
    text: &str,
    expanded: ExpandedSource,
) -> Option<Reservation> {
    let nodes = expanded.tags.saturating_mul(PARSE_BYTES_PER_NODE);
    let attributes = expanded
        .attributes
        .saturating_mul(PARSE_BYTES_PER_ATTRIBUTE);
    let added_text = expanded.bytes.saturating_sub(text.len());
    budget.reserve(nodes.saturating_add(attributes).saturating_add(added_text))
}

/// What the conversions of one SVG image and the documents it nests hold
/// from the budget until the image is parsed: usvg converts each nested
/// document while it converts the image, and keeps it in the image's tree.
#[derive(Clone)]
pub(crate) struct ConversionReservations(Arc<Mutex<Held>>);

/// The levels of a document being converted, until dropped.
pub(crate) struct Levels {
    reservations: ConversionReservations,
    levels: usize,
}

impl Drop for Levels {
    fn drop(&mut self) {
        let mut held = self.reservations.lock();
        held.levels = held.levels.saturating_sub(self.levels);
    }
}

struct Held {
    budget: DecodedImageBudget,
    reservations: Vec<Reservation>,
    /// The levels of the documents being converted: usvg converts a nested
    /// document in the middle of its `<image>`'s, recursing further.
    levels: usize,
}

impl ConversionReservations {
    pub(crate) fn new(budget: &DecodedImageBudget) -> Self {
        Self(Arc::new(Mutex::new(Held {
            budget: budget.clone(),
            reservations: Vec::new(),
            levels: 0,
        })))
    }

    /// Reserve what usvg allocates to convert `doc`, held until the image is
    /// parsed, or refuse it. Measuring it holds its own reservation meanwhile.
    /// Its levels count while the returned guard lives: until usvg has
    /// converted it, within the document nesting it.
    pub(crate) fn reserve(&self, doc: &roxmltree::Document) -> Result<Levels, SvgRefusal> {
        let budget = self.lock().budget.clone();
        let scanning = budget
            .reserve(measure_scan_bytes(doc))
            .ok_or(SvgRefusal::Budget)?;
        let measuring = budget
            .reserve(measure_bytes(doc))
            .ok_or(SvgRefusal::Budget)?;
        // The measure scans the stylesheets again for usvg's share.
        let conversion = conversion(doc).ok_or(SvgRefusal::Expansion)?;
        drop((scanning, measuring));
        let mut held = self.lock();
        let levels = held.levels.saturating_add(conversion.levels);
        if levels > MAX_NESTING {
            return Err(SvgRefusal::Expansion);
        }
        let reservation = held
            .budget
            .reserve(conversion.bytes)
            .ok_or(SvgRefusal::Budget)?;
        if conversion.work > MAX_SVG_CONVERSION_WORK {
            return Err(SvgRefusal::Expansion);
        }
        held.levels = levels;
        held.reservations.push(reservation);
        Ok(Levels {
            reservations: self.clone(),
            levels: conversion.levels,
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Held> {
        // Only plain data is behind the lock; a panic leaves it usable.
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// What retaining a parsed SVG document costs, estimated from its nodes
/// (including clip paths, masks, patterns, filters and gradient stops), path
/// points, flattened text, the paths text follows and nested documents. A
/// nested raster costs its encoded body here; its decode is charged on its
/// own (see [`crate::svg_nested_rasters`]).
pub(crate) fn retained_bytes(tree: &usvg::Tree) -> usize {
    use crate::svg_walk::{Visit, walk};
    const BYTES_PER_NODE: usize = 1024;
    const BYTES_PER_POINT: usize = 16;
    const BYTES_PER_STOP: usize = 64;
    let mut bytes = 0usize;
    let mut add = |more: usize| bytes = bytes.saturating_add(more);
    walk(tree, |visit| match visit {
        Visit::Tree(tree) => {
            add(BYTES_PER_NODE);
            let primitives = tree.filters().iter().flat_map(|filter| filter.primitives());
            add(primitives.count().saturating_mul(BYTES_PER_NODE));
            let linear = tree.linear_gradients().iter().map(|g| g.stops().len());
            let radial = tree.radial_gradients().iter().map(|g| g.stops().len());
            for stops in linear.chain(radial) {
                add(BYTES_PER_NODE.saturating_add(stops.saturating_mul(BYTES_PER_STOP)));
            }
        }
        Visit::Node(node) => {
            add(BYTES_PER_NODE);
            match node {
                usvg::Node::Path(path) => {
                    add(path.data().points().len().saturating_mul(BYTES_PER_POINT));
                }
                usvg::Node::Image(image) => match image.kind() {
                    usvg::ImageKind::JPEG(data)
                    | usvg::ImageKind::PNG(data)
                    | usvg::ImageKind::GIF(data)
                    | usvg::ImageKind::WEBP(data)
                    | usvg::ImageKind::Broken(data) => add(data.len()),
                    usvg::ImageKind::SVG(_) => {}
                },
                // A text path keeps the shape it follows; its flattened
                // glyphs are walked as nodes.
                usvg::Node::Text(text) => {
                    for chunk in text.chunks() {
                        if let usvg::TextFlow::Path(path) = chunk.text_flow() {
                            add(path.path().points().len().saturating_mul(BYTES_PER_POINT));
                        }
                    }
                }
                usvg::Node::Group(_) => {}
            }
        }
    });
    bytes
}

#[cfg(test)]
mod tests;
