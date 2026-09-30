//! Admission before recursive Stylo parsing. Use cssparser tokens so strings,
//! comments, escapes and unquoted URLs have exactly the parser's block semantics.

/// Shared ceiling for page CSS. This also bounds recursive matching, evaluation,
/// serialization and destruction of parsed CSS trees.
pub const MAX_CSS_NESTING: usize = style::custom_properties::MAX_SUBSTITUTED_VALUE_NESTING;

pub(crate) fn allowed(css: &str) -> bool {
    allowed_with_ancestors(css, 0)
}

/// Whether `css` stays within the shared ceiling when it is parsed below
/// `ancestors` already-existing CSS rule blocks.
pub(crate) fn allowed_with_ancestors(css: &str, ancestors: usize) -> bool {
    ancestors <= MAX_CSS_NESTING && !nesting_exceeds(css, MAX_CSS_NESTING - ancestors)
}

/// Sheets and declaration blocks rejected at admission behave as empty input.
pub(crate) fn bounded(css: &str) -> &str {
    if allowed(css) { css } else { "" }
}

/// Whether `css` nests blocks (functions, parentheses, square and curly
/// brackets) more than `limit` deep, as cssparser tokenizes it: nothing in
/// a comment, string or unquoted `url(` counts. The walk recurses at most
/// `limit` levels; cssparser skips a block it is not asked to enter
/// without recursing.
pub(crate) fn nesting_exceeds(css: &str, limit: usize) -> bool {
    style::custom_properties::nesting_exceeds(css, limit)
}
