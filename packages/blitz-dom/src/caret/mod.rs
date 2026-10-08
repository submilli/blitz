//! Caret navigation for `Selection.modify()`.
//! <https://w3c.github.io/selection-api/#dom-selection-modify>
//!
//! The specification leaves movement to the user agent; these rules follow
//! Chrome on macOS. Positions are the caret stops of laid-out inline content
//! (see [`flow`]), so callers must flush style and layout first. Each call
//! rebuilds its view of the document, in time linear in the document's nodes
//! and laid-out text, with no recursion and no retained state.

mod align;
mod build;
mod control;
mod flow;
mod geometry;
mod navigate;
mod order;
mod segments;

pub use control::ControlModified;

use crate::{BaseDocument, ranges::Boundary};
use flow::Flow;
use navigate::{Caret, Carets, Flows};
use order::TreeOrder;

/// `modify()`'s `alter` argument: move the caret, or extend the selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alter {
    Move,
    Extend,
}

/// `modify()`'s `direction` argument. `Left` and `Right` follow the
/// paragraph's base direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Forward,
    Backward,
    Left,
    Right,
}

/// `modify()`'s `granularity` argument.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Granularity {
    Character,
    Word,
    Sentence,
    Line,
    Paragraph,
    LineBoundary,
    SentenceBoundary,
    ParagraphBoundary,
    DocumentBoundary,
}

#[derive(Clone, Copy, Debug)]
pub struct ModifyRequest {
    pub alter: Alter,
    pub direction: Direction,
    pub granularity: Granularity,
    /// The horizontal position kept by consecutive line or paragraph
    /// movements, from the previous result. Any other selection change
    /// discards it.
    pub line_x: Option<f32>,
}

/// A selection after movement.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Modified {
    /// The canonical DOM position of the anchor's caret stop.
    pub anchor: Boundary,
    /// The canonical DOM position of the focus's caret stop.
    pub focus: Boundary,
    /// The horizontal position to pass to the next line or paragraph step.
    pub line_x: Option<f32>,
}

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
        let carets = Carets {
            anchor: flows.locate(anchor)?,
            focus: flows.locate(focus)?,
        };
        let (moved, line_x) = navigate::modify(&flows, carets, request)?;
        Some(Modified {
            anchor: flows.point(moved.anchor)?,
            focus: flows.point(moved.focus)?,
            line_x,
        })
    }
}

/// The document's flows in tree order, which caret movement follows even
/// where layout reorders boxes (flex `order`, floats).
struct DocumentFlows<'a> {
    doc: &'a BaseDocument,
    order: TreeOrder,
    flows: Vec<Flow>,
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
        Self { doc, order, flows }
    }

    /// The caret stop a boundary point renders at: its own stop; else the
    /// first stop after it, unless the point lies in an inline root whose
    /// content all precedes it (the end of a paragraph), where it takes that
    /// root's last stop; else the document's last stop.
    fn locate(&self, point: Boundary) -> Option<Caret> {
        let key = self.order.key(self.doc, point)?;
        let exact = self.flows.iter().enumerate().find_map(|(flow, found)| {
            Some(Caret {
                flow,
                index: found.exact(key)?,
            })
        });
        if exact.is_some() {
            return exact;
        }
        let after = self.flows.iter().enumerate().find_map(|(flow, found)| {
            Some(Caret {
                flow,
                index: found.after(key)?,
            })
        });
        let owner = self.owning_root(point);
        let owned = |caret: &Caret| {
            owner.is_none_or(|(start, end)| {
                let root = self.flows[caret.flow].root();
                self.order
                    .root_span(self.doc, root)
                    .is_some_and(|(a, b)| start <= a && b <= end)
            })
        };
        if let Some(caret) = after.filter(owned) {
            return Some(caret);
        }
        let before = self
            .flows
            .iter()
            .enumerate()
            .rev()
            .find_map(|(flow, found)| {
                let span = self.order.root_span(self.doc, found.root())?;
                let in_owner = owner.is_some_and(|owner| owner == span);
                let precedes = found.last_key().is_some_and(|last| last < key);
                Some(Caret {
                    flow,
                    index: found.last_stop().filter(|_| in_owner && precedes)?,
                })
            });
        before.or(after).or_else(|| self.document_end())
    }

    /// The span of the innermost inline root whose content contains the
    /// point's container.
    fn owning_root(&self, point: Boundary) -> Option<(order::Key, order::Key)> {
        let start = self.order.before(point.node)?;
        let end = self.order.end(point.node)?;
        self.flows
            .iter()
            .filter_map(|flow| self.order.root_span(self.doc, flow.root()))
            .filter(|&(a, b)| a <= start && end <= b)
            .max_by_key(|&(a, _)| a)
    }

    fn document_end(&self) -> Option<Caret> {
        let flow = self
            .flows
            .iter()
            .rposition(|flow| flow.last_stop().is_some())?;
        Some(Caret {
            flow,
            index: self.flows[flow].last_stop()?,
        })
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
}
