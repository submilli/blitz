//! Caret movement inside a focused text control, over its editor layout.
//! Positions are UTF-16 offsets into the control's value.

use super::ModifyRequest;
use super::flow::{Entry, Flow, UnitKind};
use super::geometry::LayoutGeometry;
use super::navigate::{self, Caret, Carets, Flows};
use super::order::Key;
use crate::form_selection::{ControlSelection, SelectionDirection};
use crate::{BaseDocument, DocumentMutator, NodeId, ranges::Boundary};
use icu_segmenter::GraphemeClusterSegmenter;

/// A text control's selection after movement.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ControlModified {
    /// The horizontal position to pass to the next line or paragraph step.
    pub line_x: Option<f32>,
    /// The focus affinity to pass to the next step.
    pub upstream: bool,
}

impl DocumentMutator<'_> {
    /// Move or extend a text control's selection, which is the document's
    /// selection while the control is focused. Layout must be current.
    /// Returns `None` when the control has no text selection or no current
    /// layout.
    pub fn modify_control_selection(
        &mut self,
        id: NodeId,
        request: ModifyRequest,
    ) -> Option<ControlModified> {
        let selection = self.doc.control_selection(id)?;
        let flow = SingleFlow(control_flow(self.doc, id)?);
        let (anchor, focus) = match selection.direction {
            SelectionDirection::Backward => (selection.end, selection.start),
            _ => (selection.start, selection.end),
        };
        let mut focus = flow.caret(focus)?;
        focus.upstream = request.upstream;
        let carets = Carets {
            anchor: flow.caret(anchor)?,
            focus,
        };
        let (moved, line_x) = navigate::modify(&flow, carets, request)?;
        let anchor = flow.0.point(moved.anchor.index)?.offset;
        let upstream = moved.focus.upstream;
        let focus = flow.0.point(moved.focus.index)?.offset;
        let direction = match anchor.cmp(&focus) {
            std::cmp::Ordering::Less => SelectionDirection::Forward,
            std::cmp::Ordering::Equal => SelectionDirection::None,
            std::cmp::Ordering::Greater => SelectionDirection::Backward,
        };
        let selection = ControlSelection {
            start: anchor.min(focus),
            end: anchor.max(focus),
            direction,
        };
        self.set_control_selection(id, selection);
        Some(ControlModified { line_x, upstream })
    }
}

struct SingleFlow(Flow);

impl SingleFlow {
    /// The stop at a UTF-16 offset; an offset inside a grapheme takes the
    /// next one.
    fn caret(&self, offset: usize) -> Option<Caret> {
        let key = Key::control(offset);
        let index = self.0.exact(key, false).or_else(|| self.0.after(key))?;
        Some(Caret::at(0, index))
    }
}

impl Flows for SingleFlow {
    fn count(&self) -> usize {
        1
    }

    fn flow(&self, index: usize) -> Option<&Flow> {
        (index == 0).then_some(&self.0)
    }
}

/// Every grapheme boundary of the value is a stop; a line feed is a forced
/// break.
fn control_flow(doc: &BaseDocument, id: NodeId) -> Option<Flow> {
    let node = doc.get_node(id)?;
    let input = node.element_data()?.text_input_data()?;
    let layout = input.editor.try_layout()?;
    let text = input.editor.raw_text();
    let geometry = LayoutGeometry::new(layout, node);
    let mut flow = Flow::new(id, layout.is_rtl());
    let boundaries: Vec<usize> = GraphemeClusterSegmenter::new().segment_str(text).collect();
    let mut offset = 0;
    for pair in boundaries.windows(2) {
        let cluster = &text[pair[0]..pair[1]];
        flow.push_entry(entry(id, offset, flow.len()));
        let kind = if cluster == "\n" || cluster == "\r\n" {
            UnitKind::Break
        } else {
            UnitKind::Text
        };
        flow.push_unit(kind, geometry.text(pair[0], pair[1]), cluster);
        offset += cluster.encode_utf16().count();
    }
    flow.push_entry(entry(id, offset, flow.len()));
    Some(flow.finish())
}

fn entry(id: NodeId, offset: usize, index: usize) -> Entry {
    Entry {
        key: Key::control(offset),
        point: Boundary { node: id, offset },
        index,
        candidate: true,
    }
}
