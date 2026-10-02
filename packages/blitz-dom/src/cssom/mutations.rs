//! Bounded insertion and deletion through Stylo rule lists.
use super::*;
impl BaseDocument {
    /// Insert a rule parsed from `rule` at `index` in the rule list at `path`
    /// (`CSSStyleSheet.insertRule` / `CSSGroupingRule.insertRule` /
    /// `CSSKeyframesRule.appendRule`). Returns the index of the inserted rule.
    pub fn stylesheet_insert_rule(
        &mut self,
        node_id: NodeId,
        path: &[usize],
        rule: &str,
        index: usize,
    ) -> Result<usize, CssomError> {
        let handle = self
            .retain_stylesheet(node_id)
            .ok_or(CssomError::NotFound)?;
        self.retained_stylesheet_insert_rule(&handle, path, rule, index)
    }

    /// The same operation on a retained sheet; foreign-document handles fail.
    pub fn retained_stylesheet_insert_rule(
        &mut self,
        handle: &CssomSheet,
        path: &[usize],
        rule: &str,
        index: usize,
    ) -> Result<usize, CssomError> {
        let sheet = self.retained_sheet(handle).ok_or(CssomError::NotFound)?;
        let (ancestors, list) =
            Self::resolve_rule_list_with_ancestors(sheet, &self.guard.read(), path)
                .ok_or(CssomError::NotFound)?;
        self.cssom_insert_rule(
            handle,
            ancestors,
            list,
            rule,
            index,
            self.stylesheet_is_current(handle),
        )
    }

    pub(super) fn cssom_insert_rule(
        &mut self,
        handle: &CssomSheet,
        ancestors: Vec<CssRule>,
        list: RuleList,
        rule: &str,
        index: usize,
        attached: bool,
    ) -> Result<usize, CssomError> {
        if !crate::css_limits::allowed_with_ancestors(rule, ancestors.len()) {
            return Err(CssomError::Syntax);
        }
        let sheet = self
            .retained_sheet(handle)
            .ok_or(CssomError::NotFound)?
            .clone();
        let detached_loader = DetachedStylesheetLoader {
            url_data: sheet.contents(&self.guard.read()).url_data.clone(),
        };
        let loader = self.stylesheet_loader();
        let lock = &self.guard;

        let count = match &list {
            RuleList::Css(rules) => rules.read_with(&lock.read()).0.len(),
            RuleList::Keyframes(rule) => rule.read_with(&lock.read()).keyframes.len(),
        };
        if count >= 16384 {
            return Err(CssomError::NotSupported);
        }
        let (new_rule, index) = match &list {
            RuleList::Css(rules) => {
                let new_rule = {
                    let guard = lock.read();
                    let contents = sheet.contents(&guard);
                    let containing_rule_types =
                        ancestors
                            .iter()
                            .fold(CssRuleTypes::default(), |types, rule| {
                                CssRuleTypes::from_bits(types.bits() | rule.rule_type().bit())
                            });
                    let parse_relative_rule_type = ancestors.last().map(|rule| rule.rule_type());
                    rules.read_with(&guard).parse_rule_for_insert(
                        lock,
                        rule,
                        contents,
                        index,
                        containing_rule_types,
                        parse_relative_rule_type,
                        Some(if attached { &loader } else { &detached_loader }),
                        if handle.constructed {
                            AllowImportRules::No
                        } else {
                            AllowImportRules::Yes
                        },
                    )?
                };
                {
                    let mut guard = lock.write();
                    rules
                        .write_with(&mut guard)
                        .0
                        .insert(index, new_rule.clone());
                }
                (new_rule, index)
            }
            RuleList::Keyframes(keyframes) => {
                let keyframe = {
                    let guard = lock.read();
                    let contents = sheet.contents(&guard);
                    Keyframe::parse(rule, contents, lock).map_err(|_| CssomError::Syntax)?
                };
                let index = {
                    let mut guard = lock.write();
                    let keyframes = &mut keyframes.write_with(&mut guard).keyframes;
                    keyframes.push(keyframe);
                    keyframes.len() - 1
                };
                (CssRule::Keyframes(keyframes.clone()), index)
            }
        };

        let guard = lock.read();
        if attached && let CssRule::FontFace(_) = &new_rule {
            crate::net::fetch_font_face_rules(
                std::iter::once(&new_rule),
                self.tx.clone(),
                self.id(),
                Some(self.resource_pin(handle.owner)),
                &self.net_provider,
                &self.shell_provider,
                &guard,
                self.abort_signal.as_ref(),
            );
        }
        drop(guard);
        self.invalidate_cssom_sheet(handle);
        Ok(index)
    }

    /// Remove the rule at `index` from the rule list at `path`
    /// (`CSSStyleSheet.deleteRule` / `CSSGroupingRule.deleteRule` /
    /// `CSSKeyframesRule.deleteRule`).
    pub fn stylesheet_delete_rule(
        &mut self,
        node_id: NodeId,
        path: &[usize],
        index: usize,
    ) -> Result<(), CssomError> {
        let handle = self
            .retain_stylesheet(node_id)
            .ok_or(CssomError::NotFound)?;
        self.retained_stylesheet_delete_rule(&handle, path, index)
    }

    /// The same operation on a retained sheet; foreign-document handles fail.
    pub fn retained_stylesheet_delete_rule(
        &mut self,
        handle: &CssomSheet,
        path: &[usize],
        index: usize,
    ) -> Result<(), CssomError> {
        let sheet = self.retained_sheet(handle).ok_or(CssomError::NotFound)?;
        let (_, list) = Self::resolve_rule_list_with_ancestors(sheet, &self.guard.read(), path)
            .ok_or(CssomError::NotFound)?;
        self.cssom_delete_rule(handle, list, index)
    }

    pub(super) fn cssom_delete_rule(
        &mut self,
        handle: &CssomSheet,
        list: RuleList,
        index: usize,
    ) -> Result<(), CssomError> {
        self.retained_sheet(handle).ok_or(CssomError::NotFound)?;
        {
            let mut guard = self.guard.write();
            match list {
                RuleList::Css(rules) => {
                    rules.write_with(&mut guard).remove_rule(index)?;
                }
                RuleList::Keyframes(keyframes) => {
                    let list = &mut keyframes.write_with(&mut guard).keyframes;
                    if index >= list.len() {
                        return Err(CssomError::IndexSize);
                    }
                    list.remove(index);
                }
            }
        }
        self.invalidate_cssom_sheet(handle);
        Ok(())
    }
}
