//! Coalesced slot notifications. A provisional Range split and a subsequent
//! independent edit can each be the first surviving cause.
use crate::NodeId;
use std::{
    cell::Cell,
    collections::{HashMap, HashSet},
    rc::Rc,
};

thread_local! {
    // Arenas on the same DOM thread can exchange slots before delivery.
    static NEXT_ORDER: Cell<u128> = const { Cell::new(0) };
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Outcome {
    #[default]
    Pending,
    Committed,
    Canceled,
}

#[derive(Clone, Default)]
pub(crate) struct Split(Rc<Cell<Outcome>>);

impl Split {
    fn same(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

struct Signal {
    slot: NodeId,
    order: u128,
    split: Option<Split>,
}

impl Signal {
    fn canceled(&self) -> bool {
        self.split
            .as_ref()
            .is_some_and(|s| s.0.get() == Outcome::Canceled)
    }

    fn committed(&self) -> bool {
        self.split
            .as_ref()
            .is_none_or(|s| s.0.get() == Outcome::Committed)
    }
}

#[derive(Default)]
pub(crate) struct SlotSignals {
    entries: Vec<Signal>,
    active: Option<Split>,
    retired: bool,
}

impl SlotSignals {
    pub(crate) fn begin_split(&mut self) -> Split {
        let split = Split::default();
        self.active = Some(split.clone());
        split
    }

    pub(crate) fn editing_split(&self) -> bool {
        self.active.is_some()
    }

    pub(crate) fn during_split(&mut self, split: Option<Split>) {
        self.active = split;
    }

    pub(crate) fn push(&mut self, slot: NodeId) {
        if self.retired {
            return;
        }
        if self.entries.iter().any(|entry| {
            entry.slot == slot
                && (entry.committed()
                    || entry
                        .split
                        .as_ref()
                        .zip(self.active.as_ref())
                        .is_some_and(|(a, b)| a.same(b)))
        }) {
            return;
        }
        self.entries.retain(|entry| !entry.canceled());
        let order = NEXT_ORDER.with(|next| next.replace(next.get() + 1));
        self.entries.push(Signal {
            slot,
            order,
            split: self.active.clone(),
        });
    }

    pub(crate) fn finish_split(&mut self, split: Split, restored: bool) {
        // A blur callback may already have moved the signal to another arena.
        split.0.set(if restored {
            Outcome::Canceled
        } else {
            Outcome::Committed
        });
        self.compact();
    }

    fn compact(&mut self) {
        let mut committed = HashSet::new();
        self.entries.retain(|entry| {
            !entry.canceled()
                && !committed.contains(&entry.slot)
                && (!entry.committed() || committed.insert(entry.slot))
        });
    }

    pub(crate) fn transfer(&mut self, target: &mut Self, ids: &HashMap<NodeId, NodeId>) {
        let before = target.entries.len();
        self.entries.retain_mut(|entry| {
            if entry.canceled() {
                return false;
            }
            let Some(&slot) = ids.get(&entry.slot) else {
                return true;
            };
            target.entries.push(Signal {
                slot,
                order: entry.order,
                split: entry.split.take(),
            });
            false
        });
        if target.entries.len() != before {
            target.entries.sort_unstable_by_key(|entry| entry.order);
            target.compact();
        }
    }

    pub(crate) fn retire(&mut self) {
        if !self.retired {
            self.entries.clear();
            self.retired = true;
        }
    }

    pub(crate) fn take(&mut self) -> impl Iterator<Item = (u128, NodeId)> {
        let mut seen = HashSet::new();
        std::mem::take(&mut self.entries)
            .into_iter()
            .filter_map(move |entry| {
                (!entry.canceled() && seen.insert(entry.slot)).then_some((entry.order, entry.slot))
            })
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.roots().next().is_none()
    }

    pub(crate) fn roots(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.entries
            .iter()
            .filter(|entry| !entry.canceled())
            .map(|entry| entry.slot)
    }

    pub(crate) fn remove(&mut self, slot: NodeId) {
        self.entries.retain(|entry| entry.slot != slot);
    }
}
