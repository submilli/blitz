//! Flat-tree order of boundary points, and the inline formatting roots that
//! hold caret positions, in layout order.

use crate::layout::construct::{InlineElement, inline_element};
use crate::{BaseDocument, NodeId, ranges::Boundary};
use std::collections::HashMap;
use style::computed_values::float::T as Float;

/// A boundary point's place in flat-tree order: before node number `node`,
/// plus a UTF-16 offset inside a Text node. A text control's flow instead
/// keys entries by UTF-16 offset alone ([`Key::control`]); such keys are
/// only compared within that control.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Key {
    node: u32,
    offset: u32,
}

impl Key {
    /// A text control's UTF-16 offset, ordered within that control only.
    pub(super) fn control(offset: usize) -> Self {
        Self {
            node: 0,
            offset: u32::try_from(offset).unwrap_or(u32::MAX),
        }
    }
}

/// Flat-tree preorder numbers. A node's span covers its own number and every
/// number in its subtree, so the boundary after it is the end of its span.
pub(super) struct TreeOrder {
    spans: HashMap<NodeId, (u32, u32)>,
}

impl TreeOrder {
    pub(super) fn new(doc: &BaseDocument) -> Self {
        let mut spans: HashMap<NodeId, (u32, u32)> = HashMap::new();
        let mut next = 0u32;
        let mut pending = vec![(doc.root_node().id, false)];
        while let Some((id, done)) = pending.pop() {
            if done {
                if let Some(span) = spans.get_mut(&id) {
                    span.1 = next;
                }
                continue;
            }
            spans.insert(id, (next, next));
            next = next.saturating_add(1);
            pending.push((id, true));
            let node = &doc.nodes[id];
            let children = node
                .before()
                .into_iter()
                .chain(node.flat_children().iter().copied())
                .chain(node.after());
            let start = pending.len();
            pending.extend(children.map(|child| (child, false)));
            pending[start..].reverse();
        }
        Self { spans }
    }

    /// Text boundaries keep their offset; any other boundary is the point
    /// before its child at `offset`, or the end of the parent.
    pub(super) fn key(&self, doc: &BaseDocument, point: Boundary) -> Option<Key> {
        let node = doc.get_node(point.node)?;
        if node.is_text_node() {
            let (start, _) = *self.spans.get(&point.node)?;
            return Some(Key {
                node: start,
                offset: u32::try_from(point.offset).unwrap_or(u32::MAX),
            });
        }
        let following = node.children.as_slice().get(point.offset..).unwrap_or(&[]);
        if let Some((start, _)) = following.iter().find_map(|child| self.spans.get(child)) {
            return Some(Key {
                node: *start,
                offset: 0,
            });
        }
        let owner = if self.spans.contains_key(&point.node) {
            point.node
        } else {
            doc.shadow_host_of(point.node)?
        };
        self.end(owner)
    }

    /// The boundary point before `id`.
    pub(super) fn before(&self, id: NodeId) -> Option<Key> {
        self.spans
            .get(&id)
            .map(|&(node, _)| Key { node, offset: 0 })
    }

    /// The boundary point after `id` and its flat-tree descendants.
    pub(super) fn end(&self, id: NodeId) -> Option<Key> {
        self.spans
            .get(&id)
            .map(|&(_, node)| Key { node, offset: 0 })
    }

    /// The boundary of a character inside Text node `id`.
    pub(super) fn text(&self, id: NodeId, offset: usize) -> Option<Key> {
        self.spans.get(&id).map(|&(node, _)| Key {
            node,
            offset: u32::try_from(offset).unwrap_or(u32::MAX),
        })
    }

    /// The keys a root's content can occupy. An anonymous block has no tree
    /// position of its own, so it spans its laid-out children.
    pub(super) fn root_span(&self, doc: &BaseDocument, root: NodeId) -> Option<(Key, Key)> {
        if let (Some(start), Some(end)) = (self.before(root), self.end(root)) {
            return Some((start, end));
        }
        let children = doc.nodes[root].flat_children();
        let start = children.iter().find_map(|&child| self.before(child))?;
        let end = children.iter().rev().find_map(|&child| self.end(child))?;
        Some((start, end))
    }
}

/// Laid-out inline formatting roots. A root's embedded boxes that do not
/// continue its flow (floats, positioned boxes, and inline-level boxes with
/// block content) hold roots of their own. Callers order the flows built
/// from them by tree position, as caret movement follows the DOM.
pub(super) fn inline_roots(doc: &BaseDocument) -> Vec<NodeId> {
    let mut roots = Vec::new();
    let mut pending = vec![(doc.root_node().id, false)];
    while let Some((id, embedded)) = pending.pop() {
        let node = &doc.nodes[id];
        if node.is_element() && node.display_style().is_none_or(|d| d.is_none()) {
            continue;
        }
        if embedded {
            if is_atom(node) {
                continue;
            }
            if let Some(inner) = flattened_root(doc, id).filter(|_| !is_out_of_flow(node)) {
                push_layout_children(doc, inner, true, &mut pending);
                continue;
            }
        }
        if is_laid_out_root(node) {
            roots.push(id);
            push_layout_children(doc, id, true, &mut pending);
            continue;
        }
        push_layout_children(doc, id, false, &mut pending);
    }
    roots
}

/// The inline root whose content continues the flow around an inline-level
/// box: the box itself, or a sole anonymous block inside it (the item of an
/// `inline-flex` box holding text).
pub(super) fn flattened_root(doc: &BaseDocument, id: NodeId) -> Option<NodeId> {
    let node = &doc.nodes[id];
    if is_laid_out_root(node) {
        return Some(id);
    }
    let children = node.layout_children.borrow();
    let &[child] = children.as_deref()? else {
        return None;
    };
    let child_node = doc.get_node(child)?;
    (child_node.is_anonymous() && is_laid_out_root(child_node)).then_some(child)
}

/// Floats and positioned boxes leave the flow of the root that embeds them.
pub(super) fn is_out_of_flow(node: &crate::Node) -> bool {
    node.primary_styles().is_some_and(|s| {
        s.clone_position().is_absolutely_positioned() || s.clone_float() != Float::None
    })
}

fn is_laid_out_root(node: &crate::Node) -> bool {
    node.flags.is_inline_root()
        && node
            .element_data()
            .is_some_and(|element| element.inline_layout_data.is_some())
}

/// Replaced elements and controls are single caret units; their insides
/// hold no document positions.
fn is_atom(node: &crate::Node) -> bool {
    node.element_data()
        .is_some_and(|e| inline_element(e) == InlineElement::Atom)
}

fn push_layout_children(
    doc: &BaseDocument,
    id: NodeId,
    embedded: bool,
    pending: &mut Vec<(NodeId, bool)>,
) {
    let node = &doc.nodes[id];
    let children = node.layout_children.borrow();
    let start = pending.len();
    let ids = children.as_deref().unwrap_or(node.flat_children());
    pending.extend(ids.iter().map(|&child| (child, embedded)));
    pending[start..].reverse();
}
