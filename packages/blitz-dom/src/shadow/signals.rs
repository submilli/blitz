//! Keep the first signal's queue position. A provisional Range split and a
//! subsequent independent edit can each be the first surviving cause.
use crate::{NodeId, dom_api::DomError};
use std::collections::HashSet;

#[derive(Default)]
pub(crate) struct SlotSignals {
    entries: Vec<(NodeId, Option<u64>)>,
    active: Option<u64>,
    serial: u64,
}

impl SlotSignals {
    pub(crate) fn begin_split(&mut self) -> Result<u64, DomError> {
        self.serial = self.serial.checked_add(1).ok_or(DomError::QuotaExceeded)?;
        self.active = Some(self.serial);
        Ok(self.serial)
    }

    pub(crate) fn editing_split(&self) -> bool {
        self.active.is_some()
    }

    pub(crate) fn during_split(&mut self, split: Option<u64>) {
        self.active = split;
    }

    pub(crate) fn push(&mut self, slot: NodeId) {
        if !self
            .entries
            .iter()
            .any(|&(id, cause)| id == slot && (cause.is_none() || cause == self.active))
        {
            self.entries.push((slot, self.active));
        }
    }

    pub(crate) fn finish_split(&mut self, split: u64, restored: bool) {
        if restored {
            self.entries.retain(|&(_, cause)| cause != Some(split));
        } else if self.entries.iter().any(|&(_, cause)| cause == Some(split)) {
            let mut committed = HashSet::new();
            self.entries.retain_mut(|(slot, cause)| {
                if *cause == Some(split) {
                    *cause = None;
                }
                !committed.contains(slot) && (cause.is_some() || committed.insert(*slot))
            });
        }
    }

    pub(crate) fn take(&mut self) -> Vec<NodeId> {
        let mut seen = HashSet::new();
        std::mem::take(&mut self.entries)
            .into_iter()
            .filter_map(|(id, _)| seen.insert(id).then_some(id))
            .collect()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(crate) fn roots(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.entries.iter().map(|&(id, _)| id)
    }

    pub(crate) fn remove(&mut self, slot: NodeId) {
        self.entries.retain(|&(id, _)| id != slot);
    }
}
