//! Ordered DOM children with direct position lookup. A consumed prefix lets
//! repeated front removals retain contiguous slice access without shifting the
//! entire list. Middle removals shift only the shorter side.

use crate::NodeId;
use std::collections::HashMap;
use std::ops::{Deref, Range};
use std::sync::OnceLock;
use thin_vec::ThinVec;

/// A DOM child list: callers supply unique node IDs and valid insertion offsets.
/// All mutation goes through this type; read-only slices cannot invalidate the
/// index. `positions` stores physical offsets into `nodes`, including `start`.
/// Removing near either end is amortized constant time. Middle removals shift
/// the shorter side; insertions shift the suffix.
#[derive(Clone, Debug, Default)]
pub struct Children {
    nodes: ThinVec<NodeId>,
    start: usize,
    positions: HashMap<NodeId, usize>,
    first_legend: OnceLock<Option<NodeId>>,
    first_summary: OnceLock<Option<NodeId>>,
}

impl Children {
    /// Tag names are immutable; child-list edits invalidate both lookups.
    pub(crate) fn first_legend(&self, tree: &crate::NodeTree) -> Option<NodeId> {
        self.first_html_child(&self.first_legend, "legend", tree)
    }

    pub(crate) fn first_summary(&self, tree: &crate::NodeTree) -> Option<NodeId> {
        self.first_html_child(&self.first_summary, "summary", tree)
    }

    fn first_html_child(
        &self,
        cache: &OnceLock<Option<NodeId>>,
        tag: &str,
        tree: &crate::NodeTree,
    ) -> Option<NodeId> {
        *cache.get_or_init(|| {
            self.iter().copied().find(|&id| {
                tree[id]
                    .element_data()
                    .is_some_and(|e| e.name.ns == markup5ever::ns!(html) && &*e.name.local == tag)
            })
        })
    }

    pub fn as_slice(&self) -> &[NodeId] {
        &self.nodes[self.start..]
    }

    pub fn position(&self, id: NodeId) -> Option<usize> {
        self.positions.get(&id)?.checked_sub(self.start)
    }

    pub fn push(&mut self, id: NodeId) {
        self.extend_from_slice(&[id]);
    }

    pub fn extend_from_slice(&mut self, ids: &[NodeId]) {
        self.first_legend.take();
        self.first_summary.take();
        self.compact_if_needed();
        for &id in ids {
            self.positions.insert(id, self.nodes.len());
            self.nodes.push(id);
        }
    }

    /// Insert a contiguous group; callers provide unique, detached node IDs.
    pub fn insert_slice(&mut self, index: usize, ids: &[NodeId]) {
        self.first_legend.take();
        self.first_summary.take();
        let index = self.start + index;
        self.nodes.splice(index..index, ids.iter().copied());
        self.reindex(index..self.nodes.len());
    }

    pub fn remove_id(&mut self, id: NodeId) -> bool {
        self.first_legend.take();
        self.first_summary.take();
        let Some(index) = self.positions.remove(&id) else {
            return false;
        };
        let before = index - self.start;
        let after = self.nodes.len() - index - 1;
        if before < after {
            self.nodes.copy_within(self.start..index, self.start + 1);
            self.start += 1;
            self.reindex(self.start..index + 1);
        } else {
            self.nodes.remove(index);
            self.reindex(index..self.nodes.len());
        }
        self.compact_if_needed();
        true
    }

    pub fn clear(&mut self) {
        self.first_legend.take();
        self.first_summary.take();
        self.nodes.clear();
        self.positions.clear();
        self.start = 0;
    }

    pub fn retain(&mut self, mut keep: impl FnMut(&NodeId) -> bool) {
        self.first_legend.take();
        self.first_summary.take();
        let mut index = 0;
        let start = self.start;
        self.nodes.retain(|id| {
            let live = index >= start;
            index += 1;
            live && keep(id)
        });
        self.start = 0;
        self.positions.clear();
        self.reindex(0..self.nodes.len());
    }

    fn compact_if_needed(&mut self) {
        if self.start > 0 && self.start >= self.len() {
            self.nodes.drain(..self.start);
            self.start = 0;
            self.reindex(0..self.nodes.len());
        }
    }

    fn reindex(&mut self, range: Range<usize>) {
        for index in range {
            self.positions.insert(self.nodes[index], index);
        }
    }
}

impl Deref for Children {
    type Target = [NodeId];
    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}

impl From<ThinVec<NodeId>> for Children {
    fn from(nodes: ThinVec<NodeId>) -> Self {
        let mut children = Self {
            nodes,
            ..Self::default()
        };
        children.reindex(0..children.nodes.len());
        children
    }
}

impl IntoIterator for Children {
    type Item = NodeId;
    type IntoIter = std::iter::Skip<thin_vec::IntoIter<NodeId>>;
    fn into_iter(self) -> Self::IntoIter {
        self.nodes.into_iter().skip(self.start)
    }
}

impl<'a> IntoIterator for &'a Children {
    type Item = &'a NodeId;
    type IntoIter = std::slice::Iter<'a, NodeId>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixed_edits_match_an_ordered_reference() {
        let mut children = Children::default();
        let mut expected = Vec::new();
        let mut state = 7_u64;
        for serial in 1..5000 {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            if expected.is_empty() || state.is_multiple_of(3) {
                let id = NodeId::from_u64(serial);
                let index = state as usize % (expected.len() + 1);
                children.insert_slice(index, &[id]);
                expected.insert(index, id);
            } else if state % 3 == 1 {
                let index = state as usize % expected.len();
                let id = expected.remove(index);
                assert!(children.remove_id(id));
                assert_eq!(children.position(id), None);
            } else {
                let id = NodeId::from_u64(serial);
                children.push(id);
                expected.push(id);
            }
            assert_eq!(children.as_slice(), expected);
            for (i, id) in expected.iter().enumerate() {
                assert_eq!(children.position(*id), Some(i));
            }
        }
        children.retain(|id| id.as_u64().is_multiple_of(2));
        expected.retain(|id| id.as_u64().is_multiple_of(2));
        assert_eq!(children.as_slice(), expected);
        assert_eq!(children.clone().into_iter().collect::<Vec<_>>(), expected);
        children.clear();
        assert!(children.is_empty());
        assert!(children.positions.is_empty());
    }
}
