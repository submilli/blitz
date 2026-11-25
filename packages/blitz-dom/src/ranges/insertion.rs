//! A foreign Range insertion may split Text, run blur handlers, and then
//! refuse the transfer. Only the insertion's own untouched split is reversible.
use super::RangeMutation;
use crate::{BaseDocument, NodeId, NodeLease, dom_api::DomError};
use std::{
    cell::Cell,
    rc::{Rc, Weak},
};

pub(crate) struct SplitWatch {
    prefix: NodeId,
    suffix: NodeId,
    edited: Cell<bool>,
}

#[must_use = "finish the split after insertion or refusal"]
pub struct InsertionSplit {
    parent: NodeLease,
    prefix: NodeLease,
    suffix: NodeLease,
    watch: Rc<SplitWatch>,
    signals: crate::shadow::Split,
}

impl InsertionSplit {
    pub fn suffix(&self) -> NodeId {
        self.suffix.id()
    }
}

impl BaseDocument {
    pub(crate) fn split_text_edited(&self, node: NodeId) {
        if self.slot_changes.editing_split() {
            return;
        }
        for watch in self.insertion_splits.iter().filter_map(Weak::upgrade) {
            if watch.prefix == node || watch.suffix == node {
                watch.edited.set(true);
            }
        }
    }

    pub(crate) fn split_subtree_removing(&self, root: NodeId) {
        if self.slot_changes.editing_split() {
            return;
        }
        for watch in self.insertion_splits.iter().filter_map(Weak::upgrade) {
            if !watch.edited.get()
                && [watch.prefix, watch.suffix]
                    .into_iter()
                    .any(|id| self.get_node(id).is_some() && self.is_inclusive_ancestor(root, id))
            {
                watch.edited.set(true);
            }
        }
    }

    /// The caller retains this token in the originating document through blur.
    /// During the synchronous call slot assignment is visible, while slotchange
    /// delivery waits for the next microtask checkpoint.
    pub fn split_text_for_insertion(
        &mut self,
        prefix: NodeId,
        offset: usize,
    ) -> Result<InsertionSplit, DomError> {
        let parent = self
            .get_node(prefix)
            .and_then(|n| n.parent)
            .ok_or(DomError::NotFound)?;
        let signals = self.slot_changes.begin_split();
        let suffix = {
            let mut edit = self.mutate();
            // The final child text is unchanged. Ignore the intermediate
            // duplicated suffix so it cannot reset textarea selection.
            edit.unchanged_textarea_children = Some(parent);
            edit.split_text(prefix, offset)
        };
        self.slot_changes.during_split(None);
        let suffix = suffix?;
        let watch = Rc::new(SplitWatch {
            prefix,
            suffix,
            edited: Cell::new(false),
        });
        self.insertion_splits
            .retain(|entry| entry.strong_count() != 0);
        self.insertion_splits.push(Rc::downgrade(&watch));
        Ok(InsertionSplit {
            parent: self.lease_node(parent),
            prefix: self.lease_node(prefix),
            suffix: self.lease_node(suffix),
            watch,
            signals,
        })
    }

    /// Returns true when a refused insertion's pair was restored. An edited
    /// pair keeps its data, position and pending slot signals.
    pub fn finish_insertion_split(&mut self, split: InsertionSplit, refused: bool) -> bool {
        if !split.prefix.belongs_to(self) {
            return false;
        }
        let prefix = split.prefix.id();
        let suffix = split.suffix.id();
        let parent = split.parent.id();
        self.insertion_splits.retain(|entry| {
            !entry.ptr_eq(&Rc::downgrade(&split.watch)) && entry.strong_count() != 0
        });
        if refused
            && !split.watch.edited.get()
            && self.get_node(prefix).is_some_and(|node| {
                node.parent == Some(parent) && node.forward(1).is_some_and(|n| n.id == suffix)
            })
            && let Some(head) = self.character_data_dom(prefix)
            && let Some(tail) = self.character_data_dom(suffix)
        {
            let offset = head.utf16_units().count();
            let mut joined = head.clone();
            joined.push_dom(tail);
            let pending = self.mutation_log.take();
            self.slot_changes.during_split(Some(split.signals.clone()));
            {
                let mut edit = self.mutate();
                edit.unchanged_textarea_children = Some(parent);
                edit.set_character_data_raw(prefix, joined);
                edit.doc.notify_range_mutation(RangeMutation::Merge {
                    node: suffix,
                    into: prefix,
                    offset,
                });
                edit.remove_node(suffix);
            }
            self.slot_changes.during_split(None);
            self.slot_changes.finish_split(split.signals, true);
            self.mutation_log = pending;
            return true;
        }
        self.split_text_edited(prefix);
        self.slot_changes.finish_split(split.signals, false);
        false
    }
}
