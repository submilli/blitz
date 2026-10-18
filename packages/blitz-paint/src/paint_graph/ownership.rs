//! Resolve flat effect ancestry separately from containing-block clips.
use super::*;

pub(super) fn is_flex_or_grid_item(doc: &BaseDocument, id: NodeId) -> bool {
    doc.as_ref().tree()[id]
        .layout_parent
        .get()
        .is_some_and(|parent| {
            matches!(
                doc.as_ref().tree()[parent].taffy_display(),
                taffy::Display::Flex | taffy::Display::Grid
            )
        })
}

pub(super) struct WorkBudget(pub(super) usize);
impl WorkBudget {
    pub(super) fn spend(&mut self, amount: usize) -> Option<()> {
        self.0 = self.0.saturating_add(amount);
        (self.0 <= RECORDING_WORK_LIMIT).then_some(())
    }
}

pub(super) struct ScopeIndex<'a> {
    doc: &'a BaseDocument,
    scopes: &'a [Scope],
    effects: HashMap<NodeId, Vec<usize>>,
    overflows: HashMap<NodeId, Vec<usize>>,
    clip: Rect,
}
impl<'a> ScopeIndex<'a> {
    pub(super) fn new(doc: &'a BaseDocument, scopes: &'a [Scope], clip: Rect) -> Self {
        let mut effects = HashMap::<NodeId, Vec<usize>>::new();
        let mut overflows = HashMap::<NodeId, Vec<usize>>::new();
        for (id, scope) in scopes.iter().enumerate() {
            if scope.effect {
                effects.entry(scope.node).or_default().push(id);
            }
            if scope.overflow {
                overflows.entry(scope.node).or_default().push(id);
            }
        }
        Self {
            doc,
            scopes,
            effects,
            overflows,
            clip,
        }
    }

    fn ancestry(&self, node: NodeId, budget: &mut WorkBudget) -> Option<Vec<NodeId>> {
        let mut ancestry = Vec::new();
        let mut current = Some(node);
        while let Some(id) = current {
            budget.spend(1)?;
            ancestry.push(id);
            current = self.doc.as_ref().tree()[id].flat_parent_id();
        }
        ancestry.reverse();
        Some(ancestry)
    }

    fn attach_overflow(
        &self,
        drawing: &mut Drawing,
        active: &mut HashSet<usize>,
        budget: &mut WorkBudget,
    ) -> Option<()> {
        // Hoisted visits can bypass their containing block's paint scope. Its
        // overflow still applies, while a static flat ancestor can be escaped.
        let mut containing = self.doc.as_ref().tree()[drawing.node].containing_block();
        while let Some(id) = containing {
            budget.spend(1)?;
            if let Some(scopes) = self.overflows.get(&id) {
                for &scope in scopes {
                    budget.spend(1)?;
                    if active.insert(scope) {
                        drawing.scopes.push(scope);
                    }
                }
            }
            containing = self.doc.as_ref().tree()[id].containing_block();
        }
        Some(())
    }

    fn context_path(
        &self,
        drawing: &Drawing,
        ancestry: &[NodeId],
        active: &HashSet<usize>,
    ) -> Vec<Key> {
        let deepest_context = ancestry
            .iter()
            .rposition(|&id| self.is_context(id))
            .unwrap_or(0);
        // A z-index:auto box participates at its tree position in the enclosing
        // context. Its positioned descendants participate independently, rather
        // than being trapped in an atomic group with that auto ancestor.
        let positioned_owner = ancestry.iter().enumerate().rev().find_map(|(depth, &id)| {
            (depth >= deepest_context
                && !self.is_context(id)
                && self.doc.as_ref().tree()[id]
                    .primary_styles()
                    .is_some_and(|styles| styles.clone_position() != Position::Static))
            .then_some(id)
        });
        let mut path = Vec::new();
        for (depth, &id) in ancestry.iter().enumerate() {
            let node = &self.doc.as_ref().tree()[id];
            if let Some(styles) = node.primary_styles() {
                let context = self.is_context(id);
                if context || (depth >= deepest_context && positioned_owner == Some(id)) {
                    path.push(Key::Context(id));
                }
                if styles.clone_display().is_inline_flow()
                    && !node.flags.is_inline_root()
                    && styles.get_effects().opacity < 1.0
                {
                    path.push(Key::Inline(id));
                }
            }
            if let Some(scopes) = self.effects.get(&id) {
                path.extend(
                    scopes
                        .iter()
                        .filter(|scope| id != drawing.node || active.contains(scope))
                        .copied()
                        .map(Key::Effect),
                );
            }
        }
        path
    }

    fn is_context(&self, id: NodeId) -> bool {
        let node = &self.doc.as_ref().tree()[id];
        let flex_or_grid_item = is_flex_or_grid_item(self.doc, id);
        node.is_stacking_context_root(flex_or_grid_item)
    }

    fn insert_local_scopes(
        &self,
        path: &mut Vec<Key>,
        drawing: &Drawing,
        depths: &HashMap<NodeId, usize>,
        budget: &mut WorkBudget,
    ) -> Option<()> {
        for &scope_id in &drawing.scopes {
            let scope = &self.scopes[scope_id];
            if scope.effect {
                continue;
            }
            let depth = depths.get(&scope.node).copied().unwrap_or(0);
            let mut insertion = 0;
            for (index, &key) in path.iter().enumerate() {
                budget.spend(1)?;
                let (node, inline) = match key {
                    Key::Effect(id) | Key::Local(id) => (self.scopes[id].node, false),
                    Key::Inline(node) => (node, true),
                    Key::Context(node) => (node, false),
                };
                let key_depth = depths.get(&node).copied().unwrap_or(0);
                if key_depth <= depth || inline {
                    insertion = index + 1;
                }
            }
            // Context keys carry ordering, not effects. Put local clips inside
            // them so escaping positioned boxes can keep their own clip chain
            // without splitting the enclosing context's paint order.
            while matches!(path.get(insertion), Some(Key::Context(_))) {
                insertion += 1;
            }
            path.insert(insertion, Key::Local(scope_id));
        }
        Some(())
    }

    pub(super) fn path(&self, drawing: &mut Drawing, budget: &mut WorkBudget) -> Option<Vec<Key>> {
        let ancestry = self.ancestry(drawing.node, budget)?;
        let depths = ancestry
            .iter()
            .copied()
            .enumerate()
            .map(|(depth, id)| (id, depth))
            .collect();
        let mut active = drawing.scopes.iter().copied().collect();
        self.attach_overflow(drawing, &mut active, budget)?;
        let mut path = self.context_path(drawing, &ancestry, &active);
        self.insert_local_scopes(&mut path, drawing, &depths, budget)?;
        Some(path)
    }

    pub(super) fn group(&self, key: Key) -> Group {
        let (node, command) = match key {
            Key::Effect(id) | Key::Local(id) => {
                (self.scopes[id].node, Some(self.scopes[id].command.clone()))
            }
            Key::Context(id) => (id, None),
            Key::Inline(id) => {
                let mut scene = Scene::new();
                scene.push_layer(
                    peniko::Mix::Normal,
                    self.doc.as_ref().tree()[id]
                        .primary_styles()
                        .unwrap()
                        .get_effects()
                        .opacity,
                    Affine::IDENTITY,
                    &self.clip,
                    None,
                    None,
                );
                (id, scene.commands.pop())
            }
        };
        Group {
            node,
            command,
            key: Some(key),
            children: Vec::new(),
            postlude: matches!(key, Key::Local(id) if self.scopes[id].postlude),
        }
    }
}
