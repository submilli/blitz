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
        old_value: Option<crate::DomString>,
    },
    CharacterData {
        target: NodeId,
        old_value: crate::DomString,
    },
}

impl MutationRecord {
    /// Conservative retained payload size, excluding allocator overhead.
    pub fn payload_bytes(&self) -> usize {
        match self {
            Self::ChildList { added, removed, .. } => (added.len().saturating_add(removed.len()))
                .saturating_mul(std::mem::size_of::<NodeId>()),
            Self::Attributes {
                name,
                namespace,
                old_value,
                ..
            } => name
                .len()
                .saturating_add(namespace.as_ref().map_or(0, String::len))
                .saturating_add(
                    old_value
                        .as_ref()
                        .map_or(0, crate::DomString::retained_bytes),
                ),
            Self::CharacterData { old_value, .. } => old_value.retained_bytes(),
        }
    }

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

const MAX_LOG_RECORDS: usize = 16_384;
const MAX_LOG_BYTES: usize = 16 * 1024 * 1024;

fn logged_bytes(record: &MutationRecord, ancestors: usize) -> usize {
    record
        .payload_bytes()
        .saturating_add(std::mem::size_of::<LoggedMutation>())
        .saturating_add(ancestors.saturating_mul(std::mem::size_of::<NodeId>()))
}

impl BaseDocument {
    /// Start or stop recording mutations. Stopping discards pending records.
    pub fn set_mutation_recording(&mut self, on: bool) {
        if !on {
            self.mutation_log_bytes = 0;
        }
        self.mutation_log = on.then(|| self.mutation_log.take().unwrap_or_default());
    }

    pub fn is_recording_mutations(&self) -> bool {
        self.mutation_log.is_some()
    }

    /// Slot assignment observes topology even without MutationObserver clients.
    pub(crate) fn observes_dom_changes(&self) -> bool {
        self.mutation_log.is_some() || !self.shadow_hosts.is_empty()
    }

    /// Sticky loss signal: the embedder must fail observation rather than report
    /// an incomplete stream as successful. Disabling or draining cannot clear it.
    pub fn mutation_recording_overflowed(&self) -> bool {
        self.mutation_log_overflowed
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
        self.mutation_log_bytes = 0;
        match &mut self.mutation_log {
            Some(log) => std::mem::take(log),
            None => Vec::new(),
        }
    }

    /// Check a known batch before starting a reversible edit. Even records that
    /// match no observer must fit in the native log until the embedder routes it.
    pub fn can_log_mutations(&self, batch: &[LoggedMutation]) -> bool {
        let Some(log) = &self.mutation_log else {
            return true;
        };
        !self.mutation_log_overflowed
            && batch.len() <= MAX_LOG_RECORDS.saturating_sub(log.len())
            && batch.iter().fold(self.mutation_log_bytes, |bytes, item| {
                bytes.saturating_add(logged_bytes(&item.record, item.ancestors.len()))
            }) <= MAX_LOG_BYTES
    }

    pub(crate) fn record_mutation(&mut self, record: MutationRecord) {
        self.update_slots_after_mutation(&record);
        self.log_mutation(record);
    }

    pub(crate) fn log_mutation(&mut self, record: MutationRecord) {
        if self.mutation_log.is_none() || self.mutation_log_overflowed {
            return;
        }
        let mut bytes = logged_bytes(&record, 0);
        if self
            .mutation_log
            .as_ref()
            .is_some_and(|log| log.len() >= MAX_LOG_RECORDS)
            || bytes > MAX_LOG_BYTES.saturating_sub(self.mutation_log_bytes)
        {
            self.fail_mutation_recording();
            return;
        }
        let mut ancestors = Vec::new();
        let mut current = Some(record.target());
        while let Some(id) = current {
            bytes = bytes.saturating_add(std::mem::size_of::<NodeId>());
            if bytes > MAX_LOG_BYTES.saturating_sub(self.mutation_log_bytes) {
                self.fail_mutation_recording();
                return;
            }
            ancestors.push(id);
            current = self.nodes.get(id).and_then(|n| n.parent);
        }
        if let Some(log) = &mut self.mutation_log {
            self.mutation_log_bytes += bytes;
            log.push(LoggedMutation { record, ancestors });
        }
    }

    fn fail_mutation_recording(&mut self) {
        self.mutation_log_overflowed = true;
        self.mutation_log_bytes = 0;
        if let Some(log) = &mut self.mutation_log {
            log.clear();
        }
    }

    /// (parent, previous sibling, next sibling) of `node`, if it has a parent.
    pub(crate) fn position_in_parent(
        &self,
        node: NodeId,
    ) -> Option<(NodeId, Option<NodeId>, Option<NodeId>)> {
        let parent = self.nodes[node].parent?;
        let siblings = &self.nodes[parent].children;
        let i = siblings.position(node)?;
        let previous = i.checked_sub(1).map(|j| siblings[j]);
        Some((parent, previous, siblings.get(i + 1).copied()))
    }
}

#[cfg(test)]
mod bounds_tests {
    use super::*;
    fn record(target: NodeId) -> MutationRecord {
        MutationRecord::CharacterData {
            target,
            old_value: String::new().into(),
        }
    }
    #[test]
    fn mutation_log_limit_is_sticky_and_drains_release_capacity() {
        let mut doc = BaseDocument::new(Default::default());
        doc.set_mutation_recording(true);
        for _ in 0..16_384 {
            doc.record_mutation(record(doc.root_node().id));
        }
        assert!(!doc.mutation_recording_overflowed());
        assert_eq!(doc.take_logged_mutations().len(), 16_384);
        for _ in 0..16_385 {
            doc.record_mutation(record(doc.root_node().id));
        }
        assert!(doc.mutation_recording_overflowed());
        assert!(doc.take_logged_mutations().is_empty());
        doc.set_mutation_recording(false);
        doc.set_mutation_recording(true);
        doc.record_mutation(record(doc.root_node().id));
        assert!(doc.mutation_recording_overflowed());
        assert!(doc.take_logged_mutations().is_empty());
    }
    #[test]
    fn batch_admission_preserves_pending_records_at_the_native_byte_limit() {
        let mut doc = BaseDocument::new(Default::default());
        doc.set_mutation_recording(true);
        let target = doc.root_node().id;
        doc.record_mutation(record(target));
        let batch = [LoggedMutation {
            record: MutationRecord::CharacterData {
                target,
                old_value: "x".repeat(MAX_LOG_BYTES).into(),
            },
            ancestors: vec![target],
        }];
        assert!(!doc.can_log_mutations(&batch));
        assert!(!doc.mutation_recording_overflowed());
        assert_eq!(doc.take_mutation_records(), [record(target)]);
    }

    #[test]
    fn mutation_log_bounds_retained_text() {
        let mut doc = BaseDocument::new(Default::default());
        doc.set_mutation_recording(true);
        let target = doc.root_node().id;
        doc.record_mutation(MutationRecord::CharacterData {
            target,
            old_value: "x".repeat(16 * 1024 * 1024).into(),
        });
        assert!(doc.mutation_recording_overflowed());
        assert!(doc.take_logged_mutations().is_empty());
    }
}
