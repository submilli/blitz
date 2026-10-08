//! Bounded CSS computation shared by canvas font and filter snapshots.

use style::applicable_declarations::CascadePriority;
use style::properties::declaration_block::Importance;
use style::properties::{
    CascadeMode, ComputedValues, FirstLineReparenting, PropertyDeclaration,
    PropertyDeclarationBlock, PropertyId, SourcePropertyDeclaration, apply_declarations,
    parse_one_declaration_into,
};
use style::rule_tree::{CascadeLevel, RuleCascadeFlags};
use style::servo_arc::Arc;
use style::shared_lock::StylesheetGuards;
use style::stylesheets::layer_rule::LayerOrder;
use style::stylesheets::{CssRuleType, Origin};
use style_traits::ParsingMode;

use crate::{BaseDocument, Node, NodeId};

impl BaseDocument {
    pub(crate) fn canvas_computed_style(
        &self,
        node: NodeId,
        property: &str,
        text: &str,
    ) -> Option<Arc<ComputedValues>> {
        let declarations = self.canvas_style_declarations(property, text)?;
        let inherited = self.get_node(node).and_then(|element| {
            if !element.flags.is_in_document() {
                return None;
            }
            element
                .primary_styles()
                .map(|style| style.clone())
                .or_else(|| self.resolve_undisplayed_style(node))
        });
        let fallback;
        let parent = match inherited.as_deref() {
            Some(style) => style,
            None => {
                fallback = self.compute_canvas_style(
                    &self.canvas_style_declarations("font", "10px sans-serif")?,
                    self.stylist.device().default_computed_values(),
                );
                &fallback
            }
        };
        let computed = self.compute_canvas_style(&declarations, parent);
        Some(computed)
    }

    pub(crate) fn canvas_style_declarations(
        &self,
        property: &str,
        text: &str,
    ) -> Option<PropertyDeclarationBlock> {
        self.canvas_style_declarations_for(self.root_node_id, property, text)
    }

    pub(crate) fn canvas_style_declarations_for(
        &self,
        node: NodeId,
        property: &str,
        text: &str,
    ) -> Option<PropertyDeclarationBlock> {
        if text.len() > 4096 || !crate::css_limits::allowed(text) {
            return None;
        }
        let mut source = SourcePropertyDeclaration::default();
        parse_one_declaration_into(
            &mut source,
            PropertyId::parse_enabled_for_all_content(property).ok()?,
            text,
            Origin::Author,
            &self.style_base_url(node),
            None,
            ParsingMode::DEFAULT,
            selectors::matching::QuirksMode::NoQuirks,
            CssRuleType::Style,
        )
        .ok()?;
        let mut block = PropertyDeclarationBlock::new();
        block.extend(source.drain(), Importance::Normal);
        if block.declarations().iter().any(|value| {
            matches!(
                value,
                PropertyDeclaration::WithVariables(_) | PropertyDeclaration::CSSWideKeyword(_)
            )
        }) {
            return None;
        }
        Some(block)
    }

    pub(crate) fn compute_canvas_style(
        &self,
        declarations: &PropertyDeclarationBlock,
        parent: &ComputedValues,
    ) -> Arc<ComputedValues> {
        let guard = self.guard.read();
        let guards = StylesheetGuards {
            author: &guard,
            ua_or_user: &guard,
        };
        let priority = CascadePriority::new(
            CascadeLevel::SAME_TREE_AUTHOR_NORMAL,
            LayerOrder::root(),
            RuleCascadeFlags::empty(),
        );
        apply_declarations::<&Node, _>(
            &self.stylist,
            None,
            self.stylist.rule_tree().root(),
            &guards,
            declarations
                .declarations()
                .iter()
                .map(|value| (value, priority)),
            Some(parent),
            Some(parent),
            FirstLineReparenting::No,
            &Default::default(),
            CascadeMode::Unvisited {
                visited_rules: None,
            },
            Default::default(),
            RuleCascadeFlags::empty(),
            None,
            &mut Default::default(),
            None,
            &mut Default::default(),
        )
    }
}
