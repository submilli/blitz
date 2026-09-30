//! Native topology and work references for an embedder's tracing checkpoint.
//!
//! DOM reachability is bidirectional: a retained detached descendant can expose
//! its ancestors and siblings. Layout caches are deliberately not ownership
//! edges; a future sweep must invalidate them before freeing native slots.

mod caches;
mod roots;
mod sweep;
#[cfg(test)]
mod sweep_tests;
#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};

use crate::{BaseDocument, NodeId};

/// Snapshot of the native references an embedder must add to its traced graph.
///
/// Each edge is undirected. `roots` includes the active document and native work,
/// including registered parser/native leases. Unregistered JavaScript/native
/// continuations remain the embedder's responsibility. A snapshot is
/// invalid after document mutation; it grants no authority to free slots.
#[derive(Debug)]
pub struct NodeReachability {
    pub nodes: Vec<NodeId>,
    pub edges: Vec<(NodeId, NodeId)>,
    pub roots: Vec<NodeId>,
}

impl NodeReachability {
    /// Iterative protected closure, including explicit embedder-held nodes.
    /// Stale generation IDs never alias a new occupant of the same slot.
    pub fn retained_nodes(&self, embedder_roots: &[NodeId]) -> HashSet<NodeId> {
        let live: HashSet<_> = self.nodes.iter().copied().collect();
        let mut neighbors: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
        for &(left, right) in &self.edges {
            if live.contains(&left) && live.contains(&right) {
                neighbors.entry(left).or_default().push(right);
                neighbors.entry(right).or_default().push(left);
            }
        }
        let mut retained = HashSet::new();
        let mut work: Vec<_> = self.roots.iter().chain(embedder_roots).copied().collect();
        while let Some(id) = work.pop() {
            if !live.contains(&id) || !retained.insert(id) {
                continue;
            }
            if let Some(adjacent) = neighbors.get(&id) {
                work.extend(adjacent);
            }
        }
        retained
    }
}

impl BaseDocument {
    /// Describe native ownership and pending work without changing the DOM or
    /// invoking providers. Embedders additionally trace their own pending IDs.
    pub fn node_reachability(&self) -> NodeReachability {
        let mut graph = NodeReachability {
            nodes: self.nodes.iter().map(|(id, _)| id).collect(),
            edges: Vec::new(),
            roots: roots::native_work(self),
        };
        for (id, node) in self.nodes.iter() {
            graph.edges.extend(node.parent.map(|parent| (id, parent)));
            // Anonymous boxes borrow DOM children; those cached links may be
            // stale after removal and must not turn into permanent roots.
            if !node.is_anonymous() {
                graph
                    .edges
                    .extend(node.children.iter().map(|&child| (id, child)));
            }
            graph
                .edges
                .extend(node.anonymous_blocks.iter().map(|&child| (id, child)));
            graph.edges.extend(node.before().map(|child| (id, child)));
            graph.edges.extend(node.after().map(|child| (id, child)));
            if let Some(element) = node.element_data() {
                graph
                    .edges
                    .extend(element.template_contents.map(|child| (id, child)));
                graph
                    .edges
                    .extend(element.shadow_root.map(|child| (id, child)));
            }
            if let Some(shadow) = &node.shadow_root_data {
                graph.edges.push((id, shadow.host));
            }
        }
        graph.edges.retain(|&(left, right)| {
            self.nodes.contains_key(left) && self.nodes.contains_key(right)
        });
        graph.roots.retain(|&id| self.nodes.contains_key(id));
        graph
    }
}
