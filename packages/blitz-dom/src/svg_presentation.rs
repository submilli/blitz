//! SVG presentation attributes participate below author declarations
//! (<https://svgwg.org/svg2-draft/styling.html#PresentationAttributes>).
use crate::{Node, node::NodeData};
use style::properties::{
    PropertyDeclaration, PropertyId, SourcePropertyDeclaration, parse_one_declaration_into,
};
use style::servo_arc::Arc;
use style::stylesheets::{CssRuleType, Origin, UrlExtraData};
use style_traits::ParsingMode;

impl Node {
    /// The URL presentation attributes resolve against: the owner document's.
    pub(crate) fn svg_presentation_base(&self) -> url::Url {
        let document = self.with(self.owner_document);
        match &document.data {
            NodeData::Document(data) => data.metadata.url.clone(),
            _ => None,
        }
        .unwrap_or_else(|| url::Url::parse("about:blank").expect("static parser base"))
    }

    pub(crate) fn svg_presentation_hints(&self) -> Vec<PropertyDeclaration> {
        let Some(element) = self.element_data() else {
            return Vec::new();
        };
        if element.name.ns.as_ref() != "http://www.w3.org/2000/svg" {
            return Vec::new();
        }
        let base = UrlExtraData(Arc::new(self.svg_presentation_base()));
        let mut result = Vec::new();
        for property in PRESENTATION_PROPERTIES {
            let Some(value) = element.attr_dom(markup5ever::LocalName::from(property)) else {
                continue;
            };
            if value.utf16_len_bound() > 4096 || !crate::css_limits::allowed(value.as_str_lossy()) {
                continue;
            }
            let Ok(id) = PropertyId::parse_enabled_for_all_content(property) else {
                continue;
            };
            let mut declarations = SourcePropertyDeclaration::default();
            if parse_one_declaration_into(
                &mut declarations,
                id,
                value.as_str_lossy(),
                Origin::Author,
                &base,
                None,
                // SVG attributes take unitless lengths (`font-size="8"`).
                ParsingMode::ALLOW_UNITLESS_LENGTH,
                selectors::matching::QuirksMode::NoQuirks,
                CssRuleType::Style,
            )
            .is_ok()
            {
                let mut block = style::properties::PropertyDeclarationBlock::new();
                block.extend(declarations.drain(), style::properties::Importance::Normal);
                result.extend(block.declarations().iter().cloned());
            }
        }
        result
    }
}

/// The presentation attributes mapped to CSS.
pub(crate) const PRESENTATION_PROPERTIES: [&str; 29] = [
    "color",
    "flood-color",
    "flood-opacity",
    "lighting-color",
    "color-interpolation-filters",
    "fill",
    "fill-opacity",
    "fill-rule",
    "stroke",
    "stroke-width",
    "stroke-opacity",
    "stroke-dasharray",
    "stroke-dashoffset",
    "stroke-linecap",
    "stroke-linejoin",
    "stroke-miterlimit",
    "stop-color",
    "stop-opacity",
    "paint-order",
    "marker-start",
    "marker-mid",
    "marker-end",
    "text-anchor",
    "display",
    "visibility",
    "font-family",
    "font-size",
    "font-style",
    "font-weight",
];
