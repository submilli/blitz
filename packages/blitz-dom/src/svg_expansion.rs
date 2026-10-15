//! What an SVG document expands to while it is parsed and converted, known
//! before it is allocated.
//!
//! A few kilobytes of SVG can expand a thousandfold: DTD entities repeat
//! markup, usvg copies each `<use>` target (and converts every copy),
//! converts a marker once per vertex it marks and a paint server or effect
//! once per element it applies to, flattens each text character into glyph
//! outlines, and turns an arc into curves whose number grows with its
//! radius. [`entities`] measures the source an XML parser expands, and
//! [`weight`] what usvg converts the parsed document to, so that
//! `svg_bounds` reserves both from the decoded image budget, or refuses the
//! document, before parsing and converting it.
//!
//! The measures are upper bounds: they follow every reference usvg could
//! follow, apply every stylesheet rule to every element, and resolve lengths
//! against the largest font size and viewport an element could have.

mod curves;
mod entities;
mod graph;
mod lengths;
mod markup;
mod styles;
mod stylesheet_cost;
mod weight;

pub(crate) use curves::path_curves;
pub(crate) use entities::{ExpandedSource, expand_entities};
pub(crate) use weight::conversion;

/// The declarations the measure may keep per node of a document, on average:
/// past them, a few rules matching every element are refused rather than
/// held.
const MAX_DECLARATIONS_PER_NODE: usize = 16;

/// What measuring a parsed document holds per node of it: its element and
/// costs, and its share of kept declarations (32 bytes each).
// Include resolved filter-list costs and their memoized function counts.
const MEASURE_BYTES_PER_NODE: usize = 1280 + 32 * MAX_DECLARATIONS_PER_NODE;

/// What [`conversion`] holds while it measures `doc`: its nodes, and its
/// parsed stylesheets. Finding the latter, as the measure does too, holds
/// [`measure_scan_bytes`].
pub(crate) fn measure_bytes(doc: &usvg::roxmltree::Document) -> usize {
    nodes(doc)
        .saturating_mul(MEASURE_BYTES_PER_NODE)
        .saturating_add(styles::stylesheet_bytes(doc))
}

/// What [`measure_bytes`] holds while it scans `doc`'s stylesheets.
pub(crate) fn measure_scan_bytes(doc: &usvg::roxmltree::Document) -> usize {
    styles::stylesheet_scan_bytes(doc)
}

/// How many nodes `doc` has.
fn nodes(doc: &usvg::roxmltree::Document) -> usize {
    doc.descendants().count()
}

pub(crate) const SVG_NS: &str = "http://www.w3.org/2000/svg";
pub(crate) const XLINK_NS: &str = "http://www.w3.org/1999/xlink";

/// Nodes the XML parser may build for one document. usvg refuses documents
/// with more elements; the parse reservation usually refuses them first.
pub(crate) const MAX_XML_NODES: u32 = 1_000_000;

/// Element levels the XML parser may nest through: roxmltree recurses once
/// per level (about a kilobyte of stack each in a release build, ten in a
/// debug one). libxml2, Chrome's XML parser, refuses documents nested deeper
/// by default too (`xmlParserMaxDepth`).
pub(crate) const MAX_XML_DEPTH: usize = 256;

/// Elements nested in a document and through its references, and nested
/// across the documents one SVG image nests: usvg recurses once per level
/// (a few kilobytes of stack each), so pages must not choose the depth.
pub(crate) const MAX_NESTING: usize = 128;

#[cfg(test)]
mod tests;
