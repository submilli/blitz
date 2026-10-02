//! Manual assignments and the coalesced signal-slot list (DOM slotting algorithms).
use crate::mutations::MutationRecord;
use crate::{BaseDocument, DocumentMutator, NodeId};
use std::collections::{HashMap, HashSet};

impl DocumentMutator<'_> {
    /// Assign an ordered set of slottables. References are weak, as in the DOM standard.
    pub fn assign_slot(&mut self, slot: NodeId, nodes: &[NodeId]) {
        let mut affected = Vec::new();
        let old = std::mem::take(&mut self.doc.nodes[slot].manual_slottables);
        for id in old {
            if let Some(node) = self.doc.nodes.get_mut(id) {
                node.manual_slot = None;
            }
        }
        let mut unique = HashSet::new();
        for &id in nodes {
            if !unique.insert(id) {
                continue;
            }
            if let Some(previous) = self.doc.nodes[id].manual_slot {
                if let Some(previous_node) = self.doc.nodes.get_mut(previous) {
                    previous_node.manual_slottables.retain(|&n| n != id);
                    if !affected.contains(&previous) {
                        affected.push(previous);
                    }
                }
            }
            self.doc.nodes[id].manual_slot = Some(slot);
            self.doc.nodes[slot].manual_slottables.push(id);
        }
        if !affected.contains(&slot) {
            affected.push(slot);
        }
        // The tree algorithm signals changed slots in tree order.
        let mut roots = Vec::new();
        for slot in affected {
            if let Some(root) = self.doc.containing_shadow_root(slot)
                && !roots.contains(&root)
            {
                roots.push(root);
            }
        }
        for root in roots {
            self.doc.refresh_root_slots(root);
        }
    }
}

impl BaseDocument {
    pub fn has_slot_changes(&self) -> bool {
        !self.slot_changes.is_empty()
    }
    pub fn take_slot_changes(&mut self) -> Vec<NodeId> {
        std::mem::take(&mut self.slot_changes)
    }

    fn signal_slot(&mut self, slot: NodeId) {
        // At most one entry per live node; bounded by the document node limit.
        if !self.slot_changes.contains(&slot) {
            self.slot_changes.push(slot);
        }
    }

    fn refresh_slot_assignment(&mut self, slot: NodeId) {
        let assigned = self.assigned_nodes(slot);
        if self.nodes[slot].slot_assignment != assigned {
            self.nodes[slot].slot_assignment = assigned;
            self.signal_slot(slot);
        }
    }

    pub(crate) fn refresh_dirty_slots(&mut self) {
        for root in std::mem::take(&mut self.dirty_slot_roots) {
            if self.nodes.get(root).is_some() {
                self.refresh_root_slots(root);
            }
        }
        for slot in std::mem::take(&mut self.removed_slots) {
            if self.nodes.get(slot).is_some() {
                self.refresh_slot_assignment(slot);
            }
        }
    }

    fn refresh_root_slots(&mut self, root: NodeId) {
        let slots = self.slots_in(root);
        if self.nodes[root]
            .shadow_root_data
            .as_ref()
            .is_some_and(|data| data.manual_slots)
        {
            for slot in slots {
                self.refresh_slot_assignment(slot);
            }
            return;
        }
        let Some(host) = self.shadow_host_of(root) else {
            return;
        };
        let mut first = HashMap::new();
        for &slot in &slots {
            first.entry(self.slot_own_name(slot)).or_insert(slot);
        }
        let mut assigned: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
        for child in self.slottables(host) {
            if let Some(&slot) = first.get(&self.slot_name(child)) {
                assigned.entry(slot).or_default().push(child);
            }
        }
        for slot in slots {
            let nodes = assigned.remove(&slot).unwrap_or_default();
            if self.nodes[slot].slot_assignment != nodes {
                self.nodes[slot].slot_assignment = nodes;
                self.signal_slot(slot);
            }
        }
    }

    pub(crate) fn update_slots_after_mutation(&mut self, record: &MutationRecord) {
        if self.shadow_hosts.is_empty() {
            return;
        }
        let target = match record {
            MutationRecord::ChildList {
                target, removed, ..
            } => {
                self.signal_removed_slots(*target, removed);
                if self.is_html_slot(*target)
                    && self.assigned_nodes(*target).is_empty()
                    && self.containing_shadow_root(*target).is_some()
                {
                    self.signal_slot(*target);
                }
                *target
            }
            MutationRecord::Attributes {
                target,
                name,
                namespace,
                ..
            } if namespace.is_none() && (name == "slot" || name == "name") => *target,
            _ => return,
        };
        let Some(node) = self.nodes.get(target) else {
            return;
        };
        let root = self
            .shadow_root_of(target)
            .or_else(|| node.parent.and_then(|parent| self.shadow_root_of(parent)))
            .or_else(|| self.containing_shadow_root(target));
        if let Some(root) = root {
            if !self.dirty_slot_roots.contains(&root) {
                self.dirty_slot_roots.push(root);
            }
        }
    }

    fn signal_removed_slots(&mut self, target: NodeId, removed: &[NodeId]) {
        // Removing then reinserting the same slottable still signals a
        // change, even if this mutation batch restores the final order.
        if let Some(root) = self.shadow_root_of(target) {
            let removed: HashSet<_> = removed.iter().copied().collect();
            for slot in self.slots_in(root) {
                if self.nodes[slot]
                    .slot_assignment
                    .iter()
                    .any(|id| removed.contains(id))
                {
                    self.signal_slot(slot);
                }
            }
        }
        // Removed slots also lose their assignments, even when their former
        // shadow tree is no longer reachable through the slot's ancestors.
        let mut pending = removed.to_vec();
        while let Some(id) = pending.pop() {
            if let Some(node) = self.nodes.get(id) {
                pending.extend(node.children.iter().copied());
                if !node.slot_assignment.is_empty() {
                    if !self.removed_slots.contains(&id) {
                        self.removed_slots.push(id);
                    }
                    self.signal_slot(id);
                }
            }
        }
    }

    fn is_html_slot(&self, id: NodeId) -> bool {
        self.nodes
            .get(id)
            .and_then(|node| node.element_data())
            .is_some_and(|el| el.name.ns == crate::ns!(html) && &*el.name.local == "slot")
    }
}
