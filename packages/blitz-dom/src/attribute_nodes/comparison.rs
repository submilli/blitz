//! DOM position includes attribute order without inserting attributes in the tree.
//! https://dom.spec.whatwg.org/#dom-node-comparedocumentposition
use super::NodeIdentity;
use crate::{BaseDocument, NodeId};
use slotmap::Key;

const DISCONNECTED: u16 = 1;
const PRECEDING: u16 = 2;
const FOLLOWING: u16 = 4;
const CONTAINS: u16 = 8;
const CONTAINED_BY: u16 = 16;
const IMPLEMENTATION_SPECIFIC: u16 = 32;

struct Position {
    identity: NodeIdentity,
    anchor: Option<NodeId>,
    root: (u8, u64),
}

impl BaseDocument {
    /// Return the DOM DOCUMENT_POSITION bitmask, or None for an invalid identity.
    /// Disconnected ordering uses stable versioned IDs, never object addresses.
    pub fn compare_node_positions(&self, left: NodeIdentity, right: NodeIdentity) -> Option<u16> {
        let left = self.node_position(left)?;
        let right = self.node_position(right)?;
        if left.identity == right.identity {
            return Some(0);
        }
        if let (NodeIdentity::Attribute(a), NodeIdentity::Attribute(b)) =
            (left.identity, right.identity)
            && left.anchor.is_some()
            && left.anchor == right.anchor
        {
            let owner = left.anchor?;
            let attributes = self.get_node(owner)?.element_data()?.attrs();
            let index = |id| {
                let name = self.attribute_node(id)?.name();
                attributes
                    .iter()
                    .position(|attribute| attribute.name == *name)
            };
            return Some(
                IMPLEMENTATION_SPECIFIC
                    | if index(b)? < index(a)? {
                        PRECEDING
                    } else {
                        FOLLOWING
                    },
            );
        }
        if left.root != right.root {
            return Some(
                DISCONNECTED
                    | IMPLEMENTATION_SPECIFIC
                    | if left.root < right.root {
                        FOLLOWING
                    } else {
                        PRECEDING
                    },
            );
        }
        let a = left.anchor?;
        let b = right.anchor?;
        if (self.is_inclusive_ancestor(b, a) && matches!(right.identity, NodeIdentity::Tree(_)))
            || (a == b && matches!(left.identity, NodeIdentity::Attribute(_)))
        {
            return Some(PRECEDING | CONTAINS);
        }
        if (self.is_inclusive_ancestor(a, b) && matches!(left.identity, NodeIdentity::Tree(_)))
            || (a == b && matches!(right.identity, NodeIdentity::Attribute(_)))
        {
            return Some(FOLLOWING | CONTAINED_BY);
        }
        Some(
            if self.is_inclusive_ancestor(b, a) || self.compare_document_order(b, a).is_lt() {
                PRECEDING
            } else {
                FOLLOWING
            },
        )
    }

    fn node_position(&self, identity: NodeIdentity) -> Option<Position> {
        let anchor = match identity {
            NodeIdentity::Tree(id) => {
                self.get_node(id)?;
                Some(id)
            }
            NodeIdentity::Attribute(id) => {
                let owner = self.attribute_node(id)?.owner_element();
                if let Some(owner) = owner {
                    self.get_node(owner)?.element_data()?;
                } else {
                    self.attribute_node_document(id)?;
                }
                owner
            }
        };
        let root = match (anchor, identity) {
            (Some(id), _) => (0, self.tree_root(id).as_u64()),
            (None, NodeIdentity::Attribute(id)) => (1, id.data().as_ffi()),
            (None, NodeIdentity::Tree(_)) => return None,
        };
        Some(Position {
            identity,
            anchor,
            root,
        })
    }
}
