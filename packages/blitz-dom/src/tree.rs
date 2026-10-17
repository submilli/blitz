//! Versioned storage for the nodes of the DOM tree.

use std::collections::HashMap;
use std::ops::{Index, IndexMut};
use std::sync::atomic::{AtomicU64, Ordering};

use blitz_traits::node_id::NodeId;
use slotmap::{Key as _, KeyData, SlotMap};

use crate::Node;

slotmap::new_key_type! {
    /// The internal [`slotmap`] key for node storage. Only used at the
    /// storage boundary: all public APIs use [`NodeId`].
    struct NodeKey;
}

#[inline(always)]
fn to_key(id: NodeId) -> NodeKey {
    NodeKey::from(KeyData::from_ffi(id.as_u64()))
}

#[inline(always)]
fn to_id(key: NodeKey) -> NodeId {
    NodeId::from_u64(key.data().as_ffi())
}

/// The versioned map in which the nodes of the DOM tree are stored, backed by
/// a [`slotmap::SlotMap`].
///
/// Nodes are addressed by [`NodeId`], which carries the slot's version in
/// addition to its index: when a node is dropped and its slot reused, ids
/// referring to the dropped node no longer resolve ([`NodeTree::get`] returns
/// `None`, and indexing panics) instead of aliasing the new occupant.
pub struct NodeTree {
    map: SlotMap<NodeKey, Node>,
    style_ids: HashMap<usize, NodeId>,
    // Prepared before style traversal, immutable across its worker threads.
    // Every mutable node access invalidates it, including direct embedder edits.
    style_roots: Option<HashMap<NodeId, NodeId>>,
    /// Bumped whenever box geometry may have moved (layout ran or a scroll
    /// offset changed). Values derived from `final_layout()` (e.g. hoisted
    /// paint child offsets) are memoised against it. Starts at 1 so that a
    /// stamp of 0 is never current.
    geometry_generation: AtomicU64,
    /// Scroll offset of the viewport (the root element scrolls the viewport,
    /// so its offset lives here rather than on the node).
    viewport_scroll: crate::Point<f64>,
}

impl NodeTree {
    pub(crate) fn new() -> Self {
        Self {
            map: SlotMap::with_key(),
            style_ids: HashMap::new(),
            style_roots: None,
            geometry_generation: AtomicU64::new(1),
            viewport_scroll: crate::Point::ZERO,
        }
    }

    pub fn viewport_scroll(&self) -> crate::Point<f64> {
        self.viewport_scroll
    }

    /// Memoised paint geometry may depend on the viewport scroll, so this
    /// bumps the geometry generation like any other scroll write.
    pub(crate) fn set_viewport_scroll(&mut self, scroll: crate::Point<f64>) {
        self.viewport_scroll = scroll;
        self.bump_geometry_generation();
    }

    /// See the `geometry_generation` field.
    pub fn geometry_generation(&self) -> u64 {
        self.geometry_generation.load(Ordering::Relaxed)
    }

    pub(crate) fn bump_geometry_generation(&self) {
        self.geometry_generation.fetch_add(1, Ordering::Relaxed);
    }

    /// Cache DOM roots once for style matching, which queries each element's
    /// containing shadow tree repeatedly. Path compression visits each node at
    /// most once even when arena order differs from ancestry after a move.
    pub(crate) fn prepare_style_roots(&mut self) {
        if self.style_roots.is_some() {
            return;
        }
        let mut roots = HashMap::with_capacity(self.len());
        let mut path = Vec::new();
        for (id, _) in self.iter() {
            let mut current = id;
            let root = loop {
                if let Some(&root) = roots.get(&current) {
                    break root;
                }
                path.push(current);
                match self[current].parent {
                    Some(parent) => current = parent,
                    None => break current,
                }
            };
            for node in path.drain(..) {
                roots.insert(node, root);
            }
        }
        self.style_roots = Some(roots);
    }

    /// Style queries outside a prepared traversal still see current ancestry.
    pub(crate) fn style_tree_root(&self, mut id: NodeId) -> NodeId {
        if let Some(root) = self.style_roots.as_ref().and_then(|roots| roots.get(&id)) {
            return *root;
        }
        while let Some(parent) = self[id].parent {
            id = parent;
        }
        id
    }

    /// The number of live nodes in the map.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Whether `id` resolves to a live node.
    pub fn contains_key(&self, id: NodeId) -> bool {
        self.map.contains_key(to_key(id))
    }

    /// Get a reference to the node with the given id, if it is still live.
    pub fn get(&self, id: NodeId) -> Option<&Node> {
        self.map.get(to_key(id))
    }

    /// Get a mutable reference to the node with the given id, if it is still live.
    pub fn get_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.style_roots = None;
        self.map.get_mut(to_key(id))
    }

    /// Resolve an opaque Stylo identity without dereferencing page-influenced data.
    pub(crate) fn node_for_style_identity(&self, identity: usize) -> Option<NodeId> {
        self.style_ids.get(&identity).copied()
    }

    /// Insert a node constructed with knowledge of its own id.
    pub(crate) fn insert_with_key(&mut self, f: impl FnOnce(NodeId) -> Node) -> NodeId {
        self.style_roots = None;
        let id = to_id(self.map.insert_with_key(|key| f(to_id(key))));
        self.style_ids
            .insert(self[id].style_identity().as_ptr() as usize, id);
        id
    }

    /// Remove the node with the given id, returning it if it was still live.
    pub(crate) fn remove(&mut self, id: NodeId) -> Option<Node> {
        self.style_roots = None;
        let node = self.map.remove(to_key(id))?;
        self.style_ids
            .remove(&(node.style_identity().as_ptr() as usize));
        Some(node)
    }

    /// Iterate over all live `(NodeId, &Node)` pairs.
    pub fn iter(&self) -> impl Iterator<Item = (NodeId, &Node)> {
        self.map.iter().map(|(key, node)| (to_id(key), node))
    }

    /// Iterate over all live `(NodeId, &mut Node)` pairs.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (NodeId, &mut Node)> {
        self.style_roots = None;
        self.map.iter_mut().map(|(key, node)| (to_id(key), node))
    }
}

impl Index<NodeId> for NodeTree {
    type Output = Node;

    #[track_caller]
    #[inline]
    fn index(&self, id: NodeId) -> &Node {
        &self.map[to_key(id)]
    }
}

impl IndexMut<NodeId> for NodeTree {
    #[track_caller]
    #[inline]
    fn index_mut(&mut self, id: NodeId) -> &mut Node {
        self.style_roots = None;
        &mut self.map[to_key(id)]
    }
}

#[cfg(test)]
mod style_identity_tests {
    use crate::{BaseDocument, DocumentConfig};

    #[test]
    fn opaque_identity_survives_storage_growth_and_is_removed_with_its_node() {
        let mut doc = BaseDocument::new(DocumentConfig::default());
        let id = doc.mutate().create_text_node("retained");
        let opaque = doc.nodes[id].style_identity().as_ptr() as usize;
        for _ in 0..1024 {
            doc.mutate().create_text_node("grow storage");
        }
        assert_eq!(doc.nodes[id].style_identity().as_ptr() as usize, opaque);
        assert_eq!(doc.nodes.node_for_style_identity(opaque), Some(id));
        doc.reclaim_detached_nodes(&[id]);
        assert_eq!(doc.nodes.node_for_style_identity(opaque), Some(id));
        doc.reclaim_detached_nodes(&[]);
        assert_eq!(doc.nodes.node_for_style_identity(opaque), None);
    }
}
