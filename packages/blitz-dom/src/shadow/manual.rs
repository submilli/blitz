//! A slot owns one assignment generation. Each member holds a weak link and its
//! argument position. No generation retains a DOM node or a document.
use super::*;
use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, Mutex, PoisonError, Weak,
        atomic::{AtomicUsize, Ordering},
    },
};

#[derive(Debug)]
pub(crate) struct ManualAssignment {
    owner: Mutex<(usize, NodeId)>,
    members: AtomicUsize,
}

impl ManualAssignment {
    pub(super) fn has_members(&self) -> bool {
        self.members.load(Ordering::Relaxed) != 0
    }

    fn owner(&self) -> (usize, NodeId) {
        *self.owner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn set_owner(&self, arena: usize, node: NodeId) {
        *self.owner.lock().unwrap_or_else(PoisonError::into_inner) = (arena, node);
    }
}

#[derive(Debug)]
pub(crate) struct ManualMember {
    pub(super) group: Weak<ManualAssignment>,
    pub(super) rank: usize,
}

impl ManualMember {
    pub(crate) fn local_slot(&self, arena: usize, hint: NodeId) -> NodeId {
        self.group
            .upgrade()
            .map(|group| group.owner())
            .filter(|(owner, _)| *owner == arena)
            .map_or(hint, |(_, slot)| slot)
    }
}

impl Drop for ManualMember {
    fn drop(&mut self) {
        if let Some(group) = self.group.upgrade() {
            group.members.fetch_sub(1, Ordering::Relaxed);
        }
    }
}

impl BaseDocument {
    /// Native arena of the previous slot, even when its slottable is detached.
    pub fn manual_slot_document(&self, id: NodeId) -> Option<usize> {
        let (member, _) = self.nodes.get(id)?.manual_slot.as_ref()?;
        Some(member.group.upgrade()?.owner().0)
    }

    pub(super) fn manual_slot_in_root(&self, node: NodeId, root: NodeId) -> Option<NodeId> {
        let (member, hint) = self.nodes[node].manual_slot.as_ref()?;
        let group = member.group.upgrade()?;
        // A staged tree can project its own slot before commit publishes its
        // location. Arena-local IDs are hints and must match the generation.
        let slot = if self
            .nodes
            .get(*hint)
            .and_then(|n| n.manual_assignment.as_ref())
            .is_some_and(|a| Arc::ptr_eq(a, &group))
        {
            *hint
        } else {
            let (arena, slot) = group.owner();
            if arena != self.id()
                || !self
                    .nodes
                    .get(slot)?
                    .manual_assignment
                    .as_ref()
                    .is_some_and(|a| Arc::ptr_eq(a, &group))
            {
                return None;
            }
            slot
        };
        (self.containing_shadow_root(slot) == Some(root)).then_some(slot)
    }

    /// Publish endpoint locations only after a staged transfer commits.
    pub fn transfer_manual_slots(
        &mut self,
        target: &Self,
        pairs: impl IntoIterator<Item = (NodeId, NodeId)>,
    ) {
        for (old, new) in pairs {
            let Some(source) = self.nodes.get_mut(old) else {
                continue;
            };
            if let Some(group) = source.manual_assignment.take() {
                group.set_owner(target.id(), new);
            }
            source.manual_slot = None;
        }
    }
}

impl DocumentMutator<'_> {
    /// Assign local slottables in first-occurrence order.
    pub fn assign_slot(&mut self, slot: NodeId, nodes: &[NodeId]) {
        let members: Vec<_> = nodes.iter().map(|&node| (0, node)).collect();
        assign_manual_slot(&mut [self.doc], slot, &members);
    }
}

/// Replace the first document's slot membership. Each argument names a document
/// by slice index. Include former slot owners for cross-document notification.
pub fn assign_manual_slot(docs: &mut [&mut BaseDocument], slot: NodeId, nodes: &[(usize, NodeId)]) {
    let Some(target) = docs.first() else { return };
    if !target.is_html_slot(slot) {
        return;
    }
    let target_id = target.id();
    let mut unique = HashSet::new();
    let nodes: Vec<_> = nodes
        .iter()
        .copied()
        .filter(|&entry| unique.insert(entry))
        .collect();
    if nodes.iter().any(|&(arena, node)| {
        !docs.get(arena).is_some_and(|doc| {
            doc.get_node(node)
                .is_some_and(|n| matches!(n.data, NodeData::Element(_) | NodeData::Text(_)))
        })
    }) {
        return;
    }
    if same_members(docs, slot, &nodes) {
        return;
    }

    let group = Arc::new(ManualAssignment {
        owner: Mutex::new((target_id, slot)),
        members: AtomicUsize::new(nodes.len()),
    });
    let mut affected = vec![group.clone()];
    docs[0].nodes[slot].manual_assignment = Some(group.clone());
    for (rank, (arena, id)) in nodes.into_iter().enumerate() {
        let child = &mut docs[arena].nodes[id];
        if let Some(old) = child
            .manual_slot
            .as_ref()
            .and_then(|(member, _)| member.group.upgrade())
        {
            affected.push(old);
        }
        child.manual_slot = Some((
            Arc::new(ManualMember {
                group: Arc::downgrade(&group),
                rank,
            }),
            slot,
        ));
    }
    refresh_affected(docs, affected);
}

fn same_members(docs: &[&mut BaseDocument], slot: NodeId, nodes: &[(usize, NodeId)]) -> bool {
    let Some(group) = &docs[0].nodes[slot].manual_assignment else {
        return nodes.is_empty();
    };
    if group.members.load(Ordering::Relaxed) != nodes.len() {
        return false;
    }
    let mut last = None;
    nodes.iter().all(|&(arena, id)| {
        let Some((member, _)) = &docs[arena].nodes[id].manual_slot else {
            return false;
        };
        let matches = member.group.as_ptr() == Arc::as_ptr(group)
            && last.is_none_or(|rank| rank < member.rank);
        last = Some(member.rank);
        matches
    })
}

fn refresh_affected(docs: &mut [&mut BaseDocument], affected: Vec<Arc<ManualAssignment>>) {
    let arenas: HashMap<_, _> = docs
        .iter()
        .enumerate()
        .map(|(i, doc)| (doc.id(), i))
        .collect();
    let mut roots = Vec::new();
    let mut seen_roots = HashSet::new();
    let mut slots = HashSet::new();
    for group in affected {
        let (arena, slot) = group.owner();
        let Some(&index) = arenas.get(&arena) else {
            continue;
        };
        let doc = &docs[index];
        if !doc
            .nodes
            .get(slot)
            .and_then(|n| n.manual_assignment.as_ref())
            .is_some_and(|current| Arc::ptr_eq(current, &group))
            || !slots.insert((arena, slot))
        {
            continue;
        }
        if let Some(root) = doc.containing_shadow_root(slot)
            && seen_roots.insert((index, root))
        {
            roots.push((index, root));
        }
    }
    let mut updated = HashSet::new();
    for (index, root) in roots {
        let doc = &mut docs[index];
        doc.refresh_manual_assignment(root, &slots);
        if updated.insert(index) {
            doc.dom_generation = doc.dom_generation.wrapping_add(1);
        }
    }
}
