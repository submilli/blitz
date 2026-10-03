//! Live DOM boundary adjustment at the script-free tree mutation boundary.
//! <https://dom.spec.whatwg.org/#ranges>
//!
//! The embedder owns range lifetimes. These values contain no scripting handles;
//! observers apply each mutation synchronously, before any script can run.

use crate::{BaseDocument, NodeId};

mod text;

/// A DOM boundary point, with offsets measured in children or UTF-16 code units.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Boundary {
    pub node: NodeId,
    pub offset: usize,
}

/// A completed insertion or a character-data operation. Removal uses the
/// existing pre-removal notification, while the old ancestry is still intact.
#[derive(Clone, Copy, Debug)]
pub enum RangeMutation {
    Insert {
        parent: NodeId,
        index: usize,
        count: usize,
    },
    ReplaceData {
        node: NodeId,
        offset: usize,
        removed: usize,
        added: usize,
    },
    Split {
        node: NodeId,
        new_node: NodeId,
        offset: usize,
    },
    Merge {
        node: NodeId,
        into: NodeId,
        offset: usize,
    },
}

impl Boundary {
    /// Compare valid DOM boundary points, returning None for separate trees.
    pub fn compare(self, document: &BaseDocument, other: Self) -> Option<std::cmp::Ordering> {
        if self.node == other.node {
            return Some(self.offset.cmp(&other.offset));
        }
        let ancestry = |mut id| {
            let mut path = vec![id];
            while let Some(parent) = document.get_node(id).and_then(|n| n.parent) {
                path.push(parent);
                id = parent;
            }
            path.reverse();
            path
        };
        let left = ancestry(self.node);
        let right = ancestry(other.node);
        if left.first() != right.first() {
            return None;
        }
        let shared = left.iter().zip(&right).take_while(|(a, b)| a == b).count();
        let parent = document.get_node(left[shared - 1])?;
        use std::cmp::Ordering::{Greater, Less};
        if shared == left.len() {
            let index = parent.children.position(right[shared])?;
            return Some(if self.offset <= index { Less } else { Greater });
        }
        if shared == right.len() {
            let index = parent.children.position(left[shared])?;
            return Some(if index < other.offset { Less } else { Greater });
        }
        Some(
            parent
                .children
                .position(left[shared])?
                .cmp(&parent.children.position(right[shared])?),
        )
    }

    /// DOM live-range pre-removing steps, before detachment changes ancestry.
    pub fn removing(self, document: &BaseDocument, removed: NodeId) -> Self {
        let Some(parent) = document.get_node(removed).and_then(|n| n.parent) else {
            return self;
        };
        let Some(index) = document.nodes[parent].children.position(removed) else {
            return self;
        };
        if document.is_inclusive_ancestor(removed, self.node) {
            Self {
                node: parent,
                offset: index,
            }
        } else if self.node == parent && self.offset > index {
            Self {
                offset: self.offset - 1,
                ..self
            }
        } else {
            self
        }
    }

    /// DOM insertion, replace-data, split-text and normalize boundary steps.
    pub fn changed(self, document: &BaseDocument, mutation: RangeMutation) -> Self {
        match mutation {
            RangeMutation::Insert {
                parent,
                index,
                count,
            } if self.node == parent && self.offset > index => Self {
                offset: self.offset + count,
                ..self
            },
            RangeMutation::ReplaceData {
                node,
                offset,
                removed,
                added,
            } if self.node == node && self.offset > offset => {
                let end = offset + removed;
                Self {
                    offset: if self.offset <= end {
                        offset
                    } else {
                        self.offset - removed + added
                    },
                    ..self
                }
            }
            RangeMutation::Split {
                node,
                new_node,
                offset,
            } => {
                if self.node == node && self.offset > offset {
                    return Self {
                        node: new_node,
                        offset: self.offset - offset,
                    };
                }
                if let Some(parent) = document.nodes[node].parent
                    && self.node == parent
                    && document.nodes[parent].children.position(node) == self.offset.checked_sub(1)
                {
                    return Self {
                        offset: self.offset + 1,
                        ..self
                    };
                }
                self
            }
            RangeMutation::Merge { node, into, offset } => {
                if self.node == node {
                    return Self {
                        node: into,
                        offset: offset + self.offset,
                    };
                }
                if let Some(parent) = document.nodes[node].parent
                    && self.node == parent
                    && document.nodes[parent].children.position(node) == Some(self.offset)
                {
                    return Self { node: into, offset };
                }
                self
            }
            _ => self,
        }
    }
}

impl BaseDocument {
    pub(crate) fn notify_range_mutation(&self, mutation: RangeMutation) {
        if let Some(observer) = &self.tree_observer {
            observer.range_mutation(self, mutation);
        }
    }
}

#[cfg(test)]
mod tests;
