//! Attribute identities are distinct from tree-node identities.
//! https://dom.spec.whatwg.org/#interface-attr
mod mutation;
mod projection;
mod store;

use crate::{DomString, NodeId, QualName};
pub(crate) use store::Store;

slotmap::new_key_type! {
    /// Versioned identity of an attribute object, never a tree node ID.
    pub struct AttributeId;
}

/// The native data of an Attr facade. Attached values remain on the element.
#[derive(Debug)]
pub struct AttributeNode {
    name: QualName,
    location: Location,
}

#[derive(Debug)]
enum Location {
    Attached(NodeId),
    Detached { document: NodeId, value: DomString },
}

impl AttributeNode {
    pub fn name(&self) -> &QualName {
        &self.name
    }

    pub fn owner_element(&self) -> Option<NodeId> {
        match self.location {
            Location::Attached(owner) => Some(owner),
            Location::Detached { .. } => None,
        }
    }
}

/// Failures independent of any script engine or host authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttributeNodeError {
    InvalidNode,
    InvalidAttribute,
    InUse,
    NotFound,
    Limit,
}

#[cfg(test)]
mod tests;
