//! The flat tree as Blink's `FlatTreeTraversal` walks it, and the element
//! kinds editing treats specially. Pseudo-elements are not part of it.

use crate::node::NodeData;
use crate::{BaseDocument, Node, NodeId};
use markup5ever::local_name;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// Flat-tree navigation over a document, remembering child indices. Blink
/// edits in the flat tree, and in the DOM tree (`EditingStrategy`) for a
/// selection's final canonicalization; [`Tree::dom`] walks that one.
pub(super) struct Tree<'a> {
    doc: &'a BaseDocument,
    /// Walk DOM links instead of the flat tree's.
    dom: bool,
    /// Each child's index among its parent's children in this tree, filled
    /// a parent at a time so sibling walks stay linear.
    indices: RefCell<HashMap<NodeId, usize>>,
    /// Text nodes' UTF-16 code units, transcoded once per call.
    units: RefCell<HashMap<NodeId, Rc<[u16]>>>,
}

impl<'a> Tree<'a> {
    pub(super) fn new(doc: &'a BaseDocument) -> Self {
        Self::with_links(doc, false)
    }

    /// The DOM tree: a host's light children, not its shadow content.
    pub(super) fn dom(doc: &'a BaseDocument) -> Self {
        Self::with_links(doc, true)
    }

    fn with_links(doc: &'a BaseDocument, dom: bool) -> Self {
        Self {
            doc,
            dom,
            indices: RefCell::default(),
            units: RefCell::default(),
        }
    }

    pub(super) fn doc(&self) -> &'a BaseDocument {
        self.doc
    }

    pub(super) fn node(&self, id: NodeId) -> &'a Node {
        &self.doc.nodes[id]
    }

    pub(super) fn parent(&self, id: NodeId) -> Option<NodeId> {
        if self.dom {
            return self.dom_parent(id);
        }
        self.node(id).flat_parent_id()
    }

    pub(super) fn children(&self, id: NodeId) -> &'a [NodeId] {
        if self.dom {
            return &self.node(id).children;
        }
        self.node(id).flat_children()
    }

    /// Children in the layout (flat) tree, whichever tree this walks:
    /// rendering questions are about boxes.
    pub(super) fn layout_children(&self, id: NodeId) -> &'a [NodeId] {
        self.node(id).flat_children()
    }

    pub(super) fn is_dom(&self) -> bool {
        self.dom
    }

    /// The parent in the DOM, not the flat tree: `None` at a shadow root.
    pub(super) fn dom_parent(&self, id: NodeId) -> Option<NodeId> {
        self.node(id).parent
    }

    /// The root of a node's tree scope, its document or shadow root.
    pub(super) fn scope_root(&self, id: NodeId) -> NodeId {
        let mut current = id;
        while let Some(parent) = self.dom_parent(current) {
            current = parent;
        }
        current
    }

    /// Whether `ancestor` is `id` or one of its DOM ancestors.
    pub(super) fn is_dom_inclusive_descendant(&self, id: NodeId, ancestor: NodeId) -> bool {
        let mut node = Some(id);
        while let Some(current) = node {
            if current == ancestor {
                return true;
            }
            node = self.dom_parent(current);
        }
        false
    }

    /// Children in the DOM, not the flat tree: a host's light children.
    pub(super) fn dom_children(&self, id: NodeId) -> &'a [NodeId] {
        &self.node(id).children
    }

    /// A node's index among its DOM parent's children, with that parent.
    pub(super) fn dom_index(&self, id: NodeId) -> Option<(NodeId, usize)> {
        let parent = self.dom_parent(id)?;
        if self.parent(id) == Some(parent) {
            return Some((parent, self.index(id)));
        }
        let index = self.dom_children(parent).iter().position(|&c| c == id)?;
        Some((parent, index))
    }

    /// The host of a shadow root; `None` for other nodes.
    pub(super) fn shadow_host(&self, id: NodeId) -> Option<NodeId> {
        self.node(id)
            .shadow_root_data
            .as_ref()
            .map(|data| data.host)
    }

    pub(super) fn has_children(&self, id: NodeId) -> bool {
        !self.children(id).is_empty()
    }

    pub(super) fn count_children(&self, id: NodeId) -> usize {
        self.children(id).len()
    }

    pub(super) fn child_at(&self, id: NodeId, index: usize) -> Option<NodeId> {
        self.children(id).get(index).copied()
    }

    pub(super) fn first_child(&self, id: NodeId) -> Option<NodeId> {
        self.children(id).first().copied()
    }

    pub(super) fn last_child(&self, id: NodeId) -> Option<NodeId> {
        self.children(id).last().copied()
    }

    /// A node's index among its parent's children in this tree; 0 without
    /// a parent.
    pub(super) fn index(&self, id: NodeId) -> usize {
        if let Some(&index) = self.indices.borrow().get(&id) {
            return index;
        }
        let Some(parent) = self.parent(id) else {
            return 0;
        };
        let mut indices = self.indices.borrow_mut();
        for (index, &child) in self.children(parent).iter().enumerate() {
            indices.insert(child, index);
        }
        indices.get(&id).copied().unwrap_or(0)
    }

    pub(super) fn next_sibling(&self, id: NodeId) -> Option<NodeId> {
        let parent = self.parent(id)?;
        self.child_at(parent, self.index(id) + 1)
    }

    pub(super) fn previous_sibling(&self, id: NodeId) -> Option<NodeId> {
        let parent = self.parent(id)?;
        let index = self.index(id).checked_sub(1)?;
        self.child_at(parent, index)
    }

    /// The last node in `id`'s subtree in preorder, or `id` itself.
    pub(super) fn last_within_or_self(&self, id: NodeId) -> NodeId {
        let mut node = id;
        while let Some(last) = self.last_child(node) {
            node = last;
        }
        node
    }

    /// The node after `id` in pre-order, passing its children when
    /// `skip_children`.
    pub(super) fn next_in_preorder(&self, id: NodeId, skip_children: bool) -> Option<NodeId> {
        if !skip_children {
            if let Some(first) = self.first_child(id) {
                return Some(first);
            }
        }
        let mut current = id;
        loop {
            if let Some(next) = self.next_sibling(current) {
                return Some(next);
            }
            current = self.parent(current)?;
        }
    }

    /// Whether `id` is `ancestor` or one of its descendants in this tree.
    pub(super) fn is_inclusive_descendant(&self, id: NodeId, ancestor: NodeId) -> bool {
        let mut node = Some(id);
        while let Some(current) = node {
            if current == ancestor {
                return true;
            }
            node = self.parent(current);
        }
        false
    }

    pub(super) fn is_text(&self, id: NodeId) -> bool {
        self.node(id).is_text_node()
    }

    pub(super) fn is_element(&self, id: NodeId) -> bool {
        self.node(id).is_element()
    }

    /// The UTF-16 length of a Text node's data; 0 for other nodes.
    pub(super) fn text_length(&self, id: NodeId) -> usize {
        self.text_units(id).len()
    }

    /// A Text node's data as UTF-16 code units; empty for other nodes.
    pub(super) fn text_units(&self, id: NodeId) -> Rc<[u16]> {
        if let Some(found) = self.units.borrow().get(&id) {
            return found.clone();
        }
        let units: Rc<[u16]> = self
            .node(id)
            .text_data()
            .map_or_else(|| Rc::from([]), |text| Rc::from(&*text.content.to_utf16()));
        self.units.borrow_mut().insert(id, units.clone());
        units
    }

    pub(super) fn is_character_data(&self, id: NodeId) -> bool {
        matches!(
            self.node(id).data,
            NodeData::Text(_) | NodeData::Comment { .. } | NodeData::ProcessingInstruction { .. }
        )
    }

    /// Blink's `LastOffsetForEditing`: a character node's length, a
    /// container's child count, and 1 for an empty element whose content
    /// editing ignores.
    pub(super) fn last_offset_for_editing(&self, id: NodeId) -> usize {
        if self.is_character_data(id) {
            return self.text_length(id);
        }
        if self.has_children(id) {
            return self.count_children(id);
        }
        usize::from(self.editing_ignores_content(id))
    }

    /// Whether editing treats an element as opaque: it cannot hold a range
    /// boundary (`Node::CanContainRangeEndPoint`), as replaced elements,
    /// breaks, form controls and frames.
    pub(super) fn editing_ignores_content(&self, id: NodeId) -> bool {
        let Some(element) = self.node(id).element_data() else {
            return false;
        };
        match element.name.local {
            local_name!("img")
            | local_name!("br")
            | local_name!("iframe")
            | local_name!("frame")
            | local_name!("embed")
            | local_name!("object")
            | local_name!("input")
            | local_name!("select")
            | local_name!("textarea")
            | local_name!("progress")
            | local_name!("meter") => true,
            local_name!("hr") => !self.has_children(id),
            _ => false,
        }
    }

    /// An `<option>` or `<optgroup>` of a select: a list box row. Chrome
    /// reports the point before one in its parent, as for an opaque node.
    pub(super) fn is_select_row_element(&self, id: NodeId) -> bool {
        let is_row = self.node(id).element_data().is_some_and(|element| {
            matches!(
                element.name.local,
                local_name!("option") | local_name!("optgroup")
            )
        });
        is_row && self.doc().option_select(id).is_some()
    }

    /// Form controls contain their own selection (`IsUserSelectContain`).
    pub(super) fn is_user_select_contain(&self, id: NodeId) -> bool {
        self.node(id).element_data().is_some_and(|element| {
            matches!(
                element.name.local,
                local_name!("input") | local_name!("textarea") | local_name!("select")
            )
        })
    }

    /// `IsAtomicNodeInFlatTree`: a leaf of the flat tree, for editing.
    pub(super) fn is_atomic(&self, id: NodeId) -> bool {
        !self.has_children(id) || self.editing_ignores_content(id)
    }

    pub(super) fn is_br(&self, id: NodeId) -> bool {
        self.has_local_name(id, &local_name!("br"))
    }

    pub(super) fn is_table(&self, id: NodeId) -> bool {
        self.has_local_name(id, &local_name!("table"))
    }

    pub(super) fn is_body(&self, id: NodeId) -> bool {
        self.has_local_name(id, &local_name!("body"))
    }

    pub(super) fn is_document_element(&self, id: NodeId) -> bool {
        self.parent(id)
            .is_some_and(|parent| matches!(self.node(parent).data, NodeData::Document(_)))
            && self.is_element(id)
    }

    pub(super) fn is_document(&self, id: NodeId) -> bool {
        matches!(self.node(id).data, NodeData::Document(_))
    }

    /// The document element, the root of editing's start and end positions.
    pub(super) fn document_element(&self) -> Option<NodeId> {
        let root = self.doc.root_node().id;
        self.children(root)
            .iter()
            .copied()
            .find(|&child| self.is_element(child))
    }

    fn has_local_name(&self, id: NodeId, name: &markup5ever::LocalName) -> bool {
        self.node(id)
            .element_data()
            .is_some_and(|element| element.name.local == *name)
    }
}
