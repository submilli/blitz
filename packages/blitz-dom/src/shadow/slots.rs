//! Manual assignments and the coalesced signal-slot list (DOM slotting algorithms).
use crate::mutations::MutationRecord;
use crate::{BaseDocument, NodeId};
use std::collections::{HashMap, HashSet};

impl BaseDocument {
    pub fn has_slot_changes(&self) -> bool {
        !self.slot_changes.is_empty()
    }
    pub fn take_slot_changes(&mut self) -> Vec<NodeId> {
        self.slot_changes.take().map(|(_, slot)| slot).collect()
    }

    /// Snapshot the signal slots with their original positions in DOM-thread
    /// order, so linked documents can deliver one ordered notification.
    pub fn take_ordered_slot_changes(&mut self) -> Vec<(u128, NodeId)> {
        self.slot_changes.take().collect()
    }

    /// Cancel signals present at retirement and stop generating new ones.
    /// An active document may still pass an existing signal through this arena.
    pub fn retire_slot_changes(&mut self) {
        self.slot_changes.retire();
    }

    /// At adoption commit, move pending notifications with the logical slot.
    /// A discarded staged import never changes either document's queue.
    pub fn transfer_slot_changes(
        &mut self,
        target: &mut BaseDocument,
        ids: &HashMap<NodeId, NodeId>,
    ) {
        self.slot_changes.transfer(&mut target.slot_changes, ids);
    }

    fn signal_slot(&mut self, slot: NodeId) {
        self.slot_changes.push(slot);
    }

    fn refresh_slot_assignment(&mut self, slot: NodeId) {
        let assigned = self.current_slot_members(slot);
        self.update_slot_list(slot, assigned);
    }

    // A mutation batch may query fallback or removed slots before its root's
    // settled list has been rebuilt.
    fn current_slot_members(&self, slot: NodeId) -> Vec<NodeId> {
        if let Some(root) = self.containing_shadow_root(slot)
            && self.nodes[root]
                .shadow_root_data
                .as_ref()
                .is_some_and(|data| data.manual_slots)
            && let Some(host) = self.shadow_host_of(root)
        {
            if !self.nodes[slot]
                .manual_assignment
                .as_ref()
                .is_some_and(|g| g.has_members())
            {
                return Vec::new();
            }
            return self
                .manual_slot_lists(host, &[slot])
                .pop()
                .unwrap_or_default();
        }
        self.assigned_nodes(slot)
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
        let Some(host) = self.shadow_host_of(root) else {
            return;
        };
        if self.nodes[root]
            .shadow_root_data
            .as_ref()
            .is_some_and(|data| data.manual_slots)
        {
            let assigned = self.manual_slot_lists(host, &slots);
            for (slot, members) in slots.into_iter().zip(assigned) {
                self.update_slot_list(slot, members);
            }
            return;
        }
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
            self.update_slot_list(slot, nodes);
        }
    }

    pub(super) fn refresh_manual_assignment(
        &mut self,
        root: NodeId,
        affected: &HashSet<(usize, NodeId)>,
    ) {
        let Some(host) = self.shadow_host_of(root) else {
            return;
        };
        let slots = self.slots_in(root);
        // Named distribution is independent of manual membership, but an
        // explicit membership change still signals each affected shadow slot.
        if !self.nodes[root]
            .shadow_root_data
            .as_ref()
            .is_some_and(|r| r.manual_slots)
        {
            for slot in slots {
                if affected.contains(&(self.id(), slot)) {
                    self.signal_slot(slot);
                }
            }
            return;
        }
        let assigned = self.manual_slot_lists(host, &slots);
        for (slot, list) in slots.into_iter().zip(assigned) {
            if affected.contains(&(self.id(), slot)) {
                self.signal_slot(slot);
            }
            self.update_slot_list(slot, list);
        }
    }

    fn update_slot_list(&mut self, slot: NodeId, nodes: Vec<NodeId>) {
        if self.nodes[slot].slot_assignment != nodes {
            self.nodes[slot].slot_assignment = nodes;
            self.signal_slot(slot);
        }
    }

    /// Resolve all generations in one pass over the host, preserving assignment
    /// order even if separately transferred children have changed tree order.
    pub(super) fn manual_slot_lists(&self, host: NodeId, slots: &[NodeId]) -> Vec<Vec<NodeId>> {
        let positions: HashMap<_, _> = slots
            .iter()
            .enumerate()
            .filter_map(|(index, &slot)| {
                self.nodes[slot]
                    .manual_assignment
                    .as_ref()
                    .map(|group| (std::sync::Arc::as_ptr(group), index))
            })
            .collect();
        let mut lists = vec![Vec::new(); slots.len()];
        for child in self.slottables(host) {
            if let Some((member, _)) = &self.nodes[child].manual_slot
                && let Some(&index) = positions.get(&member.group.as_ptr())
            {
                lists[index].push((member.rank, child));
            }
        }
        lists
            .into_iter()
            .map(|mut list| {
                list.sort_unstable_by_key(|&(rank, _)| rank);
                list.into_iter().map(|(_, node)| node).collect()
            })
            .collect()
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
                    && self.current_slot_members(*target).is_empty()
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
        // A nested host's light tree is also part of its enclosing shadow tree.
        // Inserting a slot there can change both roots' settled projections.
        let roots = [
            self.shadow_root_of(target),
            node.parent.and_then(|parent| self.shadow_root_of(parent)),
            self.containing_shadow_root(target),
        ];
        for root in roots.into_iter().flatten() {
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
        let manual = self
            .containing_shadow_root(target)
            .and_then(|root| self.nodes[root].shadow_root_data.as_ref())
            .is_some_and(|r| r.manual_slots);
        let mut pending = removed.to_vec();
        while let Some(id) = pending.pop() {
            if let Some(node) = self.nodes.get(id) {
                pending.extend(node.children.iter().copied());
                if !node.slot_assignment.is_empty()
                    || (manual
                        && node
                            .manual_assignment
                            .as_ref()
                            .is_some_and(|group| group.has_members()))
                {
                    if !self.removed_slots.contains(&id) {
                        self.removed_slots.push(id);
                    }
                    self.signal_slot(id);
                }
            }
        }
    }

    pub(super) fn is_html_slot(&self, id: NodeId) -> bool {
        self.nodes
            .get(id)
            .and_then(|node| node.element_data())
            .is_some_and(|el| el.name.ns == crate::ns!(html) && &*el.name.local == "slot")
    }
}
