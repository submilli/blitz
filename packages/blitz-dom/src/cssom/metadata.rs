//! CSSOM interface names and serialized rule attributes.
use super::*;
pub(super) fn css_rule_interface(rule: &CssRule) -> &'static str {
    match rule {
        CssRule::Style(_) => "CSSStyleRule",
        CssRule::Namespace(_) => "CSSNamespaceRule",
        CssRule::Import(_) => "CSSImportRule",
        CssRule::Media(_) => "CSSMediaRule",
        CssRule::CustomMedia(_) => "CSSCustomMediaRule",
        CssRule::Container(_) => "CSSContainerRule",
        CssRule::FontFace(_) => "CSSFontFaceRule",
        CssRule::FontFeatureValues(_) => "CSSFontFeatureValuesRule",
        CssRule::FontPaletteValues(_) => "CSSFontPaletteValuesRule",
        CssRule::CounterStyle(_) => "CSSCounterStyleRule",
        CssRule::Keyframes(_) => "CSSKeyframesRule",
        CssRule::Margin(_) => "CSSMarginRule",
        CssRule::Supports(_) => "CSSSupportsRule",
        CssRule::Page(_) => "CSSPageRule",
        CssRule::Property(_) => "CSSPropertyRule",
        CssRule::Document(_) => "CSSMozDocumentRule",
        CssRule::LayerBlock(_) => "CSSLayerBlockRule",
        CssRule::LayerStatement(_) => "CSSLayerStatementRule",
        CssRule::Scope(_) => "CSSScopeRule",
        CssRule::StartingStyle(_) => "CSSStartingStyleRule",
        CssRule::AppearanceBase(_) => "CSSAppearanceBaseRule",
        CssRule::PositionTry(_) => "CSSPositionTryRule",
        CssRule::NestedDeclarations(_) => "CSSNestedDeclarations",
        CssRule::ViewTransition(_) => "CSSViewTransitionRule",
    }
}

/// Join the CSS serializations of `items` with `", "`
fn comma_separated<'a, T: ToCss + 'a>(items: impl IntoIterator<Item = &'a T>) -> String {
    let mut css = CssStringWriter::new();
    for (i, item) in items.into_iter().enumerate() {
        if i > 0 {
            css.push_str(", ");
        }
        let _ = item.to_css(&mut style_traits::CssWriter::new(&mut css));
    }
    css
}

pub(super) fn rule_attributes(
    rule: &CssRule,
    guard: &SharedRwLockReadGuard,
) -> Vec<(&'static str, String)> {
    match rule {
        CssRule::Style(r) => {
            vec![("selectorText", r.read_with(guard).selectors.to_css_string())]
        }
        CssRule::Namespace(r) => vec![
            ("namespaceURI", r.url.to_string()),
            (
                "prefix",
                r.prefix.as_ref().map(|p| p.to_string()).unwrap_or_default(),
            ),
        ],
        CssRule::Import(r) => {
            let r = r.read_with(guard);
            vec![
                ("href", r.url.as_str().to_string()),
                (
                    "media",
                    r.stylesheet
                        .media(guard)
                        .map(|m| m.to_css_string())
                        .unwrap_or_default(),
                ),
            ]
        }
        CssRule::Media(r) => vec![(
            "conditionText",
            r.media_queries.read_with(guard).to_css_string(),
        )],
        CssRule::Supports(r) => vec![("conditionText", r.condition.to_css_string())],
        CssRule::Container(r) => vec![("conditionText", r.conditions.to_css_string())],
        CssRule::Page(r) => vec![("selectorText", r.read_with(guard).selectors.to_css_string())],
        CssRule::Keyframes(r) => vec![("name", r.read_with(guard).name.to_css_string())],
        CssRule::LayerBlock(r) => vec![(
            "name",
            r.name
                .as_ref()
                .map(|n| n.to_css_string())
                .unwrap_or_default(),
        )],
        CssRule::LayerStatement(r) => vec![("nameList", comma_separated(r.names.iter()))],
        CssRule::Property(r) => {
            let mut attrs = vec![("name", r.name.to_css_string())];
            for (attr, id) in [
                ("syntax", style::properties::property::DescriptorId::Syntax),
                (
                    "inherits",
                    style::properties::property::DescriptorId::Inherits,
                ),
                (
                    "initialValue",
                    style::properties::property::DescriptorId::InitialValue,
                ),
            ] {
                let mut css = CssStringWriter::new();
                let _ = r.descriptors.get(id, &mut css);
                attrs.push((attr, css));
            }
            attrs
        }
        CssRule::CounterStyle(r) => vec![("name", r.read_with(guard).name().to_css_string())],
        CssRule::FontFeatureValues(r) => {
            vec![("fontFamily", comma_separated(r.family_names.iter()))]
        }
        CssRule::FontPaletteValues(r) => vec![
            ("name", r.name.to_css_string()),
            ("fontFamily", comma_separated(r.family_names.iter())),
            (
                "basePalette",
                r.base_palette
                    .as_ref()
                    .map(|b| b.to_css_string())
                    .unwrap_or_default(),
            ),
            ("overrideColors", comma_separated(r.override_colors.iter())),
        ],
        CssRule::Margin(r) => vec![("name", format!("{:?}", r.rule_type))],
        CssRule::PositionTry(r) => vec![("name", r.read_with(guard).name.to_css_string())],
        _ => Vec::new(),
    }
}
