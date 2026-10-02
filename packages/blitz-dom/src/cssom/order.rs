//! Cascade order follows the DOM, independently of node allocation or reuse.
use crate::{BaseDocument, NodeId};

impl BaseDocument {
    pub(crate) fn stylesheet_owners_in_tree_order(
        &self,
        owners: impl IntoIterator<Item = NodeId>,
    ) -> Vec<NodeId> {
        let mut owners: Vec<_> = owners.into_iter().collect();
        // Do not cache all paths: many deeply nested owners would retain
        // sheet-count × depth entries simultaneously.
        owners.sort_by(|a, b| {
            self.stylesheet_tree_position(*a)
                .cmp(&self.stylesheet_tree_position(*b))
        });
        owners
    }

    /// Commit a complete tree-order view after a batch of owner moves.
    pub(crate) fn flush_author_stylesheet_order(&mut self) {
        if !std::mem::take(&mut self.stylesheet_order_dirty) {
            return;
        }
        let owners = self.stylesheet_owners_in_tree_order(self.nodes_to_stylesheet.keys().copied());
        let sheets: Vec<_> = owners
            .iter()
            .filter_map(|owner| self.nodes_to_stylesheet.get(owner).cloned())
            .collect();
        let guard = self.guard.read();
        for sheet in &sheets {
            self.stylist.remove_stylesheet(sheet.clone(), &guard);
        }
        for sheet in sheets {
            if let Some(adopted) = self.adopted_stylesheets.first() {
                self.stylist
                    .insert_stylesheet_before(sheet, adopted.clone(), &guard);
            } else {
                self.stylist.append_stylesheet(sheet, &guard);
            }
        }
    }

    fn stylesheet_tree_position(&self, owner: NodeId) -> (NodeId, Vec<usize>) {
        let mut node = owner;
        let mut position = Vec::new();
        // Nodes in distinct detached/document/shadow trees need only a stable
        // relative order. Within each tree, sibling positions define CSS order.
        for _ in 0..self.nodes.len() {
            let Some(parent) = self.get_node(node).and_then(|node| node.parent) else {
                break;
            };
            let Some(index) = self
                .get_node(parent)
                .and_then(|parent| parent.children.position(node))
            else {
                break;
            };
            position.push(index);
            node = parent;
        }
        position.reverse();
        (node, position)
    }
}
