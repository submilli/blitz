//! DOM subtree equality, shared by script bindings and native callers.
use crate::{Attribute, BaseDocument, ElementData, NodeData, NodeId};

impl BaseDocument {
    /// Compare ordinary DOM children, node kinds, names, attributes and data.
    /// Owner documents, attachment, form state, template contents and shadow
    /// trees do not participate. Attribute order and prefixes are ignored.
    /// Missing or stale node IDs compare unequal. Work and temporary storage
    /// are bounded by the two admitted trees and their attribute lists.
    ///
    /// https://dom.spec.whatwg.org/#concept-node-equals
    pub fn is_equal_node(&self, id: NodeId, other: &Self, other_id: NodeId) -> bool {
        let mut pending = vec![(id, other_id)];
        while let Some((id, other_id)) = pending.pop() {
            let (Some(a), Some(b)) = (self.get_node(id), other.get_node(other_id)) else {
                return false;
            };
            if !data_equal(&a.data, &b.data) || a.children.len() != b.children.len() {
                return false;
            }
            pending.extend(a.children.iter().copied().zip(b.children.iter().copied()));
        }
        true
    }
}

fn data_equal(a: &NodeData, b: &NodeData) -> bool {
    match (a, b) {
        (NodeData::Element(a), NodeData::Element(b)) => a.name == b.name && attributes_equal(a, b),
        (NodeData::Text(a), NodeData::Text(b)) => a.content == b.content,
        (NodeData::Comment { contents: a }, NodeData::Comment { contents: b }) => a == b,
        (
            NodeData::ProcessingInstruction {
                target: target_a,
                contents: contents_a,
            },
            NodeData::ProcessingInstruction {
                target: target_b,
                contents: contents_b,
            },
        ) => target_a == target_b && contents_a == contents_b,
        (
            NodeData::Doctype {
                name: name_a,
                public_id: public_a,
                system_id: system_a,
            },
            NodeData::Doctype {
                name: name_b,
                public_id: public_b,
                system_id: system_b,
            },
        ) => name_a == name_b && public_a == public_b && system_a == system_b,
        (NodeData::DocumentFragment, NodeData::DocumentFragment)
        | (NodeData::Document(_), NodeData::Document(_)) => true,
        _ => false,
    }
}

fn attributes_equal(a: &ElementData, b: &ElementData) -> bool {
    if a.attrs().len() != b.attrs().len() {
        return false;
    }
    sorted_attributes(a)
        .into_iter()
        .zip(sorted_attributes(b))
        .all(|(a, b)| a.name.ns == b.name.ns && a.name.local == b.name.local && a.value == b.value)
}

fn sorted_attributes(element: &ElementData) -> Vec<&Attribute> {
    let mut attributes: Vec<&Attribute> = element.attrs().iter().collect();
    attributes.sort_unstable_by(|a, b| {
        a.name
            .ns
            .cmp(&b.name.ns)
            .then_with(|| a.name.local.cmp(&b.name.local))
            .then_with(|| a.value.cmp(&b.value))
    });
    attributes
}

#[cfg(test)]
mod tests;
