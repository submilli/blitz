//! Rebuild a subtree that lives in another arena.
//!
//! Node IDs, style locks, layout boxes and stylo data belong to one
//! [`BaseDocument`]. A node cannot move between arenas, so an embedder that
//! adopts or imports across documents recreates the subtree here and carries
//! over only its portable DOM state. The source arena is read, never changed;
//! the embedder removes and reclaims the source subtree itself.
//!
//! [Adoption](https://dom.spec.whatwg.org/#concept-node-adopt) keeps a node's
//! state ([`ImportMode::Transfer`]); [cloning](https://dom.spec.whatwg.org/#concept-node-clone)
//! copies what the clone steps copy ([`ImportMode::Copy`]).

use std::collections::HashMap;

use crate::dom_api::{DomError, node_type};
use crate::node_seed::Seed;
use crate::{BaseDocument, DocumentMutator, NodeId};

/// What crosses into this arena.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportMode {
    /// `cloneNode`/`importNode` semantics: a new identity, with descendants
    /// and template contents only when `deep`.
    Copy { deep: bool },
    /// Adoption: the shadow-including subtree with its template contents and
    /// the state that a moved node keeps.
    Transfer,
}

/// The detached root built in this arena, and which source node each new node
/// stands for. Source IDs are meaningful only in the source arena.
#[derive(Debug)]
pub struct ImportedSubtree {
    pairs: Vec<(NodeId, NodeId)>,
}

impl ImportedSubtree {
    pub fn root(&self) -> NodeId {
        self.pairs[0].1
    }

    /// `(source, imported)` pairs, the root first.
    pub fn pairs(&self) -> &[(NodeId, NodeId)] {
        &self.pairs
    }
}

impl DocumentMutator<'_> {
    /// Rebuild `root` from `source` as a detached subtree whose node document
    /// is `document`, a Document of this arena. Admission covers the whole
    /// subtree before any allocation, so a rejected import changes nothing.
    /// No script runs here and no mutation records or reactions are queued;
    /// the embedder adopts, inserts or discards the result.
    pub fn try_import_foreign(
        &mut self,
        source: &BaseDocument,
        root: NodeId,
        document: NodeId,
        mode: ImportMode,
    ) -> Result<ImportedSubtree, DomError> {
        if source.node_type(root) == node_type::DOCUMENT {
            return Err(DomError::NotSupported);
        }
        let count = imported_node_count(source, root, mode);
        self.doc
            .check_node_allocation(count)
            .map_err(|_| DomError::QuotaExceeded)?;
        // Building a detached tree is not an observable mutation.
        let log = self.doc.mutation_log.take();
        let imported = self.build(source, root, document, mode);
        self.doc.mutation_log = log;
        if mode == ImportMode::Transfer {
            self.link_transferred(source, &imported);
        }
        Ok(imported)
    }

    /// Drop a detached import that was never inserted, for example after a
    /// hierarchy check failed.
    pub fn discard_import(&mut self, imported: ImportedSubtree) {
        let root = imported.root();
        if self
            .doc
            .get_node(root)
            .is_some_and(|node| node.parent.is_none())
        {
            for &(_, id) in imported.pairs() {
                self.doc.form_associated_custom.remove(&id);
            }
            self.doc.drop_node_ignoring_parent(root);
        }
    }

    /// Plant the subtree in shadow-including preorder, so `pairs` lists each
    /// node before its shadow root and children (template contents follow).
    fn build(
        &mut self,
        source: &BaseDocument,
        root: NodeId,
        document: NodeId,
        mode: ImportMode,
    ) -> ImportedSubtree {
        let deep = !matches!(mode, ImportMode::Copy { deep: false });
        let transfer = mode == ImportMode::Transfer;
        let mut pairs = Vec::new();
        let mut pending = vec![(root, Place::Root(document))];
        while let Some((from, place)) = pending.pop() {
            let copy = match place {
                Place::Root(document) => {
                    self.plant(Seed::capture(source, from, transfer), document)
                }
                Place::Child(parent) => {
                    let document = self.doc.node_document(parent);
                    let copy = self.plant(Seed::capture(source, from, transfer), document);
                    self.append_children(parent, &[copy]);
                    copy
                }
                Place::Shadow(host) => self.plant_shadow_root(source, from, host),
                Place::Contents(template) => self.template_contents(template),
            };
            pairs.push((from, copy));
            // Push in reverse visiting order: shadow root, children, contents.
            // Only a shallow copy's root omits its descendants.
            let node = &source.nodes[from];
            let descend = deep || !matches!(place, Place::Root(_));
            if descend {
                if let Some(contents) = node.element_data().and_then(|e| e.template_contents) {
                    pending.push((contents, Place::Contents(copy)));
                }
                pending.extend(
                    node.children
                        .iter()
                        .rev()
                        .map(|&child| (child, Place::Child(copy))),
                );
            }
            if let Some(shadow) = imported_shadow_root(source, from, transfer) {
                pending.push((shadow, Place::Shadow(copy)));
            }
        }
        ImportedSubtree { pairs }
    }

    fn plant_shadow_root(&mut self, source: &BaseDocument, shadow: NodeId, host: NodeId) -> NodeId {
        let mut data = source.nodes[shadow]
            .shadow_root_data
            .as_deref()
            .expect("shadow roots carry their options")
            .clone();
        data.host = host;
        let copy = self.create_document_fragment();
        self.doc.nodes[copy].shadow_root_data = Some(Box::new(data));
        self.doc.nodes[host]
            .element_data_mut()
            .expect("shadow hosts are elements")
            .shadow_root = Some(copy);
        self.doc.shadow_hosts.insert(host);
        self.set_node_document(copy, self.doc.node_document(host));
        copy
    }

    /// Restore the references that name other transferred nodes.
    fn link_transferred(&mut self, source: &BaseDocument, imported: &ImportedSubtree) {
        let map: HashMap<NodeId, NodeId> = imported.pairs().iter().copied().collect();
        for &(from, to) in imported.pairs() {
            let node = &source.nodes[from];
            let manual_slot = node.manual_slot.and_then(|slot| map.get(&slot).copied());
            let manual_slottables = node
                .manual_slottables
                .iter()
                .filter_map(|id| map.get(id).copied())
                .collect();
            let anchor = node
                .element_data()
                .and_then(|element| element.custom_internals.as_ref())
                .and_then(|internals| internals.anchor)
                .and_then(|anchor| map.get(&anchor).copied());
            let target = &mut self.doc.nodes[to];
            target.manual_slot = manual_slot;
            target.manual_slottables = manual_slottables;
            if let Some(internals) = target
                .element_data_mut()
                .and_then(|element| element.custom_internals.as_mut())
            {
                internals.anchor = anchor;
            }
            // Assignments are derived; refresh them when the mutator flushes.
            // Imported roots are fresh, so none is listed yet.
            if target.shadow_root_data.is_some() {
                self.doc.dirty_slot_roots.push(to);
            }
        }
    }
}

/// Where a planted node goes.
#[derive(Clone, Copy)]
enum Place {
    /// The detached root, owned by this document.
    Root(NodeId),
    Child(NodeId),
    Shadow(NodeId),
    Contents(NodeId),
}

/// The shadow root of `host` that comes along: any for a transfer, a
/// clonable one for a copy. Admission and planting must agree on this.
fn imported_shadow_root(source: &BaseDocument, host: NodeId, transfer: bool) -> Option<NodeId> {
    let shadow = source.shadow_root_of(host)?;
    source.nodes[shadow]
        .shadow_root_data
        .as_ref()
        .is_some_and(|data| transfer || data.clonable)
        .then_some(shadow)
}

/// Count every node [`DocumentMutator::try_import_foreign`] will create.
fn imported_node_count(source: &BaseDocument, root: NodeId, mode: ImportMode) -> usize {
    let transfer = mode == ImportMode::Transfer;
    let mut pending = vec![(root, !matches!(mode, ImportMode::Copy { deep: false }))];
    let mut count = 0usize;
    while let Some((id, deep)) = pending.pop() {
        count = count.saturating_add(1);
        let node = &source.nodes[id];
        if let Some(shadow) = imported_shadow_root(source, id, transfer) {
            count = count.saturating_add(1);
            pending.extend(source.nodes[shadow].children.iter().map(|&id| (id, true)));
        }
        if deep {
            pending.extend(node.children.iter().map(|&id| (id, true)));
            if let Some(contents) = node.element_data().and_then(|e| e.template_contents) {
                pending.push((contents, true));
            }
        }
    }
    count
}

#[cfg(test)]
mod tests;
