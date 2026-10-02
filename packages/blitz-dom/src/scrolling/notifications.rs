//! Bounded CSSOM notification ownership, coalesced until each target is dispatched.
use crate::{BaseDocument, NodeId, NodeLease};
use std::collections::{HashSet, VecDeque};

const MAX_PENDING_SCROLLS: usize = 65_536;

#[derive(Default)]
pub(crate) struct ScrollNotifications {
    targets: VecDeque<NodeLease>,
    ids: HashSet<NodeId>,
    overflowed: bool,
}
impl ScrollNotifications {
    fn push(&mut self, lease: NodeLease) {
        if self.ids.contains(&lease.id()) {
            return;
        }
        if self.targets.len() >= MAX_PENDING_SCROLLS {
            self.overflowed = true;
            return;
        }
        self.ids.insert(lease.id());
        self.targets.push_back(lease);
    }
    fn pop(&mut self) -> Option<NodeLease> {
        let target = self.targets.pop_front()?;
        self.ids.remove(&target.id());
        Some(target)
    }
}
impl BaseDocument {
    pub(super) fn queue_scroll_notification(&mut self, id: NodeId) {
        let id = if self.try_root_element().is_some_and(|node| node.id == id) {
            self.root_node_id
        } else {
            id
        };
        self.scroll_notifications.push(self.lease_node(id));
    }
    /// Snapshot the number of targets eligible for this rendering update.
    pub fn scroll_notification_count(&self) -> usize {
        self.scroll_notifications.targets.len()
    }
    /// Take the next target immediately before dispatch. Later queued targets
    /// stay coalesced when earlier callbacks scroll them again.
    pub fn next_scroll_notification(&mut self) -> Option<NodeLease> {
        self.scroll_notifications.pop()
    }
    /// A sticky resource error, consumed by the embedder's diagnostic boundary.
    pub fn take_scroll_notification_overflow(&mut self) -> bool {
        std::mem::take(&mut self.scroll_notifications.overflowed)
    }
}
