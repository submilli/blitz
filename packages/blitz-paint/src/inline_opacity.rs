//! Inline effects belong to a whole element, even when Parley splits its text
//! across lines or font runs. Record each opacity group once, then replay its
//! contents within one layer (CSS Color 4, `opacity`).
use std::collections::HashSet;

use anyrender::PaintScene;
use blitz_dom::{BaseDocument, NodeId};

pub(crate) trait NodeScene {
    type Scene: PaintScene;
    fn for_node(&mut self, id: NodeId) -> &mut Self::Scene;
    fn for_child(&mut self, id: NodeId) -> &mut Self::Scene {
        self.for_node(id)
    }
}

pub(crate) struct DirectScene<'a, S>(pub &'a mut S);

impl<S: PaintScene> NodeScene for DirectScene<'_, S> {
    type Scene = S;
    fn for_node(&mut self, _: NodeId) -> &mut S {
        self.0
    }
}

/// A zero-opacity inline can be absent from the box paint tree. Keep its
/// suppression on every flattened descendant, including hoisted positioned boxes.
pub(crate) fn hidden_inline_descendants(doc: &BaseDocument) -> HashSet<NodeId> {
    let mut hidden = HashSet::new();
    let mut work = vec![(doc.root_node().id, false)];
    while let Some((id, ancestor_hidden)) = work.pop() {
        let node = &doc.as_ref().tree()[id];
        let own_hidden = node.primary_styles().is_some_and(|styles| {
            styles.clone_display().is_inline_flow() && styles.get_effects().opacity == 0.0
        });
        let suppressed = ancestor_hidden || own_hidden;
        if suppressed {
            hidden.insert(id);
        }
        work.extend(node.after().map(|id| (id, suppressed)));
        work.extend(
            node.flat_children()
                .iter()
                .rev()
                .map(|&id| (id, suppressed)),
        );
        work.extend(node.before().map(|id| (id, suppressed)));
    }
    hidden
}
