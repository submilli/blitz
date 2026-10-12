//! The declarations usvg gives each element: its presentation attributes,
//! the rules of the document's stylesheets that match it, and its `style`
//! attribute.
//!
//! usvg copies each declaration from a stylesheet or a `style` attribute
//! into a string of its own on every element it builds, `<use>` copies
//! included, so a long declaration matched by many elements costs its
//! length for each of them. Stylesheets are read and matched as usvg reads
//! them (`resolve_css` and `parse_svg_element` in its `svgtree`).

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

/// The stylesheets of one document.
pub(super) struct Styles<'a> {
    sheet: simplecss::StyleSheet<'a>,
}

/// The declarations of the properties the measure reads that one element is
/// given; a declaration later in the cascade does not hide an earlier one,
/// so that every value usvg may read is seen.
#[derive(Default)]
pub(super) struct Declarations<'a> {
    list: Vec<(&'a str, &'a str)>,
    /// What usvg allocates to copy the stylesheet and `style` declarations.
    pub copied_bytes: usize,
}

impl<'a> Styles<'a> {
    /// The rules of `doc`'s `<style>` elements of type `text/css` (or none).
    pub(super) fn of(doc: &'a roxmltree::Document) -> Self {
        let mut sheet = simplecss::StyleSheet::new();
        for node in doc.descendants().filter(|n| n.has_tag_name("style")) {
            if node
                .attribute("type")
                .is_some_and(|kind| kind != "text/css")
            {
                continue;
            }
            if let Some(text) = node.text() {
                sheet.parse_more(text);
            }
        }
        Self { sheet }
    }

    /// The declarations `node` is given, or `None` once more than `allowance`
    /// would be kept.
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
        let element = XmlNode(node);
        for rule in &self.sheet.rules {
            if !rule.selector.matches(&element) {
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
struct XmlNode<'a, 'input>(Node<'a, 'input>);

impl simplecss::Element for XmlNode<'_, '_> {
    fn parent_element(&self) -> Option<Self> {
        self.0.parent_element().map(XmlNode)
    }

    fn prev_sibling_element(&self) -> Option<Self> {
        self.0.prev_sibling_element().map(XmlNode)
    }

    fn has_local_name(&self, local_name: &str) -> bool {
        self.0.tag_name().name() == local_name
    }

    fn attribute_matches(&self, local_name: &str, operator: simplecss::AttributeOperator) -> bool {
        self.0
            .attribute(local_name)
            .is_some_and(|value| operator.matches(value))
    }

    fn pseudo_class_matches(&self, class: simplecss::PseudoClass) -> bool {
        match class {
            simplecss::PseudoClass::FirstChild => self.prev_sibling_element().is_none(),
            _ => false,
        }
    }
}
