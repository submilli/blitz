//! Floats, which end a flow of their inline root.

use super::{Builder, Layer, sibling_points};
use crate::NodeId;
use crate::caret::flow::UnitKind;
use crate::caret::geometry::Edges;

impl Builder<'_> {
    /// A floated or positioned replaced element is a flow of one atom, with
    /// a stop on each side for character movement. It takes no inline
    /// space, so both its sides draw at its start edge. Line movement
    /// treats floats as Chrome does (see [`crate::caret::visible`]).
    pub(super) fn floated_atom(&mut self, id: NodeId) {
        self.split();
        if self.opaque.is_some() || self.order.is_unselectable(id) {
            return;
        }
        let x = self.doc.nodes[id].unrounded_absolute_position(0.0, 0.0).x;
        let Some(line) = self.layers.last().map(Layer::continuing_line) else {
            return;
        };
        let edges = Edges {
            leading: x,
            trailing: x,
            line,
        };
        let around = sibling_points(self.doc, id);
        if let Some((before, _)) = around {
            self.entry(before, self.flow.len(), true);
        }
        self.push_unit(UnitKind::Atom, edges, "\u{FFFC}");
        if let Some((_, after)) = around {
            self.entry(after, self.flow.len(), true);
        }
        self.split();
    }
}
