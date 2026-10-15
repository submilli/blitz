/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Nearest box-generating ancestors during one top-down style traversal.

use crate::dom::{OpaqueNode, SendElement, TElement, TNode};
use rustc_hash::FxHashMap;
use smallvec::SmallVec;

/// Ancestor styles have already been computed when a child is cascaded.
/// Keep this cache within one traversal's thread-local context: neither DOM
/// ancestry nor those styles may change while its entries are reused.
pub(crate) struct LayoutParentCache<E: TElement> {
    parents: FxHashMap<OpaqueNode, SendElement<E>>,
}

impl<E: TElement> LayoutParentCache<E> {
    pub(crate) fn new() -> Self {
        Self {
            parents: FxHashMap::default(),
        }
    }

    /// Return a cached replacement only for a display:contents inheritance
    /// parent. Ordinary inheritance and absent parent styles retain their path.
    pub(crate) fn resolve(&mut self, element: E) -> Option<E> {
        let mut current = element.inheritance_parent()?;
        if !current
            .borrow_data()?
            .styles
            .primary()
            .is_display_contents()
        {
            return None;
        }
        let mut path = SmallVec::<[E; 8]>::new();
        let parent = loop {
            if let Some(parent) = self.parents.get(&current.as_node().opaque()) {
                break **parent;
            }
            if !current
                .borrow_data()?
                .styles
                .primary()
                .is_display_contents()
            {
                break current;
            }
            path.push(current);
            match current.traversal_parent() {
                Some(parent) => current = parent,
                None => break current,
            }
        };
        for element in path {
            // SAFETY: the traversal keeps all elements alive, and entries are
            // only read on their owning worker before sequential tasks run.
            self.parents.insert(element.as_node().opaque(), unsafe {
                SendElement::new(parent)
            });
        }
        Some(parent)
    }
}
