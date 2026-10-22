//! Bounded versioned records and identity discovery; no GC or script pointers.
use super::{AttributeId, AttributeNode, AttributeNodeError as Error, Location};
use crate::{DomString, NodeId, QualName};
use slotmap::SlotMap;
use std::collections::{HashMap, HashSet};

#[derive(Default)]
pub(crate) struct Store {
    records: SlotMap<AttributeId, AttributeNode>,
    attached: HashMap<(NodeId, QualName), AttributeId>,
}

impl Store {
    pub(super) fn len(&self) -> usize {
        self.records.len()
    }

    pub(super) fn get(&self, id: AttributeId) -> Option<&AttributeNode> {
        self.records.get(id)
    }

    pub(super) fn get_mut(&mut self, id: AttributeId) -> Option<&mut AttributeNode> {
        self.records.get_mut(id)
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = (AttributeId, &AttributeNode)> {
        self.records.iter()
    }

    pub(super) fn find(&self, owner: NodeId, name: &QualName) -> Option<AttributeId> {
        self.attached.get(&(owner, name.clone())).copied()
    }

    pub(super) fn ensure(
        &mut self,
        owner: NodeId,
        name: QualName,
        limit: usize,
    ) -> Result<AttributeId, Error> {
        if let Some(id) = self.find(owner, &name) {
            return Ok(id);
        }
        self.admit(limit)?;
        let id = self.records.insert(AttributeNode {
            name: name.clone(),
            location: Location::Attached(owner),
        });
        self.attached.insert((owner, name), id);
        Ok(id)
    }

    pub(super) fn create(
        &mut self,
        document: NodeId,
        name: QualName,
        value: DomString,
        limit: usize,
    ) -> Result<AttributeId, Error> {
        self.admit(limit)?;
        Ok(self.records.insert(AttributeNode {
            name,
            location: Location::Detached { document, value },
        }))
    }

    fn admit(&self, limit: usize) -> Result<(), Error> {
        if self.len() >= limit {
            Err(Error::Limit)
        } else {
            Ok(())
        }
    }

    pub(super) fn detach(
        &mut self,
        owner: NodeId,
        name: &QualName,
        document: NodeId,
        value: DomString,
    ) {
        let Some(id) = self.attached.remove(&(owner, name.clone())) else {
            return;
        };
        if let Some(record) = self.records.get_mut(id) {
            record.location = Location::Detached { document, value };
        }
    }

    pub(super) fn attach(&mut self, id: AttributeId, owner: NodeId) -> Result<(), Error> {
        let record = self.records.get(id).ok_or(Error::InvalidAttribute)?;
        if let Location::Attached(previous) = record.location {
            return if previous == owner {
                Ok(())
            } else {
                Err(Error::InUse)
            };
        }
        let key = (owner, record.name.clone());
        if self.attached.contains_key(&key) {
            return Err(Error::InUse);
        }
        let record = self.records.get_mut(id).ok_or(Error::InvalidAttribute)?;
        record.location = Location::Attached(owner);
        self.attached.insert(key, id);
        Ok(())
    }

    pub(super) fn reclaim(&mut self, live: &HashSet<AttributeId>) -> Vec<AttributeId> {
        let dead: Vec<_> = self
            .records
            .keys()
            .filter(|id| !live.contains(id))
            .collect();
        for &id in &dead {
            self.remove(id);
        }
        dead
    }

    pub(super) fn prune_nodes(&mut self, live: &HashSet<NodeId>) {
        let dead: Vec<_> = self
            .records
            .iter()
            .filter_map(|(id, record)| {
                let owner = match record.location {
                    Location::Attached(owner) => owner,
                    Location::Detached { document, .. } => document,
                };
                (!live.contains(&owner)).then_some(id)
            })
            .collect();
        for id in dead {
            self.remove(id);
        }
    }

    fn remove(&mut self, id: AttributeId) {
        let Some(record) = self.records.remove(id) else {
            return;
        };
        if let Location::Attached(owner) = record.location {
            let key = (owner, record.name);
            if self.attached.get(&key) == Some(&id) {
                self.attached.remove(&key);
            }
        }
    }
}
