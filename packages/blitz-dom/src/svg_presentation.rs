//! SVG filter presentation attributes participate below author declarations.
use crate::{Node, node::NodeData};
use style::properties::{
    PropertyDeclaration, PropertyId, SourcePropertyDeclaration, parse_one_declaration_into,
};
use style::servo_arc::Arc;
use style::stylesheets::{CssRuleType, Origin, UrlExtraData};
use style_traits::ParsingMode;

impl Node {
    pub(crate) fn svg_filter_presentation_hints(&self) -> Vec<PropertyDeclaration> {
        let Some(element) = self.element_data() else {
            return Vec::new();
        };
        if element.name.ns.as_ref() != "http://www.w3.org/2000/svg" {
            return Vec::new();
        }
        let document = self.with(self.owner_document);
        let url = match &document.data {
            NodeData::Document(data) => data.metadata.url.clone(),
            _ => None,
        }
        .unwrap_or_else(|| url::Url::parse("about:blank").expect("static parser base"));
        let base = UrlExtraData(Arc::new(url));
        let mut result = Vec::new();
        for property in [
            "color",
            "flood-color",
            "flood-opacity",
            "lighting-color",
            "color-interpolation-filters",
        ] {
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
                ParsingMode::DEFAULT,
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
