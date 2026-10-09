//! Shadow DOM: shadow roots attached to host elements, and the flat tree.
//!
//! A shadow root is a fragment node whose [`Node::shadow_root_data`] names
//! its host; the host's [`ElementData::shadow_root`] points back. The shadow
//! tree is not among the host's children, so DOM traversal from the document
//! does not enter it.
//!
//! Style and layout work on the *flat tree*: a host's children there are its
//! shadow root's children, and a `<slot>`'s are the host children assigned
//! to it (by `slot` name), or its own children as fallback. Host children
//! assigned to no slot are not rendered. [`BaseDocument::compose_shadow_trees`]
//! records this in [`Node::composed_children`] and [`Node::flat_parent`]
//! before each style pass.
//!
//! [`ElementData::shadow_root`]: crate::node::ElementData::shadow_root

mod slots;
#[cfg(test)]
mod tests;

use markup5ever::local_name;

use style::invalidation::element::restyle_hints::RestyleHint;

use crate::layout::damage::ALL_DAMAGE;
use crate::node::{FlatParent, NodeData, ShadowRootData};
use crate::{BaseDocument, DocumentMutator, NodeId};

/// A shadow root's stylesheets (from `<style>` and `<link>` elements in its
/// tree) and the cascade data Stylo matches its elements against.
pub struct ShadowStyles {
    /// Owner lookup; cascade order is derived from the current DOM tree.
    pub sheets: std::collections::BTreeMap<NodeId, style::stylesheets::DocumentStyleSheet>,
    pub(crate) adopted: Vec<style::stylesheets::DocumentStyleSheet>,
    pub author_styles: style::author_styles::AuthorStyles<style::stylesheets::DocumentStyleSheet>,
    /// The sheets changed since `author_styles` was built.
    pub dirty: bool,
}

impl Default for ShadowStyles {
    fn default() -> Self {
        Self {
            sheets: Default::default(),
            adopted: Vec::new(),
            author_styles: style::author_styles::AuthorStyles::new(),
            dirty: false,
        }
    }
}

/// `attachShadow`'s options.
#[derive(Debug, Clone, Copy, Default)]
pub struct ShadowRootInit {
    pub open: bool,
    pub delegates_focus: bool,
    pub manual_slots: bool,
    pub clonable: bool,
    pub serializable: bool,
}

/// Why `attachShadow` refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachShadowError {
    /// The element cannot host a shadow root (`NotSupportedError`).
    NotSupported,
    /// It already has one (`NotSupportedError` too).
    AlreadyAttached,
    /// The document has no remaining node capacity (`QuotaExceededError`).
    NodeBudgetExceeded,
}

/// Elements that may host a shadow root, besides autonomous custom elements.
const VALID_HOSTS: &[&str] = &[
    "article",
    "aside",
    "blockquote",
    "body",
    "div",
    "footer",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "main",
    "nav",
    "p",
    "section",
    "span",
];

impl DocumentMutator<'_> {
    /// `Element.attachShadow(init)`.
    pub fn attach_shadow(
        &mut self,
        host: NodeId,
        init: ShadowRootInit,
    ) -> Result<NodeId, AttachShadowError> {
        let element = self.doc.nodes[host]
            .element_data()
            .ok_or(AttachShadowError::NotSupported)?;
        let local = &*element.name.local;
        let valid = element.name.ns == markup5ever::ns!(html)
            && (VALID_HOSTS.contains(&local)
                || crate::custom_elements::is_valid_custom_element_name(local));
        let disabled = &self.doc.document_metadata(host).disabled_shadow_definitions;
        // Autonomous definitions take precedence over any inert `is` attribute.
        let shadow_disabled = disabled.get(local).is_some_and(|name| name == local)
            || element.custom_element_is.as_deref().is_some_and(|name| {
                disabled
                    .get(name)
                    .is_some_and(|local_name| local_name == local)
            });
        if !valid || shadow_disabled {
            return Err(AttachShadowError::NotSupported);
        }
        if let Some(root) = element.shadow_root {
            let data = self.doc.nodes[root]
                .shadow_root_data
                .as_deref()
                .expect("host points to a shadow root");
            if !data.declarative || data.open != init.open {
                return Err(AttachShadowError::AlreadyAttached);
            }
            self.replace_all(root, None);
            self.doc.nodes[root]
                .shadow_root_data
                .as_deref_mut()
                .expect("host points to a shadow root")
                .declarative = false;
            return Ok(root);
        }
        let root = self
            .try_create_document_fragment()
            .map_err(|_| AttachShadowError::NodeBudgetExceeded)?;
        self.set_node_document(root, self.doc.node_document(host));
        self.doc.nodes[root].shadow_root_data = Some(Box::new(ShadowRootData {
            host,
            open: init.open,
            delegates_focus: init.delegates_focus,
            manual_slots: init.manual_slots,
            clonable: init.clonable,
            serializable: init.serializable,
            declarative: false,
        }));
        self.doc.nodes[host]
            .element_data_mut()
            .expect("checked")
            .shadow_root = Some(root);
        self.doc.shadow_hosts.insert(host);
        // The host now renders its (empty) shadow tree instead of its
        // children.
        let node = &mut self.doc.nodes[host];
        node.insert_damage(ALL_DAMAGE);
        node.mark_ancestors_dirty();
        if node.flags.is_in_document() {
            self.connect_subtree(root);
        }
        Ok(root)
    }
}

impl BaseDocument {
    /// The shadow root attached to `host`.
    pub fn shadow_root_of(&self, host: NodeId) -> Option<NodeId> {
        self.nodes.get(host)?.element_data()?.shadow_root
    }

    /// The host of the shadow root `root`.
    pub fn shadow_host_of(&self, root: NodeId) -> Option<NodeId> {
        self.nodes
            .get(root)?
            .shadow_root_data
            .as_ref()
            .map(|d| d.host)
    }

    /// The root of `node`'s tree, if it is a shadow root.
    pub fn containing_shadow_root(&self, node: NodeId) -> Option<NodeId> {
        let root = self.tree_root(node);
        self.nodes
            .get(root)
            .is_some_and(crate::Node::is_shadow_root)
            .then_some(root)
    }

    /// The slot name a host child would be assigned by: an element's `slot`
    /// attribute, or "" (the default slot) for text and unnamed elements.
    fn slot_name(&self, node: NodeId) -> String {
        match &self.nodes[node].data {
            NodeData::Element(element) => {
                element.attr(local_name!("slot")).unwrap_or("").to_string()
            }
            _ => String::new(),
        }
    }

    /// The `<slot>` elements of a shadow tree, in tree order (not entering
    /// nested shadow trees).
    fn slots_in(&self, root: NodeId) -> Vec<NodeId> {
        let mut slots = Vec::new();
        let mut stack: Vec<NodeId> = self.nodes[root].children.iter().rev().copied().collect();
        while let Some(id) = stack.pop() {
            let node = &self.nodes[id];
            if node.element_data().is_some_and(|e| {
                e.name.ns == markup5ever::ns!(html) && e.name.local == local_name!("slot")
            }) {
                slots.push(id);
            }
            stack.extend(node.children.iter().rev().copied());
        }
        slots
    }

    /// Host children that take part in slotting: elements and text.
    fn slottables(&self, host: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        self.nodes[host]
            .children
            .iter()
            .copied()
            .filter(|&c| matches!(self.nodes[c].data, NodeData::Element(_) | NodeData::Text(_)))
    }

    /// `assignedSlot`: the slot `node` (a host's child) is assigned to.
    pub fn assigned_slot(&self, node: NodeId) -> Option<NodeId> {
        let slottable = self.nodes.get(node)?;
        if !matches!(slottable.data, NodeData::Element(_) | NodeData::Text(_)) {
            return None;
        }
        let host = slottable.parent?;
        let root = self.shadow_root_of(host)?;
        if self.nodes[root].shadow_root_data.as_ref()?.manual_slots {
            let slot = slottable.manual_slot?;
            self.nodes.get(slot)?;
            return (self.containing_shadow_root(slot) == Some(root)).then_some(slot);
        }
        let name = self.slot_name(node);
        self.slots_in(root)
            .into_iter()
            .find(|&slot| self.slot_own_name(slot) == name)
    }

    fn slot_own_name(&self, slot: NodeId) -> String {
        self.nodes[slot]
            .element_data()
            .and_then(|e| e.attr(local_name!("name")))
            .unwrap_or("")
            .to_string()
    }

    /// `slot.assignedNodes()`: the host children assigned to `slot`.
    pub fn assigned_nodes(&self, slot: NodeId) -> Vec<NodeId> {
        let Some(root) = self.containing_shadow_root(slot) else {
            return Vec::new();
        };
        let host = self.shadow_host_of(root).expect("shadow root");
        if self.nodes[root]
            .shadow_root_data
            .as_ref()
            .is_some_and(|data| data.manual_slots)
        {
            return self.nodes[slot]
                .manual_slottables
                .iter()
                .copied()
                .filter(|&id| {
                    self.nodes.get(id).is_some_and(|node| {
                        node.parent == Some(host) && node.manual_slot == Some(slot)
                    })
                })
                .collect();
        }
        let name = self.slot_own_name(slot);
        // Only the first slot of a name receives nodes.
        if self
            .slots_in(root)
            .into_iter()
            .find(|&s| self.slot_own_name(s) == name)
            != Some(slot)
        {
            return Vec::new();
        }
        self.slottables(host)
            .filter(|&c| self.slot_name(c) == name)
            .collect()
    }

    /// Record the flat tree for every shadow host (see the module docs),
    /// damaging what moved.
    pub(crate) fn compose_shadow_trees(&mut self) {
        let hosts: Vec<NodeId> = self.shadow_hosts.iter().copied().collect();
        for host in hosts {
            let Some(root) = self.shadow_root_of(host) else {
                self.shadow_hosts.remove(&host);
                continue;
            };
            self.set_composed_children(host, Some(self.nodes[root].children.to_vec()));
            let root_children = self.nodes[root].children.clone();
            for child in root_children {
                self.set_flat_parent(child, FlatParent::Node(host));
            }
            let slots = self.slots_in(root);
            let assigned: Vec<_> = slots
                .iter()
                .map(|&slot| (slot, self.assigned_nodes(slot)))
                .collect();
            for child in self.slottables(host).collect::<Vec<_>>() {
                let parent = self
                    .assigned_slot(child)
                    .map_or(FlatParent::None, FlatParent::Node);
                self.set_flat_parent(child, parent);
            }
            // Comments and the like under a host are never rendered.
            let host_children = self.nodes[host].children.clone();
            for child in host_children {
                if !matches!(
                    self.nodes[child].data,
                    NodeData::Element(_) | NodeData::Text(_)
                ) {
                    self.set_flat_parent(child, FlatParent::None);
                }
            }
            for (slot, nodes) in assigned {
                // A slot with nothing assigned shows its own children.
                self.set_composed_children(slot, (!nodes.is_empty()).then_some(nodes));
            }
        }
    }

    /// Add (or replace) the stylesheet of `node`, which is in the shadow tree
    /// rooted at `root`.
    pub(crate) fn add_shadow_stylesheet(
        &mut self,
        root: NodeId,
        node: NodeId,
        sheet: style::stylesheets::DocumentStyleSheet,
    ) {
        let styles = self.nodes[root]
            .shadow_styles
            .get_or_insert_with(Default::default);
        styles.sheets.insert(node, sheet);
        styles.dirty = true;
    }

    /// Forget `node`'s stylesheet in whichever shadow tree held it.
    pub(crate) fn remove_shadow_stylesheet(&mut self, node: NodeId) {
        let roots: Vec<NodeId> = self
            .shadow_hosts
            .iter()
            .filter_map(|&h| self.shadow_root_of(h))
            .collect();
        for root in roots {
            if let Some(styles) = self.nodes[root].shadow_styles.as_mut()
                && styles.sheets.remove(&node).is_some()
            {
                styles.dirty = true;
            }
        }
    }

    /// Rebuild the cascade data of shadow roots whose sheets changed, and
    /// restyle their hosts' subtrees (`:host` rules style the host).
    pub(crate) fn flush_shadow_styles(&mut self) {
        let hosts: Vec<NodeId> = self.shadow_hosts.iter().copied().collect();
        for host in hosts {
            let Some(root) = self.shadow_root_of(host) else {
                continue;
            };
            let Some(mut styles) = self.nodes[root].shadow_styles.take() else {
                continue;
            };
            if styles.dirty {
                let guard = self.guard.clone();
                let guard = guard.read();
                let mut author_styles = style::author_styles::AuthorStyles::new();
                // Stylo must recognize the prior cascade when CSSOM mutates a
                // sheet in place; starting from empty would reuse stale cache data.
                author_styles.data = styles.author_styles.data.clone();
                let custom_media = style::stylesheets::CustomMediaMap::default();
                let owners = self.stylesheet_owners_in_tree_order(styles.sheets.keys().copied());
                let sheets = owners.iter().filter_map(|owner| styles.sheets.get(owner));
                for sheet in sheets.chain(styles.adopted.iter()) {
                    author_styles.stylesheets.append_stylesheet(
                        None,
                        &custom_media,
                        sheet.clone(),
                        &guard,
                    );
                }
                // A disabled/removed last sheet adds no effective rules, but
                // must still replace the previous nonempty cascade.
                author_styles.stylesheets.force_dirty();
                author_styles.flush(&mut self.stylist, &guard);
                styles.author_styles = author_styles;
                styles.dirty = false;
                let host_node = &mut self.nodes[host];
                host_node.insert_damage(ALL_DAMAGE);
                host_node.set_restyle_hint(RestyleHint::restyle_subtree());
                host_node.mark_ancestors_dirty();
            }
            self.nodes[root].shadow_styles = Some(styles);
        }
    }

    fn set_composed_children(&mut self, id: NodeId, children: Option<Vec<NodeId>>) {
        let node = &mut self.nodes[id];
        let new = children.map(|c| c.into_iter().collect());
        if node.composed_children != new {
            node.composed_children = new;
            node.insert_damage(ALL_DAMAGE);
            node.set_restyle_hint(RestyleHint::restyle_subtree());
        }
    }

    fn set_flat_parent(&mut self, id: NodeId, parent: FlatParent) {
        let node = &self.nodes[id];
        if node.flat_parent.get() != parent {
            node.flat_parent.set(parent);
            // Its inherited style comes from its new flat parent.
            let node = &mut self.nodes[id];
            node.insert_damage(ALL_DAMAGE);
            node.set_restyle_hint(RestyleHint::restyle_subtree());
        }
    }
}

/// The requested flattened assignment exceeded the embedder's work/output limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlotFlattenLimit;

/// Flatten named slot assignments iteratively, bounded before allocating output.
/// DOM: find flattened slottables. Missing/non-slot receivers yield no nodes.
impl BaseDocument {
    pub fn flattened_assigned_nodes(
        &self,
        slot: NodeId,
        limit: usize,
    ) -> Result<Vec<NodeId>, SlotFlattenLimit> {
        let mut output = Vec::new();
        if self.containing_shadow_root(slot).is_none() {
            return Ok(output);
        }
        let mut pending = vec![(slot, true)];
        let mut visited = 0usize;
        while let Some((id, expand)) = pending.pop() {
            visited += 1;
            if visited > limit {
                return Err(SlotFlattenLimit);
            }
            let Some(node) = self.get_node(id) else {
                continue;
            };
            if !expand {
                output.push(id);
                continue;
            }
            let assigned = self.assigned_nodes(id);
            let children = if assigned.is_empty() {
                node.children.as_slice()
            } else {
                assigned.as_slice()
            };
            if children.len() > limit.saturating_sub(pending.len() + output.len()) {
                return Err(SlotFlattenLimit);
            }
            for &child in children.iter().rev() {
                let Some(child_node) = self.get_node(child) else {
                    continue;
                };
                let nested = child_node.element_data().is_some_and(|el| {
                    el.name.ns == markup5ever::ns!(html) && &*el.name.local == "slot"
                }) && self.containing_shadow_root(child).is_some();
                if child_node.is_element()
                    || matches!(child_node.data, crate::node::NodeData::Text(_))
                {
                    pending.push((child, nested));
                }
            }
        }
        Ok(output)
    }
}
