//! CSS capability queries use the same Stylo parsers as author styles.

use crate::BaseDocument;
use cssparser::{Parser, ParserInput};
use selectors::matching::QuirksMode;
use style::parser::ParserContext;
use style::stylesheets::supports_rule::parse_condition_or_declaration;
use style::stylesheets::{CssRuleType, Origin};
use style_traits::ParsingMode;

impl BaseDocument {
    /// Evaluate a `@supports` condition (e.g. `(display: grid)`,
    /// `selector(:hover)`) or a bare declaration (e.g. `display: grid`), as
    /// used by the CSSOM `CSS.supports(conditionText)` API. Returns `false`
    /// for unparseable conditions.
    pub fn css_supports_condition(&self, condition: &str) -> bool {
        if !crate::css_limits::allowed(condition) {
            return false;
        }
        let mut input = ParserInput::new(condition);
        let mut parser = Parser::new(&mut input);
        let Ok(condition) = parser.parse_entirely(parse_condition_or_declaration) else {
            return false;
        };

        let url_data = self.url.url_extra_data();
        let context = ParserContext::new(
            Origin::Author,
            &url_data,
            Some(CssRuleType::Style),
            ParsingMode::DEFAULT,
            QuirksMode::NoQuirks,
            Default::default(),
            None,
            None,
            Default::default(),
        );
        condition.eval(&context)
    }

    /// Test a CSSOM property/value pair. Unlike a condition's declaration,
    /// the name is literal and the value must not contain a priority.
    pub fn css_supports_property(&self, property: &str, value: &str) -> bool {
        !value.is_empty() && self.css_declaration_is_valid(property, value)
    }
}

#[cfg(test)]
mod tests;
