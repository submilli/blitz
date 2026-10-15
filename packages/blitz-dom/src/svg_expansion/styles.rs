//! The declarations usvg gives each element: its presentation attributes,
//! the rules of the document's stylesheets that match it, and its `style`
//! attribute.
//!
//! usvg copies each declaration from a stylesheet or a `style` attribute
//! into a string of its own on every element it builds, `<use>` copies
//! included, so a long declaration matched by many elements costs its
//! length for each of them. Stylesheets are read and matched as usvg reads
//! them (`resolve_css` and `parse_svg_element` in its `svgtree`).

use std::cell::Cell;
use usvg::roxmltree::{self, Node};

use super::SVG_NS;

/// What usvg allocates for one declaration it copies, besides its value: the
/// attribute entry and the string's header.
const BYTES_PER_COPIED_DECLARATION: usize = 64;
/// The properties the measure reads: those naming elements, the font size
/// relative lengths resolve against, and the lists usvg copies per element.
const READ: [&str; 11] = [
    "fill",
    "stroke",
    "marker-start",
    "marker-mid",
    "marker-end",
    "clip-path",
    "mask",
    "filter",
    "font-size",
    "stroke-dasharray",
    "font-family",
];
/// What usvg sets for a `font` shorthand: it resets twelve longhands first,
/// then sets up to six.
const FONT_LONGHANDS: usize = 18;

pub(super) const MAX_MATCH_WORK: usize = 2_000_000;

/// The stylesheets of one document.
pub(super) struct Styles<'a> {
    sheet: simplecss::StyleSheet<'a>,
    remaining: Cell<usize>,
}

/// The declarations of the properties the measure reads that one element is
/// given; a declaration later in the cascade does not hide an earlier one,
/// so that every value usvg may read is seen.
#[derive(Default)]
pub(super) struct Declarations<'a> {
    list: Vec<(&'a str, &'a str)>,
    /// What usvg allocates to copy the stylesheet and `style` declarations.
    pub copied_bytes: usize,
    /// Matching work repeated when usvg builds a copy of this XML element.
    pub match_work: usize,
}

impl<'a> Styles<'a> {
    /// The rules of `doc`'s `<style>` elements of type `text/css` (or none).
    /// Refuses excessive parsing, sorting or selector recursion before matching.
    pub(super) fn of(doc: &'a roxmltree::Document) -> Option<Self> {
        let mut bytes = 0usize;
        let mut sheets = 0usize;
        for css in stylesheets(doc) {
            bytes = bytes.saturating_add(css.len());
            sheets = sheets.saturating_add(1);
            // simplecss recursively skips comments in its selector tokenizer.
            // Count even comments inside strings: this is an admission bound,
            // not a second CSS parser.
            if css.matches("/*").count() > super::MAX_NESTING {
                return None;
            }
        }
        // parse_more recomputes specificity and sorts all accumulated rules,
        // even for an empty later sheet. Bound those repeated source visits.
        if bytes.saturating_mul(sheets) > MAX_MATCH_WORK {
            return None;
        }
        let mut sheet = simplecss::StyleSheet::new();
        for text in stylesheets(doc) {
            sheet.parse_more(text);
        }
        // simplecss recurses once per combinator. Its public serialization
        // exposes every combinator; counting whitespace inside attributes too
        // is deliberately conservative. Bound subselector scans as well.
        for rule in &sheet.rules {
            let selector = rule.selector.to_string();
            let levels = selector
                .bytes()
                .filter(|b| b.is_ascii_whitespace() || matches!(b, b'>' | b'+'))
                .count();
            if selector.len() > 4096 || levels > super::MAX_NESTING {
                return None;
            }
        }
        Some(Self {
            sheet,
            remaining: Cell::new(MAX_MATCH_WORK),
        })
    }

    /// The declarations `node` is given. Consumes the document's shared
    /// matching allowance through the matcher's immutable callbacks; refuses
    /// once it is exhausted or more than `allowance` declarations would be kept.
    pub(super) fn declarations(
        &self,
        node: Node<'a, '_>,
        allowance: usize,
    ) -> Option<Declarations<'a>> {
        let mut declarations = Declarations::default();
        for attribute in node.attributes() {
            if matches!(attribute.namespace(), None | Some(SVG_NS)) {
                declarations.keep(attribute.name(), attribute.value());
            }
        }
        declarations.within(allowance)?;
        let before = self.remaining.get();
        let element = XmlNode {
            node,
            remaining: &self.remaining,
        };
        for rule in &self.sheet.rules {
            element.charge(1);
            let matched = rule.selector.matches(&element);
            if self.remaining.get() == 0 {
                return None;
            }
            if !matched {
                continue;
            }
            for declaration in &rule.declarations {
                declarations.copy(declaration.name, declaration.value);
                declarations.within(allowance)?;
            }
        }
        if let Some(style) = node.attribute("style") {
            for declaration in simplecss::DeclarationTokenizer::from(style) {
                declarations.copy(declaration.name, declaration.value);
                declarations.within(allowance)?;
            }
        }
        declarations.match_work = before - self.remaining.get();
        Some(declarations)
    }
}

impl<'a> Declarations<'a> {
    /// `Some` while at most `allowance` declarations are kept.
    fn within(&self, allowance: usize) -> Option<()> {
        (self.len() <= allowance).then_some(())
    }

    /// How many declarations are kept.
    pub(super) fn len(&self) -> usize {
        self.list.len()
    }

    /// Every declared `(name, value)`, shorthands as the longhands usvg sets.
    pub(super) fn iter(&self) -> impl Iterator<Item = (&'a str, &'a str)> + '_ {
        self.list.iter().copied()
    }

    /// The values declared for `name`.
    pub(super) fn values(&self, name: &'a str) -> impl Iterator<Item = &'a str> + '_ {
        self.iter()
            .filter(move |(declared, _)| *declared == name)
            .map(|(_, value)| value)
    }

    /// Keep a declaration of a property the measure reads.
    fn keep(&mut self, name: &'a str, value: &'a str) {
        if READ.contains(&name) {
            self.list.push((name, value));
        }
    }

    /// A declaration usvg copies, with the longhands it sets for a shorthand.
    fn copy(&mut self, name: &'a str, value: &'a str) {
        let copies = match name {
            "marker" => {
                for longhand in ["marker-start", "marker-mid", "marker-end"] {
                    self.keep(longhand, value);
                }
                3
            }
            "font" => {
                // Only the size is read here; usvg parses it from the value.
                if let Ok(font) = svgtypes::FontShorthand::from_str(value) {
                    self.keep("font-size", font.font_size);
                    self.keep("font-family", font.font_family);
                }
                FONT_LONGHANDS
            }
            _ => {
                self.keep(name, value);
                1
            }
        };
        let bytes = BYTES_PER_COPIED_DECLARATION.saturating_add(value.len());
        self.copied_bytes = self
            .copied_bytes
            .saturating_add(bytes.saturating_mul(copies));
    }
}

/// What parsing `doc`'s stylesheets holds, as usvg and the measure each do
/// (see [`super::stylesheet_cost`]).
pub(super) fn stylesheet_bytes(doc: &roxmltree::Document) -> usize {
    stylesheets(doc)
        .map(super::stylesheet_cost::parsed_bytes)
        .fold(0, usize::saturating_add)
}

/// What [`stylesheet_bytes`] holds while it scans `doc`'s stylesheets.
pub(super) fn stylesheet_scan_bytes(doc: &roxmltree::Document) -> usize {
    stylesheets(doc)
        .map(|css| {
            css.len()
                .saturating_mul(super::stylesheet_cost::SCAN_BYTES_PER_BYTE)
        })
        .max()
        .unwrap_or(0)
}

/// The text of `doc`'s `<style>` elements of type `text/css` (or none), as
/// usvg reads them (`resolve_css`).
fn stylesheets<'a>(doc: &'a roxmltree::Document) -> impl Iterator<Item = &'a str> {
    doc.descendants()
        .filter(|node| node.has_tag_name("style"))
        .filter(|node| node.attribute("type").is_none_or(|kind| kind == "text/css"))
        .filter_map(|node| node.text())
}

/// An element as usvg matches selectors against it.
struct XmlNode<'b, 'a, 'input> {
    node: Node<'a, 'input>,
    remaining: &'b Cell<usize>,
}

impl XmlNode<'_, '_, '_> {
    fn charge(&self, work: usize) -> bool {
        self.remaining
            .set(self.remaining.get().saturating_sub(work));
        self.remaining.get() != 0
    }
}

impl simplecss::Element for XmlNode<'_, '_, '_> {
    fn parent_element(&self) -> Option<Self> {
        if !self.charge(1) {
            return None;
        }
        self.node.parent_element().map(|node| Self {
            node,
            remaining: self.remaining,
        })
    }

    fn prev_sibling_element(&self) -> Option<Self> {
        if !self.charge(1) {
            return None;
        }
        let mut previous = self.node.prev_sibling();
        while let Some(node) = previous {
            if !self.charge(1) {
                return None;
            }
            if node.is_element() {
                return Some(Self {
                    node,
                    remaining: self.remaining,
                });
            }
            previous = node.prev_sibling();
        }
        None
    }

    fn has_local_name(&self, local_name: &str) -> bool {
        self.charge(local_name.len().saturating_add(1)) && self.node.tag_name().name() == local_name
    }

    fn attribute_matches(&self, local_name: &str, operator: simplecss::AttributeOperator) -> bool {
        for attribute in self.node.attributes() {
            if !self.charge(
                attribute
                    .name()
                    .len()
                    .saturating_add(attribute.value().len())
                    .saturating_add(1),
            ) {
                return false;
            }
            if attribute.name() == local_name && attribute.namespace().is_none() {
                return operator.matches(attribute.value());
            }
        }
        false
    }

    fn pseudo_class_matches(&self, class: simplecss::PseudoClass) -> bool {
        if !self.charge(1) {
            return false;
        }
        match class {
            simplecss::PseudoClass::FirstChild => self.prev_sibling_element().is_none(),
            _ => false,
        }
    }
}
