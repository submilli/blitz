//! CSSOM stylesheet access (`document.styleSheets`, `CSSStyleSheet`,
//! `CSSRuleList`, `CSSRule` and friends) backed by stylo's stylesheet objects.
//!
//! Owner-node and path APIs remain convenient entry points, while `CssomSheet`
//! and `CssomRule` retain the actual stylesheet and rule allocations. Stable
//! handles survive index shifts, deletion, detachment, and source replacement.
//! An empty path denotes a sheet's top-level rule list.

use cssparser::{Parser, ParserInput};
use selectors::matching::QuirksMode;
use selectors::parser::{ParseRelative, SelectorList};
use style::font_face::FontFaceRule;
use style::parser::ParserContext;
use style::properties::font_face::DescriptorId as FontFaceDescriptorId;
use style::properties::{
    Importance, PropertyDeclarationBlock, PropertyId, SourcePropertyDeclaration,
    parse_one_declaration_into,
};
use style::selector_parser::SelectorParser;
use style::servo_arc::Arc as ServoArc;
use style::shared_lock::{Locked, SharedRwLockReadGuard, ToCssWithGuard};
use style::stylesheets::keyframes_rule::{Keyframe, KeyframesRule};
use style::stylesheets::{
    AllowImportRules, CssRule, CssRuleType, CssRuleTypes, CssRules, DocumentStyleSheet, Origin,
    RulesMutateError, StylesheetInDocument,
};
use style_traits::{CssStringWriter, ParsingMode, ToCss};
// For `SelectorList::to_css_string`
use cssparser::ToCss as _;

use blitz_traits::node_id::NodeId;

use crate::BaseDocument;
use crate::net::StylesheetLoader;

mod metadata;
mod mutations;
mod order;
use metadata::{css_rule_interface, rule_attributes};
mod retained;
#[cfg(test)]
mod sheet_tests;
mod sheets;
#[cfg(test)]
mod tests;
pub use retained::CssomRule;

/// Errors from CSSOM mutation operations, mirroring the `DOMException` names
/// which the corresponding JavaScript APIs throw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CssomError {
    /// The owner node has no associated stylesheet, or the rule path does not
    /// resolve to a rule (list).
    NotFound,
    /// `SyntaxError`
    Syntax,
    /// `IndexSizeError`
    IndexSize,
    /// `HierarchyRequestError`
    HierarchyRequest,
    /// `InvalidStateError`
    InvalidState,
    /// `NotSupportedError`
    NotSupported,
}

impl CssomError {
    /// The `DOMException` name corresponding to this error
    pub fn dom_exception_name(self) -> &'static str {
        match self {
            Self::NotFound => "NotFoundError",
            Self::Syntax => "SyntaxError",
            Self::IndexSize => "IndexSizeError",
            Self::HierarchyRequest => "HierarchyRequestError",
            Self::InvalidState => "InvalidStateError",
            Self::NotSupported => "NotSupportedError",
        }
    }
}

impl From<RulesMutateError> for CssomError {
    fn from(err: RulesMutateError) -> Self {
        match err {
            RulesMutateError::Syntax => Self::Syntax,
            RulesMutateError::IndexSize => Self::IndexSize,
            RulesMutateError::HierarchyRequest => Self::HierarchyRequest,
            RulesMutateError::InvalidState => Self::InvalidState,
        }
    }
}

/// A stylesheet retained independently of its owner node's attachment.
/// Only the originating document may inspect or mutate it: Stylo locks and
/// loading policy belong to that document. Attachment controls application.
#[derive(Clone)]
pub struct CssomSheet {
    document: usize,
    owner: NodeId,
    sheet: DocumentStyleSheet,
    constructed: bool,
    origin_clean: bool,
}

/// Detached sheets still parse import rules, but have no loading authority.
struct DetachedStylesheetLoader {
    url_data: style::stylesheets::UrlExtraData,
}
impl style::stylesheets::StylesheetLoader for DetachedStylesheetLoader {
    fn request_stylesheet(
        &self,
        url: style::values::CssUrl,
        source_location: style::values::SourceLocation,
        lock: &style::shared_lock::SharedRwLock,
        media: ServoArc<Locked<style::media_queries::MediaList>>,
        supports: Option<style::stylesheets::import_rule::ImportSupportsCondition>,
        layer: style::stylesheets::import_rule::ImportLayer,
    ) -> ServoArc<Locked<style::stylesheets::ImportRule>> {
        // An empty child retains the import media without granting loading authority.
        let sheet = style::stylesheets::Stylesheet::from_str(
            "",
            self.url_data.clone(),
            Origin::Author,
            media,
            lock.clone(),
            None,
            None,
            QuirksMode::NoQuirks,
            AllowImportRules::No,
        );
        ServoArc::new(lock.wrap(style::stylesheets::ImportRule {
            url,
            source_location,
            supports,
            layer,
            stylesheet: style::stylesheets::import_rule::ImportSheet::new(ServoArc::new(sheet)),
        }))
    }
}

/// A snapshot of the CSSOM-visible attributes of a rule
#[derive(Debug, Clone)]
pub struct CssRuleInfo {
    /// The name of the CSSOM interface for the rule (e.g. `"CSSStyleRule"`)
    pub interface: &'static str,
    /// The legacy `CSSRule.type` constant (0 for rules without one)
    pub rule_type: u16,
    /// `CSSRule.cssText`
    pub css_text: String,
    /// Whether the rule has a child rule list (`cssRules`)
    pub has_child_rules: bool,
    /// Whether the rule has a `style` declaration block
    pub has_style: bool,
    /// Additional interface-specific string attributes (e.g. `selectorText`)
    pub attributes: Vec<(&'static str, String)>,
}

/// A rule list that CSSOM rule paths can index into
#[derive(Clone)]
enum RuleList {
    Css(ServoArc<Locked<CssRules>>),
    Keyframes(ServoArc<Locked<KeyframesRule>>),
}

/// A rule resolved from a CSSOM rule path
#[derive(Clone)]
enum RuleHandle {
    Rule(CssRule),
    Keyframe(ServoArc<Locked<Keyframe>>),
}

/// The declarations of a rule (`CSSRule.style`)
enum DeclarationTarget {
    Block {
        block: ServoArc<Locked<PropertyDeclarationBlock>>,
        rule_type: CssRuleType,
    },
    FontFace(ServoArc<Locked<FontFaceRule>>),
}

fn child_rule_list(rule: &CssRule, guard: &SharedRwLockReadGuard) -> Option<RuleList> {
    let rules = match rule {
        CssRule::Style(r) => r.read_with(guard).rules.clone()?,
        CssRule::Media(r) => r.rules.clone(),
        CssRule::Supports(r) => r.rules.clone(),
        CssRule::Container(r) => r.rules.clone(),
        CssRule::Page(r) => r.read_with(guard).rules.clone(),
        CssRule::Document(r) => r.rules.clone(),
        CssRule::LayerBlock(r) => r.rules.clone(),
        CssRule::Scope(r) => r.rules.clone(),
        CssRule::StartingStyle(r) => r.rules.clone(),
        CssRule::Keyframes(r) => return Some(RuleList::Keyframes(r.clone())),
        _ => return None,
    };
    Some(RuleList::Css(rules))
}

fn declaration_target(
    handle: &RuleHandle,
    guard: &SharedRwLockReadGuard,
) -> Option<DeclarationTarget> {
    let (block, rule_type) = match handle {
        RuleHandle::Keyframe(k) => (k.read_with(guard).block.clone(), CssRuleType::Keyframe),
        RuleHandle::Rule(rule) => match rule {
            CssRule::Style(r) => (r.read_with(guard).block.clone(), CssRuleType::Style),
            CssRule::Page(r) => (r.read_with(guard).block.clone(), CssRuleType::Page),
            CssRule::Margin(r) => (r.block.clone(), CssRuleType::Margin),
            CssRule::NestedDeclarations(r) => (
                r.read_with(guard).block.clone(),
                CssRuleType::NestedDeclarations,
            ),
            CssRule::PositionTry(r) => (r.read_with(guard).block.clone(), CssRuleType::PositionTry),
            CssRule::FontFace(r) => return Some(DeclarationTarget::FontFace(r.clone())),
            _ => return None,
        },
    };
    Some(DeclarationTarget::Block { block, rule_type })
}

impl BaseDocument {
    /// Retain the current stylesheet object, even if its owner is later removed
    /// or its text is replaced. A replacement yields a distinct handle.
    pub fn retain_stylesheet(&self, owner: NodeId) -> Option<CssomSheet> {
        Some(CssomSheet {
            document: self.id(),
            owner,
            sheet: self.active_stylesheet(owner)?.clone(),
            constructed: false,
            origin_clean: self
                .get_node(owner)?
                .element_data()?
                .stylesheet_origin_clean,
        })
    }

    /// Whether this exact sheet is still the owner's active stylesheet.
    pub fn stylesheet_is_current(&self, handle: &CssomSheet) -> bool {
        handle.document == self.id()
            && self
                .active_stylesheet(handle.owner)
                .is_some_and(|sheet| ServoArc::ptr_eq(&sheet.0, &handle.sheet.0))
    }

    fn active_stylesheet(&self, owner: NodeId) -> Option<&DocumentStyleSheet> {
        if let Some(root) = self.containing_shadow_root(owner) {
            return self
                .get_node(root)?
                .shadow_styles
                .as_ref()?
                .sheets
                .get(&owner);
        }
        self.nodes_to_stylesheet.get(&owner)
    }

    fn retained_sheet<'a>(&self, handle: &'a CssomSheet) -> Option<&'a DocumentStyleSheet> {
        (handle.document == self.id()).then_some(&handle.sheet)
    }

    /// The owner nodes (`<style>` / `<link>` elements) of the document's author
    /// stylesheets (`document.styleSheets`).
    ///
    /// Ordered by current DOM tree position, including after moves and slot reuse.
    pub fn stylesheet_owner_nodes(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.stylesheet_owners_in_tree_order(self.nodes_to_stylesheet.keys().copied())
            .into_iter()
    }

    /// Whether `node_id` currently owns a stylesheet
    pub fn node_has_stylesheet(&self, node_id: NodeId) -> bool {
        self.nodes_to_stylesheet.contains_key(&node_id)
    }

    /// A counter which changes whenever the set of stylesheets (or the
    /// stylesheet object owned by some node) changes. CSSOM wrappers use it to
    /// detect that rule data cached on the script side is stale.
    pub fn stylesheet_generation(&self) -> u64 {
        self.stylesheet_generation
    }

    fn stylesheet_loader(&self) -> StylesheetLoader {
        StylesheetLoader {
            tx: self.tx.clone(),
            doc_id: self.id(),
            net_provider: self.net_provider.clone(),
            shell_provider: self.shell_provider.clone(),
            abort_signal: self.abort_signal.clone(),
            import_depth: 0,
        }
    }

    /// Resolve `path` to the rule it denotes. The rule's ancestors (outermost
    /// first) are passed to `on_ancestor` as they are traversed.
    fn resolve_rule(
        sheet: &DocumentStyleSheet,
        guard: &SharedRwLockReadGuard,
        path: &[usize],
        on_ancestor: impl FnMut(&CssRule),
    ) -> Option<RuleHandle> {
        let (index, parent_path) = path.split_last()?;
        let list = Self::resolve_rule_list(sheet, guard, parent_path, on_ancestor)?;
        Some(match list {
            RuleList::Css(rules) => RuleHandle::Rule(rules.read_with(guard).0.get(*index)?.clone()),
            RuleList::Keyframes(keyframes) => {
                RuleHandle::Keyframe(keyframes.read_with(guard).keyframes.get(*index)?.clone())
            }
        })
    }

    /// Resolve `path` to the rule list owned by the rule it denotes (or the
    /// stylesheet's top-level rule list for an empty path). The ancestor rules
    /// of that list (outermost first, including the rule at `path` itself) are
    /// passed to `on_ancestor` as they are traversed.
    fn resolve_rule_list(
        sheet: &DocumentStyleSheet,
        guard: &SharedRwLockReadGuard,
        path: &[usize],
        mut on_ancestor: impl FnMut(&CssRule),
    ) -> Option<RuleList> {
        let mut list = RuleList::Css(sheet.contents(guard).rules.clone());
        for &index in path {
            let RuleList::Css(rules) = &list else {
                return None;
            };
            let rules = rules.read_with(guard);
            let rule = rules.0.get(index)?;
            let child_list = child_rule_list(rule, guard)?;
            on_ancestor(rule);
            list = child_list;
        }
        Some(list)
    }

    /// [`Self::resolve_rule_list`], also collecting the ancestor rules
    fn resolve_rule_list_with_ancestors(
        sheet: &DocumentStyleSheet,
        guard: &SharedRwLockReadGuard,
        path: &[usize],
    ) -> Option<(Vec<CssRule>, RuleList)> {
        let mut ancestors = Vec::with_capacity(path.len());
        let list =
            Self::resolve_rule_list(sheet, guard, path, |rule| ancestors.push(rule.clone()))?;
        Some((ancestors, list))
    }

    /// The number of rules in the rule list at `path` (`CSSRuleList.length`)
    pub fn stylesheet_rule_count(&self, node_id: NodeId, path: &[usize]) -> Option<usize> {
        let handle = self.retain_stylesheet(node_id)?;
        self.retained_stylesheet_rule_count(&handle, path)
    }

    /// The same operation on a retained sheet; foreign-document handles fail.
    pub fn retained_stylesheet_rule_count(
        &self,
        handle: &CssomSheet,
        path: &[usize],
    ) -> Option<usize> {
        let sheet = self.retained_sheet(handle)?;
        let guard = self.guard.read();
        let list = Self::resolve_rule_list(sheet, &guard, path, |_| {})?;
        Some(match list {
            RuleList::Css(rules) => rules.read_with(&guard).0.len(),
            RuleList::Keyframes(keyframes) => keyframes.read_with(&guard).keyframes.len(),
        })
    }

    /// The CSSOM-visible attributes of the rule at `path`
    pub fn stylesheet_rule_info(&self, node_id: NodeId, path: &[usize]) -> Option<CssRuleInfo> {
        let handle = self.retain_stylesheet(node_id)?;
        self.retained_stylesheet_rule_info(&handle, path)
    }

    /// The same operation on a retained sheet; foreign-document handles fail.
    pub fn retained_stylesheet_rule_info(
        &self,
        handle: &CssomSheet,
        path: &[usize],
    ) -> Option<CssRuleInfo> {
        let sheet = self.retained_sheet(handle)?;
        let guard = self.guard.read();
        let handle = Self::resolve_rule(sheet, &guard, path, |_| {})?;
        self.rule_info(&handle, &guard)
    }

    pub fn stylesheet_rule_style_css_text(
        &self,
        node_id: NodeId,
        path: &[usize],
    ) -> Option<String> {
        let handle = self.retain_stylesheet(node_id)?;
        self.retained_stylesheet_rule_style_css_text(&handle, path)
    }

    pub fn retained_stylesheet_rule_style_css_text(
        &self,
        sheet: &CssomSheet,
        path: &[usize],
    ) -> Option<String> {
        self.retained_rule_style_css_text(&self.retain_stylesheet_rule(sheet, path)?)
    }

    pub fn stylesheet_rule_style_property_names(
        &self,
        node_id: NodeId,
        path: &[usize],
    ) -> Option<Vec<String>> {
        let handle = self
            .retain_stylesheet(node_id)
            .and_then(|sheet| self.retain_stylesheet_rule(&sheet, path))?;
        self.retained_rule_style_property_names(&handle)
    }

    pub fn stylesheet_rule_style_get_property(
        &self,
        node_id: NodeId,
        path: &[usize],
        property: &str,
    ) -> Option<(String, bool)> {
        let handle = self
            .retain_stylesheet(node_id)
            .and_then(|sheet| self.retain_stylesheet_rule(&sheet, path))?;
        self.retained_rule_style_get_property(&handle, property)
    }

    pub fn stylesheet_rule_style_set_property(
        &mut self,
        node_id: NodeId,
        path: &[usize],
        property: &str,
        value: &str,
        important: bool,
    ) -> Result<(), CssomError> {
        let handle = self
            .retain_stylesheet(node_id)
            .and_then(|sheet| self.retain_stylesheet_rule(&sheet, path))
            .ok_or(CssomError::NotFound)?;
        self.retained_rule_style_set_property(&handle, property, value, important)
    }

    pub fn stylesheet_rule_style_remove_property(
        &mut self,
        node_id: NodeId,
        path: &[usize],
        property: &str,
    ) -> Result<String, CssomError> {
        let handle = self
            .retain_stylesheet(node_id)
            .and_then(|sheet| self.retain_stylesheet_rule(&sheet, path))
            .ok_or(CssomError::NotFound)?;
        self.retained_rule_style_remove_property(&handle, property)
    }

    pub fn stylesheet_rule_set_selector_text(
        &mut self,
        node_id: NodeId,
        path: &[usize],
        selectors: &str,
    ) -> Result<(), CssomError> {
        let handle = self
            .retain_stylesheet(node_id)
            .and_then(|sheet| self.retain_stylesheet_rule(&sheet, path))
            .ok_or(CssomError::NotFound)?;
        self.retained_rule_set_selector_text(&handle, selectors)
    }

    pub fn stylesheet_rule_style_set_css_text(
        &mut self,
        node_id: NodeId,
        path: &[usize],
        css: &str,
    ) -> Result<(), CssomError> {
        let handle = self
            .retain_stylesheet(node_id)
            .and_then(|sheet| self.retain_stylesheet_rule(&sheet, path))
            .ok_or(CssomError::NotFound)?;
        self.retained_rule_style_set_css_text(&handle, css)
    }
}
