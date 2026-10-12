//! The root size usvg is given for an SVG document, made to agree with
//! Chrome's reading of it.
//!
//! - usvg refuses a document whose root `width` or `height` resolves to an
//!   empty size (`Error::InvalidSize`). Chrome loads it with that natural
//!   dimension zero, and paints its content wherever the image is given a box
//!   (a sized `<img>`, a `background-size`, an `feImage`, a nested `<image>`).
//!   usvg is given `100%` instead, as if the dimension were missing.
//! - usvg also reads a prefixed `width` or `height` in the SVG, XLink or XML
//!   namespace, before an unprefixed one; Chrome ignores those. They are
//!   removed, so usvg's tree and the natural size come from one attribute.
//!
//! The declared dimension still decides the natural size (see
//! [`crate::node::SvgIntrinsicDimensions`]).

use std::ops::Range;
use std::sync::Arc;
use usvg::roxmltree;

/// The namespaces whose `width` and `height` usvg reads besides the
/// unprefixed ones.
const USVG_NAMESPACES: [&str; 3] = [
    "http://www.w3.org/2000/svg",
    "http://www.w3.org/1999/xlink",
    "http://www.w3.org/XML/1998/namespace",
];

/// Parse SVG source as usvg parses it, DTDs allowed.
pub(crate) fn parse_xml(text: &str) -> Result<roxmltree::Document<'_>, usvg::Error> {
    let options = roxmltree::ParsingOptions {
        allow_dtd: true,
        ..Default::default()
    };
    roxmltree::Document::parse_with_options(text, options).map_err(usvg::Error::ParsingFailed)
}

/// Whether a root dimension leaves the image empty: zero or negative in any
/// unit (usvg resolves `0%` to nothing too), including a length too small
/// for usvg's single precision.
pub(crate) fn is_empty_length(length: svgtypes::Length) -> bool {
    (length.number as f32) <= 0.0
}

/// The root's unprefixed `name` attribute, the one Chrome reads.
pub(crate) fn root_dimension<'a, 'input>(
    doc: &'a roxmltree::Document<'input>,
    name: &str,
) -> Option<roxmltree::Attribute<'a, 'input>> {
    doc.root_element()
        .attributes()
        .find(|attribute| attribute.name() == name && attribute.namespace().is_none())
}

/// `text` with the root size usvg should read: each empty `width` or
/// `height` replaced by `100%`, and the prefixed ones Chrome ignores removed;
/// `None` when usvg already reads it as Chrome does.
pub(crate) fn chrome_root_size(text: &str, doc: &roxmltree::Document) -> Option<String> {
    let mut edits: Vec<(Range<usize>, &str)> = Vec::new();
    for (name, missing) in [("width", "width=\"100%\""), ("height", "height=\"100%\"")] {
        if let Some(range) = empty_dimension(text, doc, name) {
            edits.push((range, missing));
        }
        edits.extend(ignored_dimensions(doc, name).map(|range| (range, "")));
    }
    if edits.is_empty() {
        return None;
    }
    edits.sort_by_key(|(range, _)| range.start);
    let mut resized = String::with_capacity(text.len() + 16);
    let mut end = 0;
    for (range, replacement) in edits {
        resized.push_str(text.get(end..range.start)?);
        resized.push_str(replacement);
        end = range.end;
    }
    resized.push_str(text.get(end..)?);
    Some(resized)
}

/// A nested document's `data` with [`chrome_root_size`], for usvg to parse
/// again after refusing it, or `None` when that would not help: not a type
/// usvg's `default_data_resolver` parses as SVG, not an XML document, or
/// nothing to change. The copy is transient and bounded by the nested source
/// bound, like an inflated SVGZ document.
pub(crate) fn nested_chrome_root_size(mime: &str, data: &[u8]) -> Option<Arc<Vec<u8>>> {
    if !matches!(mime, "image/svg+xml" | "text/plain") {
        return None;
    }
    let text = std::str::from_utf8(data).ok()?;
    let doc = parse_xml(text).ok()?;
    let resized = chrome_root_size(text, &doc)?;
    Some(Arc::new(resized.into_bytes()))
}

/// The source range of the root's unprefixed `name` attribute when it is an
/// empty length. A length usvg cannot parse is already missing to it.
fn empty_dimension(text: &str, doc: &roxmltree::Document, name: &str) -> Option<Range<usize>> {
    let attribute = root_dimension(doc, name)?;
    let length = attribute.value().parse::<svgtypes::Length>().ok()?;
    // `range` spans `name="value"` exactly (roxmltree's value range is
    // misplaced past 255 spaces around `=`).
    let range = attribute.range();
    let declared = text.get(range.clone())?;
    (is_empty_length(length) && declared.starts_with(name)).then_some(range)
}

/// The source ranges of the root's prefixed `name` attributes that usvg
/// reads and Chrome ignores.
fn ignored_dimensions<'a>(
    doc: &'a roxmltree::Document,
    name: &'a str,
) -> impl Iterator<Item = Range<usize>> + 'a {
    doc.root_element()
        .attributes()
        .filter(move |attribute| {
            attribute.name() == name
                && attribute
                    .namespace()
                    .is_some_and(|namespace| USVG_NAMESPACES.contains(&namespace))
        })
        .map(|attribute| attribute.range())
}

#[cfg(test)]
mod tests;
