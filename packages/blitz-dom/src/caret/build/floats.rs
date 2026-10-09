//! Floats and embedded boxes, which end a flow and decide what the stops
//! beside a float stand for (see [`crate::caret::float`]).

use super::{Builder, Layer, sibling_points};
use crate::NodeId;
use crate::caret::float::{self, Beside, FloatStops, Walked};
use crate::caret::flow::{Flow, UnitKind};
use crate::caret::geometry::Edges;
use crate::caret::order::is_out_of_flow;
use style::values::specified::box_::DisplayOutside;

impl Builder<'_> {
    /// A floated or positioned replaced element is a flow of one atom on the
    /// line it splits. It takes no inline space, so both its sides draw at
    /// its start edge.
    pub(super) fn floated_atom(&mut self, id: NodeId) {
        let closed = self.flows.len();
        let collapsible_end = self.hanging_space;
        self.split();
        if self.opaque.is_some() {
            return;
        }
        if self.order.is_unselectable(id) {
            // It takes no stops, but a float stop before it stands for
            // nothing past it, as for a box without content.
            let line = self.layers.last().map_or(0, Layer::continuing_line);
            return self.reach(Walked::Box { end: None, line });
        }
        let previous = self.flows.get(closed);
        let before = float::before_stop(self.line_start, self.last, previous, collapsible_end);
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
        let follows_content = match self.last {
            Walked::Float(index) => self
                .flows
                .get(index)
                .and_then(Flow::float_stops)
                .is_some_and(|stops| stops.follows_content),
            _ => before == Beside::Previous,
        };
        self.flow.set_float_stops(FloatStops {
            before,
            after: Beside::LineEnd,
            after_break: matches!(self.last, Walked::Break),
            follows_content,
        });
        let index = self.flows.len();
        self.split();
        if self.flows.len() > index {
            self.reach(Walked::Float(index));
        }
    }

    /// A box whose content forms flows of its own ends this flow. Only an
    /// in-flow inline-level box or a float sits beside the line's floats;
    /// block-level and absolutely positioned boxes leave them as they are.
    pub(super) fn end_at_embedded_box(&mut self, id: NodeId) {
        self.split();
        let node = &self.doc.nodes[id];
        let floated = node
            .primary_styles()
            .is_some_and(|style| style.clone_float().is_floating());
        let inline_level = node
            .display_style()
            .is_some_and(|display| display.outside() == DisplayOutside::Inline);
        if floated || (inline_level && !is_out_of_flow(node)) {
            let line = self.layers.last().map_or(0, |layer| layer.box_line(id));
            let end = self.order.end(id);
            self.reach(Walked::Box { end, line });
            self.line_start = false;
        }
    }

    /// Move past `next`, which decides what the stop after a float just
    /// passed stands for.
    pub(super) fn reach(&mut self, next: Walked) {
        if let Walked::Float(index) = self.last
            && let Some(flow) = self.flows.get_mut(index)
            && let Some(stops) = flow.float_stops()
        {
            flow.set_float_after(float::after_stop(next, stops));
        }
        self.last = next;
    }
}
