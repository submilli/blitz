//! Script-free focus eligibility; callers flush style before inspecting rendering.
use crate::{BaseDocument, Node, NodeId, ns};
use markup5ever::local_name;
use std::collections::HashMap;
use style::computed_values::visibility::T as Visibility;
use style::values::specified::box_::{DisplayInside, DisplayOutside};

impl Node {
    /// Programmatic focus accepts negative tabindex, unlike sequential navigation.
    /// Connectedness, disabledness and rendering are evaluated from current state.
    pub fn is_programmatically_focusable(&self) -> bool {
        self.flags.is_in_document()
            && self.has_focusable_semantics()
            && !self.is_disabled()
            && self.is_rendered_for_focus()
    }

    fn has_focusable_semantics(&self) -> bool {
        let Some(element) = self.element_data() else {
            return false;
        };
        if element.name.ns == ns!(html)
            && element.name.local == local_name!("input")
            && element
                .attr(local_name!("type"))
                .is_some_and(|s| s.eq_ignore_ascii_case("hidden"))
        {
            return false;
        }
        if self.tab_index().is_some() {
            return true;
        }
        if element.name.ns != ns!(html) {
            return false;
        }
        match &*element.name.local {
            "input" | "button" | "select" | "textarea" | "iframe" | "frame" | "object" => true,
            "a" | "area" => element.has_attr(local_name!("href")),
            "summary" => self.parent.is_some_and(|parent| {
                let parent = self.with(parent);
                parent
                    .element_data()
                    .is_some_and(|e| e.name.ns == ns!(html) && &*e.name.local == "details")
                    && parent.children.first_summary(self.tree()) == Some(self.id)
            }),
            _ => element
                .attr(local_name!("contenteditable"))
                .is_some_and(|v| {
                    v.is_empty()
                        || v.eq_ignore_ascii_case("true")
                        || v.eq_ignore_ascii_case("plaintext-only")
                }),
        }
    }

    pub(crate) fn tab_index(&self) -> Option<i32> {
        let value = self.element_data()?.attr(local_name!("tabindex"))?;
        style::servo::attr::parse_integer(value.chars()).ok()
    }

    fn is_rendered_for_focus(&self) -> bool {
        if self
            .primary_styles()
            .is_some_and(|s| s.clone_visibility() != Visibility::Visible)
        {
            return false;
        }
        let mut node = self;
        loop {
            if node.primary_styles().is_some_and(|s| {
                s.clone_display().outside() == DisplayOutside::None
                    && (node.id == self.id || s.clone_display().inside() != DisplayInside::Contents)
            }) || node
                .element_data()
                .is_some_and(|e| e.has_attr(local_name!("inert")))
            {
                return false;
            }
            let Some(parent) = node.parent else {
                return true;
            };
            node = self.with(parent);
        }
    }
}

#[derive(Clone, Copy, Default)]
struct FocusContext {
    disabled_ancestors: bool,
    hidden_subtree: bool,
}

impl BaseDocument {
    /// First eligible control in caller order after layout has been resolved.
    /// Memoize ancestor context across candidates, including distinct DOM roots,
    /// so reporting many unfocusable controls visits each ancestor at most once.
    pub fn first_programmatically_focusable(&self, candidates: &[NodeId]) -> Option<NodeId> {
        let mut contexts = HashMap::new();
        candidates
            .iter()
            .copied()
            .find(|&id| self.focusable_with_context(id, &mut contexts))
    }

    pub(crate) fn sequential_focus_target(&self, start: NodeId, backwards: bool) -> Option<NodeId> {
        let start = self.get_node(start)?;
        let mut contexts = HashMap::new();
        let eligible = |node: &Node| {
            node.tab_index().is_none_or(|i| i >= 0)
                && self.focusable_with_context(node.id, &mut contexts)
        };
        if backwards {
            self.prev_node(start, eligible)
        } else {
            self.next_node(start, eligible)
        }
    }

    fn focusable_with_context(
        &self,
        id: NodeId,
        contexts: &mut HashMap<NodeId, FocusContext>,
    ) -> bool {
        let Some(node) = self
            .get_node(id)
            .filter(|n| n.flags.is_in_document() && n.has_focusable_semantics())
        else {
            return false;
        };
        let context = self.focus_context(id, contexts);
        !context.hidden_subtree
            && !node.is_disabled_with_ancestors(context.disabled_ancestors)
            && !node.primary_styles().is_some_and(|s| {
                s.clone_visibility() != Visibility::Visible
                    || s.clone_display().outside() == DisplayOutside::None
            })
    }

    fn focus_context(&self, id: NodeId, cache: &mut HashMap<NodeId, FocusContext>) -> FocusContext {
        let mut path = Vec::new();
        let mut current = Some(id);
        while let Some(id) = current {
            if cache.contains_key(&id) {
                break;
            }
            let Some(node) = self.get_node(id) else { break };
            path.push(id);
            current = node.parent;
        }
        for id in path.into_iter().rev() {
            let node = &self.nodes[id];
            let parent = node.parent.and_then(|id| self.get_node(id));
            let mut context = parent
                .and_then(|n| cache.get(&n.id).copied())
                .unwrap_or_default();
            if let Some(parent) = parent
                && parent.fieldset_disables(node)
            {
                context.disabled_ancestors = true;
            }
            context.hidden_subtree |= node.primary_styles().is_some_and(|s| {
                s.clone_display().outside() == DisplayOutside::None
                    && s.clone_display().inside() != DisplayInside::Contents
            }) || node
                .element_data()
                .is_some_and(|e| e.has_attr(local_name!("inert")));
            cache.insert(id, context);
        }
        cache.get(&id).copied().unwrap_or_default()
    }
}
