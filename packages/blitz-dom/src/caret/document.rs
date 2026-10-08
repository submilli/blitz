//! Caret movement over a document's laid-out inline content.

use super::flow::Flow;
use super::navigate::{self, Caret, Carets, Flows};
use super::order::{self, Key, TreeOrder};
use super::{Modified, ModifyRequest, build};
use crate::{BaseDocument, NodeId, ranges::Boundary};
use std::cell::RefCell;
use std::collections::HashMap;

impl BaseDocument {
    /// Move or extend a document selection. Returns `None` when an endpoint
    /// has no caret position, for example in a document without rendered
    /// text; the selection should then stay as it is.
    pub fn modify_selection(
        &self,
        anchor: Boundary,
        focus: Boundary,
        request: ModifyRequest,
    ) -> Option<Modified> {
        let flows = DocumentFlows::new(self);
        let mut focus_caret = flows.locate(focus)?;
        focus_caret.upstream = request.upstream;
        let carets = Carets {
            anchor: flows.locate(anchor)?,
            focus: focus_caret,
        };
        let (moved, line_x) = navigate::modify(&flows, carets, request)?;
        Some(Modified {
            anchor: flows.point(moved.anchor)?,
            focus: flows.point(moved.focus)?,
            line_x,
            upstream: moved.focus.upstream,
        })
    }
}

/// The document's flows in tree order, which caret movement follows even
/// where layout reorders boxes (flex `order`, floats).
struct DocumentFlows<'a> {
    doc: &'a BaseDocument,
    order: TreeOrder,
    flows: Vec<Flow>,
    /// Tree roots found so far, so clamping walks each ancestor chain once.
    roots: RefCell<HashMap<NodeId, NodeId>>,
}

impl<'a> DocumentFlows<'a> {
    fn new(doc: &'a BaseDocument) -> Self {
        let order = TreeOrder::new(doc);
        let mut flows: Vec<Flow> = order::inline_roots(doc)
            .into_iter()
            .flat_map(|root| build::document_flows(doc, &order, root))
            .filter(|flow| flow.first_key().is_some())
            .collect();
        flows.sort_by_key(Flow::first_key);
        Self {
            doc,
            order,
            flows,
            roots: RefCell::default(),
        }
    }

    /// The caret stop a boundary point renders at. Inside an inline root
    /// it is the point's own stop or the last stop before it (Chrome's
    /// upstream preference, which keeps the end of a paragraph in it); a
    /// point between blocks takes the next stop.
    fn locate(&self, point: Boundary) -> Option<Caret> {
        let key = self.order.key(self.doc, point)?;
        let Some(owner) = self.owning_root(point) else {
            return self
                .exact(key, None)
                .or_else(|| self.first_after(key))
                .or_else(|| self.document_end());
        };
        self.exact(key, Some(owner))
            .or_else(|| self.last_owned_before(key, owner))
            .or_else(|| self.first_after(key))
            .or_else(|| self.document_end())
    }

    /// The stop of a point that renders, in a flow of the owning root.
    /// A point that represents a stop wins over one that only renders
    /// there (the position after a `<br>` that a following float also
    /// starts at), so returned points locate back to the same stop.
    fn exact(&self, key: Key, owner: Option<(Key, Key)>) -> Option<Caret> {
        let owned = |flow: &Flow| owner.is_none_or(|owner| self.within(flow, owner));
        self.find(|flow| flow.exact(key, true).filter(|_| owned(flow)))
            .or_else(|| self.find(|flow| flow.exact(key, false).filter(|_| owned(flow))))
    }

    /// The last stop before the point among the owning root's flows.
    fn last_owned_before(&self, key: Key, owner: (Key, Key)) -> Option<Caret> {
        self.flows
            .iter()
            .enumerate()
            .rev()
            .find_map(|(flow, found)| {
                if !self.within(found, owner) {
                    return None;
                }
                Some(Caret::at(flow, found.before(key)?))
            })
    }

    /// Whether a flow's root lies inside `owner`'s span.
    fn within(&self, flow: &Flow, (start, end): (Key, Key)) -> bool {
        self.span(flow).is_some_and(|(a, b)| start <= a && b <= end)
    }

    fn first_after(&self, key: Key) -> Option<Caret> {
        self.find(|flow| flow.after(key))
    }

    fn document_end(&self) -> Option<Caret> {
        self.flows
            .iter()
            .enumerate()
            .rev()
            .find_map(|(flow, found)| Some(Caret::at(flow, found.last_stop()?)))
    }

    fn find(&self, stop: impl Fn(&Flow) -> Option<usize>) -> Option<Caret> {
        self.flows
            .iter()
            .enumerate()
            .find_map(|(flow, found)| Some(Caret::at(flow, stop(found)?)))
    }

    /// The span of the innermost inline root containing the point's node.
    fn owning_root(&self, point: Boundary) -> Option<(Key, Key)> {
        let start = self.order.before(point.node)?;
        let end = self.order.end(point.node)?;
        self.flows
            .iter()
            .filter_map(|flow| self.span(flow))
            .filter(|&(a, b)| a <= start && end <= b)
            .max_by_key(|&(a, _)| a)
    }

    fn span(&self, flow: &Flow) -> Option<(Key, Key)> {
        self.order.root_span(self.doc, flow.root())
    }

    /// The root of a node's tree, memoized along the ancestor chain.
    fn tree_root(&self, node: NodeId) -> NodeId {
        let mut roots = self.roots.borrow_mut();
        let mut path = Vec::new();
        let mut current = node;
        let root = loop {
            if let Some(&root) = roots.get(&current) {
                break root;
            }
            path.push(current);
            match self.doc.get_node(current).and_then(|n| n.parent) {
                Some(parent) => current = parent,
                None => break current,
            }
        };
        for id in path {
            roots.insert(id, root);
        }
        root
    }

    fn point(&self, caret: Caret) -> Option<Boundary> {
        self.flows.get(caret.flow)?.point(caret.index)
    }
}

impl Flows for DocumentFlows<'_> {
    fn count(&self) -> usize {
        self.flows.len()
    }

    fn flow(&self, index: usize) -> Option<&Flow> {
        self.flows.get(index)
    }

    fn tree(&self, caret: Caret) -> Option<NodeId> {
        Some(self.tree_root(self.point(caret)?.node))
    }
}
