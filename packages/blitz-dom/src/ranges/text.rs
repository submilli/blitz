//! Character-data operations preserve live boundary semantics as one mutation.
//! <https://dom.spec.whatwg.org/#concept-cd-replace>
//! <https://dom.spec.whatwg.org/#concept-text-split>
//! <https://dom.spec.whatwg.org/#dom-node-normalize>

use super::RangeMutation;
use crate::{DocumentMutator, DomString, NodeId, dom_api::DomError, node::NodeData};

impl DocumentMutator<'_> {
    /// Replace UTF-16 code units, rejecting an offset beyond the current data.
    pub fn replace_character_data(
        &mut self,
        node: NodeId,
        offset: usize,
        count: usize,
        value: &DomString,
    ) -> Result<(), DomError> {
        let Some(data) = self.doc.character_data_dom(node) else {
            return Err(DomError::NotSupported);
        };
        let mut units = data.to_utf16().into_owned();
        if offset > units.len() {
            return Err(DomError::IndexSize);
        }
        let removed = count.min(units.len() - offset);
        let added = value.to_utf16();
        units.splice(offset..offset + removed, added.iter().copied());
        self.doc.notify_range_mutation(RangeMutation::ReplaceData {
            node,
            offset,
            removed,
            added: added.len(),
        });
        self.set_character_data_raw(node, DomString::from_utf16(units));
        Ok(())
    }

    /// DOM splitText, including the special parent boundary immediately after
    /// the original node. An unparented split only applies replace-data steps.
    pub fn split_text(&mut self, node: NodeId, offset: usize) -> Result<NodeId, DomError> {
        if !matches!(self.doc.nodes[node].data, NodeData::Text(_)) {
            return Err(DomError::NotSupported);
        }
        let units = self
            .doc
            .character_data_dom(node)
            .expect("Text nodes contain character data")
            .to_utf16()
            .into_owned();
        if offset > units.len() {
            return Err(DomError::IndexSize);
        }
        let tail = DomString::from_utf16(units[offset..].to_vec());
        let new_node = self
            .try_create_text_node(&tail)
            .map_err(|_| DomError::QuotaExceeded)?;
        self.set_node_document(new_node, self.doc.node_document(node));
        if self.doc.nodes[node].parent.is_some() {
            self.insert_nodes_after(node, &[new_node]);
            self.doc.notify_range_mutation(RangeMutation::Split {
                node,
                new_node,
                offset,
            });
        }
        self.replace_character_data(node, offset, units.len() - offset, &DomString::new())?;
        Ok(new_node)
    }

    /// DOM normalize: visit descendants iteratively; append data, move boundary
    /// points from merged siblings, then detach those siblings in tree order.
    pub fn normalize_subtree(&mut self, root: NodeId) {
        let mut pending: Vec<_> = self.doc.nodes[root]
            .children
            .iter()
            .rev()
            .copied()
            .collect();
        while let Some(node) = pending.pop() {
            if !matches!(self.doc.nodes[node].data, NodeData::Text(_)) {
                pending.extend(self.doc.nodes[node].children.iter().rev().copied());
                continue;
            }
            // A previous visit may already have merged this text sibling.
            if self.doc.nodes[node].parent.is_none() {
                continue;
            }
            let mut length = self
                .doc
                .character_data_dom(node)
                .expect("Text nodes contain character data")
                .to_utf16()
                .len();
            if length == 0 {
                self.remove_node(node);
                continue;
            }
            let mut siblings = Vec::new();
            let mut next = self.doc.nodes[node].forward(1).map(|n| n.id);
            let mut suffix = DomString::new();
            while let Some(id) = next {
                let Some(NodeData::Text(text)) = self.doc.get_node(id).map(|n| &n.data) else {
                    break;
                };
                suffix.push_dom(&text.content);
                siblings.push((id, text.content.to_utf16().len()));
                next = self.doc.nodes[id].forward(1).map(|n| n.id);
            }
            self.replace_character_data(node, length, 0, &suffix)
                .expect("the append offset is the current text length");
            for (next, added) in siblings {
                self.doc.notify_range_mutation(RangeMutation::Merge {
                    node: next,
                    into: node,
                    offset: length,
                });
                self.remove_node(next);
                length += added;
            }
        }
    }
}
