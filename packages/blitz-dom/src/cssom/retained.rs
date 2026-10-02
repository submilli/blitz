//! Stable rule objects retain Stylo allocations, never mutable list indices.
use super::*;

/// A CSS rule independent of its position or continued membership in its sheet.
/// Retention is reference-counted; dropping the wrapper releases the rule tree.
#[derive(Clone)]
pub struct CssomRule {
    pub(super) sheet: CssomSheet,
    pub(super) rule: RuleHandle,
    pub(super) ancestors: Vec<CssRule>,
}
impl CssomRule {
    /// Opaque identity valid while this handle is retained.
    pub fn identity(&self) -> usize {
        rule_identity(&self.rule)
    }
}
fn rule_identity(rule: &RuleHandle) -> usize {
    match rule {
        RuleHandle::Keyframe(value) => (&**value as *const _) as usize,
        RuleHandle::Rule(rule) => match rule {
            CssRule::Style(value) => (&**value as *const _) as usize,
            CssRule::Namespace(value) => (&**value as *const _) as usize,
            CssRule::Import(value) => (&**value as *const _) as usize,
            CssRule::Media(value) => (&**value as *const _) as usize,
            CssRule::CustomMedia(value) => (&**value as *const _) as usize,
            CssRule::Container(value) => (&**value as *const _) as usize,
            CssRule::FontFace(value) => (&**value as *const _) as usize,
            CssRule::FontFeatureValues(value) => (&**value as *const _) as usize,
            CssRule::FontPaletteValues(value) => (&**value as *const _) as usize,
            CssRule::CounterStyle(value) => (&**value as *const _) as usize,
            CssRule::Keyframes(value) => (&**value as *const _) as usize,
            CssRule::Margin(value) => (&**value as *const _) as usize,
            CssRule::Supports(value) => (&**value as *const _) as usize,
            CssRule::Page(value) => (&**value as *const _) as usize,
            CssRule::Property(value) => (&**value as *const _) as usize,
            CssRule::Document(value) => (&**value as *const _) as usize,
            CssRule::LayerBlock(value) => (&**value as *const _) as usize,
            CssRule::LayerStatement(value) => (&**value as *const _) as usize,
            CssRule::Scope(value) => (&**value as *const _) as usize,
            CssRule::StartingStyle(value) => (&**value as *const _) as usize,
            CssRule::AppearanceBase(value) => (&**value as *const _) as usize,
            CssRule::PositionTry(value) => (&**value as *const _) as usize,
            CssRule::NestedDeclarations(value) => (&**value as *const _) as usize,
            CssRule::ViewTransition(value) => (&**value as *const _) as usize,
        },
    }
}
impl BaseDocument {
    /// Retain the rule currently at a path. Its identity survives list edits.
    pub fn retain_stylesheet_rule(&self, sheet: &CssomSheet, path: &[usize]) -> Option<CssomRule> {
        let native = self.retained_sheet(sheet)?;
        let guard = self.guard.read();
        let mut ancestors = Vec::new();
        let rule = Self::resolve_rule(native, &guard, path, |rule| ancestors.push(rule.clone()))?;
        Some(CssomRule {
            sheet: sheet.clone(),
            rule,
            ancestors,
        })
    }
    /// Whether this rule remains in the retained sheet's rule tree.
    pub fn retained_rule_is_attached(&self, handle: &CssomRule) -> bool {
        let Some(sheet) = self.retained_sheet(&handle.sheet) else {
            return false;
        };
        let guard = self.guard.read();
        let mut list = RuleList::Css(sheet.contents(&guard).rules.clone());
        for rule in &handle.ancestors {
            if !list_contains(&list, &RuleHandle::Rule(rule.clone()), &guard) {
                return false;
            }
            let Some(children) = child_rule_list(rule, &guard) else {
                return false;
            };
            list = children;
        }
        list_contains(&list, &handle.rule, &guard)
    }
    /// Whether the rule still belongs to its immediate grouping rule.
    /// Removing the group from a sheet does not sever its child relationships.
    pub fn retained_rule_has_parent(&self, handle: &CssomRule) -> bool {
        if self.retained_sheet(&handle.sheet).is_none() {
            return false;
        }
        let Some(parent) = handle.ancestors.last() else {
            return false;
        };
        let guard = self.guard.read();
        child_rule_list(parent, &guard)
            .is_some_and(|list| list_contains(&list, &handle.rule, &guard))
    }
    pub fn retained_rule_info(&self, handle: &CssomRule) -> Option<CssRuleInfo> {
        self.retained_sheet(&handle.sheet)?;
        self.rule_info(&handle.rule, &self.guard.read())
    }
    pub(super) fn rule_info(
        &self,
        handle: &RuleHandle,
        guard: &SharedRwLockReadGuard,
    ) -> Option<CssRuleInfo> {
        let mut css_text = CssStringWriter::new();
        let has_style = declaration_target(handle, guard).is_some();
        Some(match handle.clone() {
            RuleHandle::Rule(rule) => {
                let _ = rule.to_css(guard, &mut css_text);
                CssRuleInfo {
                    interface: css_rule_interface(&rule),
                    rule_type: rule.rule_type() as u16,
                    css_text,
                    has_child_rules: child_rule_list(&rule, guard).is_some(),
                    has_style,
                    attributes: rule_attributes(&rule, guard),
                }
            }
            RuleHandle::Keyframe(keyframe) => {
                let keyframe = keyframe.read_with(guard);
                let _ = keyframe.to_css(guard, &mut css_text);
                CssRuleInfo {
                    interface: "CSSKeyframeRule",
                    rule_type: CssRuleType::Keyframe as u16,
                    css_text,
                    has_child_rules: false,
                    has_style,
                    attributes: vec![("keyText", keyframe.selector.to_css_string())],
                }
            }
        })
    }
    fn retained_declaration_target(
        &self,
        handle: &CssomRule,
        guard: &SharedRwLockReadGuard,
    ) -> Option<DeclarationTarget> {
        self.retained_sheet(&handle.sheet)?;
        declaration_target(&handle.rule, guard)
    }
    pub fn retained_rule_count(&self, handle: &CssomRule) -> Option<usize> {
        self.retained_sheet(&handle.sheet)?;
        let guard = self.guard.read();
        let RuleHandle::Rule(rule) = &handle.rule else {
            return None;
        };
        Some(match child_rule_list(rule, &guard)? {
            RuleList::Css(rules) => rules.read_with(&guard).0.len(),
            RuleList::Keyframes(rule) => rule.read_with(&guard).keyframes.len(),
        })
    }
    pub fn retain_child_rule(&self, handle: &CssomRule, index: usize) -> Option<CssomRule> {
        self.retained_sheet(&handle.sheet)?;
        let guard = self.guard.read();
        let RuleHandle::Rule(parent) = &handle.rule else {
            return None;
        };
        let rule = match child_rule_list(parent, &guard)? {
            RuleList::Css(rules) => RuleHandle::Rule(rules.read_with(&guard).0.get(index)?.clone()),
            RuleList::Keyframes(rule) => {
                RuleHandle::Keyframe(rule.read_with(&guard).keyframes.get(index)?.clone())
            }
        };
        let mut ancestors = handle.ancestors.clone();
        ancestors.push(parent.clone());
        Some(CssomRule {
            sheet: handle.sheet.clone(),
            rule,
            ancestors,
        })
    }
    /// Insert within this grouping rule, including after the group is detached.
    pub fn retained_rule_insert_rule(
        &mut self,
        handle: &CssomRule,
        text: &str,
        index: usize,
    ) -> Result<usize, CssomError> {
        let (ancestors, list) = self.retained_child_list(handle)?;
        self.cssom_insert_rule(
            &handle.sheet,
            ancestors,
            list,
            text,
            index,
            self.stylesheet_is_current(&handle.sheet) && self.retained_rule_is_attached(handle),
        )
    }
    pub fn retained_rule_delete_rule(
        &mut self,
        handle: &CssomRule,
        index: usize,
    ) -> Result<(), CssomError> {
        let (_, list) = self.retained_child_list(handle)?;
        self.cssom_delete_rule(&handle.sheet, list, index)
    }
    fn retained_child_list(
        &self,
        handle: &CssomRule,
    ) -> Result<(Vec<CssRule>, RuleList), CssomError> {
        self.retained_sheet(&handle.sheet)
            .ok_or(CssomError::NotFound)?;
        let RuleHandle::Rule(rule) = &handle.rule else {
            return Err(CssomError::NotSupported);
        };
        let list = child_rule_list(rule, &self.guard.read()).ok_or(CssomError::NotSupported)?;
        let mut ancestors = handle.ancestors.clone();
        ancestors.push(rule.clone());
        Ok((ancestors, list))
    }
    pub fn retained_rule_style_css_text(&self, handle: &CssomRule) -> Option<String> {
        self.retained_sheet(&handle.sheet)?;
        let guard = self.guard.read();
        let rule = &handle.rule;
        let target = declaration_target(rule, &guard)?;
        let mut css = CssStringWriter::new();
        match target {
            DeclarationTarget::Block { block, .. } => {
                let _ = block.read_with(&guard).to_css(&mut css);
            }
            DeclarationTarget::FontFace(rule) => {
                let _ = rule
                    .read_with(&guard)
                    .descriptors
                    .to_css(&mut style_traits::CssWriter::new(&mut css));
                // `Descriptors::to_css` emits a trailing space after each declaration
                let trimmed = css.trim_end().len();
                css.truncate(trimmed);
            }
        }
        Some(css)
    }

    pub fn retained_rule_style_property_names(&self, handle: &CssomRule) -> Option<Vec<String>> {
        let guard = self.guard.read();
        let target = self.retained_declaration_target(handle, &guard)?;
        Some(match target {
            DeclarationTarget::Block { block, .. } => block
                .read_with(&guard)
                .declarations()
                .iter()
                .map(|decl| decl.id().name().into_owned())
                .collect(),
            DeclarationTarget::FontFace(rule) => {
                let descriptors = &rule.read_with(&guard).descriptors;
                (0..descriptors.len())
                    .filter_map(|i| descriptors.at(i))
                    .map(|id| id.name().to_string())
                    .collect()
            }
        })
    }

    pub fn retained_rule_style_get_property(
        &self,
        handle: &CssomRule,
        property: &str,
    ) -> Option<(String, bool)> {
        let guard = self.guard.read();
        let target = self.retained_declaration_target(handle, &guard)?;
        let mut css = CssStringWriter::new();
        let important = match target {
            DeclarationTarget::Block { block, .. } => {
                let Ok(property_id) = PropertyId::parse_enabled_for_all_content(property) else {
                    return Some((String::new(), false));
                };
                let block = block.read_with(&guard);
                let _ = block.property_value_to_css(&property_id, &mut css);
                block.property_priority(&property_id).important()
            }
            DeclarationTarget::FontFace(rule) => {
                let Ok(id) = FontFaceDescriptorId::from_ident(property) else {
                    return Some((String::new(), false));
                };
                let _ = rule.read_with(&guard).descriptors.get(id, &mut css);
                false
            }
        };
        Some((css, important))
    }

    pub fn retained_rule_style_set_property(
        &mut self,
        handle: &CssomRule,
        property: &str,
        value: &str,
        important: bool,
    ) -> Result<(), CssomError> {
        if !crate::css_limits::allowed_with_ancestors(value, handle.ancestors.len() + 1) {
            return Ok(());
        }
        if value.trim().is_empty() {
            self.retained_rule_style_remove_property(handle, property)?;
            return Ok(());
        }
        let target = self
            .retained_declaration_target(handle, &self.guard.read())
            .ok_or(CssomError::NotFound)?;
        let url_data = handle
            .sheet
            .sheet
            .contents(&self.guard.read())
            .url_data
            .clone();
        let lock = &self.guard;

        match target {
            DeclarationTarget::Block { block, rule_type } => {
                let Ok(property_id) = PropertyId::parse_enabled_for_all_content(property) else {
                    return Ok(());
                };
                let mut source = SourcePropertyDeclaration::default();
                if parse_one_declaration_into(
                    &mut source,
                    property_id,
                    value,
                    Origin::Author,
                    &url_data,
                    None,
                    ParsingMode::DEFAULT,
                    QuirksMode::NoQuirks,
                    rule_type,
                )
                .is_err()
                {
                    return Ok(());
                }
                let importance = if important {
                    Importance::Important
                } else {
                    Importance::Normal
                };
                let mut guard = lock.write();
                block
                    .write_with(&mut guard)
                    .extend(source.drain(), importance);
            }
            DeclarationTarget::FontFace(font_face) => {
                let Ok(id) = FontFaceDescriptorId::from_ident(property) else {
                    return Ok(());
                };
                let context = ParserContext::new(
                    Origin::Author,
                    &url_data,
                    Some(CssRuleType::FontFace),
                    ParsingMode::DEFAULT,
                    QuirksMode::NoQuirks,
                    Default::default(),
                    None,
                    None,
                    Default::default(),
                );
                let mut input = ParserInput::new(value);
                let mut parser = Parser::new(&mut input);
                let mut guard = lock.write();
                let descriptors = &mut font_face.write_with(&mut guard).descriptors;
                if descriptors.set(id, &context, &mut parser).is_err() {
                    return Ok(());
                }
            }
        };

        self.invalidate_cssom_sheet(&handle.sheet);
        Ok(())
    }

    pub fn retained_rule_style_remove_property(
        &mut self,
        handle: &CssomRule,
        property: &str,
    ) -> Result<String, CssomError> {
        let target = self
            .retained_declaration_target(handle, &self.guard.read())
            .ok_or(CssomError::NotFound)?;
        let lock = &self.guard;
        let mut removed_value = CssStringWriter::new();

        match target {
            DeclarationTarget::Block { block, .. } => {
                let Ok(property_id) = PropertyId::parse_enabled_for_all_content(property) else {
                    return Ok(removed_value);
                };
                let mut guard = lock.write();
                let block = block.write_with(&mut guard);
                let _ = block.property_value_to_css(&property_id, &mut removed_value);
                let Some(first_declaration) = block.first_declaration_to_remove(&property_id)
                else {
                    return Ok(removed_value);
                };
                block.remove_property(&property_id, first_declaration);
            }
            DeclarationTarget::FontFace(font_face) => {
                let Ok(id) = FontFaceDescriptorId::from_ident(property) else {
                    return Ok(removed_value);
                };
                let mut guard = lock.write();
                let descriptors = &mut font_face.write_with(&mut guard).descriptors;
                let _ = descriptors.get(id, &mut removed_value);
                if !descriptors.remove(id) {
                    return Ok(removed_value);
                }
            }
        };

        self.invalidate_cssom_sheet(&handle.sheet);
        Ok(removed_value)
    }

    pub fn retained_rule_set_selector_text(
        &mut self,
        handle: &CssomRule,
        selectors: &str,
    ) -> Result<(), CssomError> {
        if !crate::css_limits::allowed_with_ancestors(selectors, handle.ancestors.len() + 1) {
            return Ok(());
        }
        let sheet = self
            .retained_sheet(&handle.sheet)
            .ok_or(CssomError::NotFound)?
            .clone();
        let lock = &self.guard;
        let ancestors = &handle.ancestors;
        let RuleHandle::Rule(CssRule::Style(style_rule)) = &handle.rule else {
            return Err(CssomError::NotSupported);
        };
        // Nested style rules take relative selectors (`> .a`), resolved
        // against the innermost enclosing style rule or `@scope`, mirroring
        // stylo's `NestingContext`.
        let parse_relative = ancestors
            .iter()
            .rev()
            .find_map(|rule| match rule.rule_type() {
                CssRuleType::Style => Some(ParseRelative::ForNesting),
                CssRuleType::Scope => Some(ParseRelative::ForScope),
                _ => None,
            })
            .unwrap_or(ParseRelative::No);

        let new_selectors = {
            let guard = lock.read();
            let contents = sheet.contents(&guard);
            let selector_parser = SelectorParser {
                stylesheet_origin: contents.origin,
                namespaces: &contents.namespaces,
                url_data: &contents.url_data,
                for_supports_rule: false,
            };
            let mut input = ParserInput::new(selectors);
            let mut parser = Parser::new(&mut input);
            let Ok(new_selectors) =
                SelectorList::parse(&selector_parser, &mut parser, parse_relative)
            else {
                return Ok(());
            };
            new_selectors
        };

        {
            let mut guard = lock.write();
            style_rule.write_with(&mut guard).selectors = new_selectors;
        }

        self.invalidate_cssom_sheet(&handle.sheet);
        Ok(())
    }

    pub fn retained_rule_style_set_css_text(
        &mut self,
        handle: &CssomRule,
        css: &str,
    ) -> Result<(), CssomError> {
        let target = self
            .retained_declaration_target(handle, &self.guard.read())
            .ok_or(CssomError::NotFound)?;
        let css = if crate::css_limits::allowed_with_ancestors(css, handle.ancestors.len() + 1) {
            css
        } else {
            ""
        };
        let url = handle
            .sheet
            .sheet
            .contents(&self.guard.read())
            .url_data
            .clone();
        match target {
            DeclarationTarget::Block { block, rule_type } => {
                let parsed = style::properties::declaration_block::parse_style_attribute(
                    css,
                    &url,
                    None,
                    QuirksMode::NoQuirks,
                    rule_type,
                );
                *block.write_with(&mut self.guard.write()) = parsed;
            }
            DeclarationTarget::FontFace(rule) => {
                let context = ParserContext::new(
                    Origin::Author,
                    &url,
                    Some(CssRuleType::FontFace),
                    ParsingMode::DEFAULT,
                    QuirksMode::NoQuirks,
                    Default::default(),
                    None,
                    None,
                    Default::default(),
                );
                let mut input = ParserInput::new(css);
                let mut parser = Parser::new(&mut input);
                let source_location = rule.read_with(&self.guard.read()).source_location;
                let parsed =
                    style::font_face::parse_font_face_block(&context, &mut parser, source_location);
                rule.write_with(&mut self.guard.write()).descriptors = parsed.descriptors;
            }
        }
        self.invalidate_cssom_sheet(&handle.sheet);
        Ok(())
    }
}
fn list_contains(list: &RuleList, rule: &RuleHandle, guard: &SharedRwLockReadGuard) -> bool {
    let identity = rule_identity(rule);
    match list {
        RuleList::Css(rules) => rules
            .read_with(guard)
            .0
            .iter()
            .any(|rule| rule_identity(&RuleHandle::Rule(rule.clone())) == identity),
        RuleList::Keyframes(rule) => rule
            .read_with(guard)
            .keyframes
            .iter()
            .any(|rule| (&**rule as *const _) as usize == identity),
    }
}
