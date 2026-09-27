//! Mutation records (the data behind `MutationObserver`).
//!
//! While recording is on, the mutator appends a record for every tree,
//! attribute and character-data change, including those made by the parser.
//! The embedder drains them and delivers them to its observers.

use crate::{BaseDocument, NodeId};

#[derive(Debug, Clone, PartialEq)]
pub enum MutationRecord {
    ChildList {
        target: NodeId,
        added: Vec<NodeId>,
        removed: Vec<NodeId>,
        previous_sibling: Option<NodeId>,
        next_sibling: Option<NodeId>,
    },
    Attributes {
        target: NodeId,
        name: String,
        namespace: Option<String>,
        old_value: Option<String>,
    },
    CharacterData {
        target: NodeId,
        old_value: String,
    },
}

impl MutationRecord {
    pub fn target(&self) -> NodeId {
        match self {
            Self::ChildList { target, .. }
            | Self::Attributes { target, .. }
            | Self::CharacterData { target, .. } => *target,
        }
    }
}

/// A record with the target's inclusive ancestors when it was made, target
/// first. Observers match on these: a node's position at the time of the
/// mutation decides which observers see it, not where it is later.
#[derive(Debug, Clone, PartialEq)]
pub struct LoggedMutation {
    pub record: MutationRecord,
    pub ancestors: Vec<NodeId>,
}

impl BaseDocument {
    /// Start or stop recording mutations. Stopping discards pending records.
    pub fn set_mutation_recording(&mut self, on: bool) {
        self.mutation_log = on.then(|| self.mutation_log.take().unwrap_or_default());
    }

    pub fn is_recording_mutations(&self) -> bool {
        self.mutation_log.is_some()
    }

    /// Records since the last call, oldest first.
    pub fn take_mutation_records(&mut self) -> Vec<MutationRecord> {
        self.take_logged_mutations()
            .into_iter()
            .map(|m| m.record)
            .collect()
    }

    /// Records since the last call with their targets' ancestors, oldest
    /// first.
    pub fn take_logged_mutations(&mut self) -> Vec<LoggedMutation> {
        match &mut self.mutation_log {
            Some(log) => std::mem::take(log),
            None => Vec::new(),
        }
    }

    pub(crate) fn record_mutation(&mut self, record: MutationRecord) {
        if self.mutation_log.is_none() {
            return;
        }
        let mut ancestors = Vec::new();
        let mut current = Some(record.target());
        while let Some(id) = current {
            ancestors.push(id);
            current = self.nodes.get(id).and_then(|n| n.parent);
        }
        if let Some(log) = &mut self.mutation_log {
            log.push(LoggedMutation { record, ancestors });
        }
    }

    /// (parent, previous sibling, next sibling) of `node`, if it has a parent.
    pub(crate) fn position_in_parent(
        &self,
        node: NodeId,
    ) -> Option<(NodeId, Option<NodeId>, Option<NodeId>)> {
        let parent = self.nodes[node].parent?;
        let siblings = &self.nodes[parent].children;
        let i = siblings.iter().position(|&c| c == node)?;
        let previous = i.checked_sub(1).map(|j| siblings[j]);
        Some((parent, previous, siblings.get(i + 1).copied()))
    }
}
