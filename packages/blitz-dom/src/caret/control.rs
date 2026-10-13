//! Caret movement inside a focused text control, over its editor layout.
//! Positions are UTF-16 offsets into the control's value.

use super::flow::{Entry, Flow, UnitKind};
use super::geometry::LayoutGeometry;
use super::navigate::{self, Caret, Carets, Flows};
use super::order::Key;
use super::{Alter, ModifyRequest};
use crate::form_selection::{ControlSelection, SelectionDirection, utf16_to_byte};
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
        let anchor_start = self.doc.control_anchor_is_start(id, &selection, request)?;
        let (anchor, focus) = if anchor_start {
            (selection.start, selection.end)
        } else {
            (selection.end, selection.start)
        };
        let mut focus = flow.caret(focus)?;
        focus.upstream = request.memory.upstream;
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

impl BaseDocument {
    /// Whether a step starts from a control's selection with its anchor at
    /// the start. A range whose direction is "none" is not directional: an
    /// extension grows it at the side it moves toward
    /// (`PrepareToModifySelection`).
    fn control_anchor_is_start(
        &self,
        id: NodeId,
        selection: &ControlSelection,
        request: ModifyRequest,
    ) -> Option<bool> {
        let oriented = request.alter == Alter::Extend && selection.start < selection.end;
        Some(match selection.direction {
            SelectionDirection::Forward => true,
            SelectionDirection::Backward => false,
            SelectionDirection::None if !oriented => true,
            SelectionDirection::None => {
                // Bounded by the control's value; its layout exists, since
                // the step's flow was built from it.
                let rtl = self.control_selection_rtl(id, selection)?;
                request.direction.is_forward(|| rtl)
            }
        })
    }

    /// `DirectionOfSelection` for a control's range: right to left when
    /// the text at its first and last characters is, left to right when
    /// both are, else the control's direction.
    fn control_selection_rtl(&self, id: NodeId, selection: &ControlSelection) -> Option<bool> {
        let node = self.get_node(id)?;
        let input = node.element_data()?.text_input_data()?;
        let layout = input.editor.try_layout()?;
        let text = input.editor.raw_text();
        let geometry = LayoutGeometry::new(layout, node);
        let first = geometry.rtl_at(utf16_to_byte(text, selection.start));
        let last = geometry.rtl_at(utf16_to_byte(text, selection.end).saturating_sub(1));
        Some(match (first, last) {
            (Some(first), Some(last)) if first == last => first,
            _ => layout.is_rtl(),
        })
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
