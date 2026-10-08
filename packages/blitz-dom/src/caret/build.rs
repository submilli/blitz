//! Builds the flows of an inline root by walking its content in the order
//! layout construction pushed it to Parley (see `layout/construct.rs`).
//!
//! A root yields several flows when it embeds boxes that do not continue its
//! flow (floats, positioned boxes, inline-level boxes with block content):
//! their content forms flows of its own, between the root's flows in tree
//! order, and entering or leaving it is a step.

use super::align::{Aligner, TextStyle};
use super::flow::{Entry, Flow, UnitKind};
use super::geometry::{Edges, LayoutGeometry};
use super::order::{TreeOrder, flattened_root, is_out_of_flow};
use crate::layout::construct::{InlineElement, inline_element};
use crate::node::{ListItemLayoutPosition, Marker, NodeData, TextLayout};
use crate::{BaseDocument, NodeId, ranges::Boundary};
use style::values::computed::{Display, UserSelect};
use style::values::specified::box_::{DisplayInside, DisplayOutside};

/// The flows of a laid-out inline root, empty without a current layout.
pub(super) fn document_flows(doc: &BaseDocument, order: &TreeOrder, root: NodeId) -> Vec<Flow> {
    let Some(text) = inline_layout(doc, root) else {
        return Vec::new();
    };
    let rtl = text.layout.is_rtl();
    let mut builder = Builder {
        doc,
        order,
        root,
        flows: Vec::new(),
        flow: Flow::new(root, rtl),
        layers: vec![Layer::new(&doc.nodes[root], text, None)],
        hanging_space: false,
        opaque: None,
    };
    builder.marker(root);
    if unselectable(doc, root) {
        builder.opaque = Some(Opaque {
            element: root,
            edges: None,
        });
    }
    builder.walk(root);
    builder.leave_opaque(root);
    builder.drop_hanging_space();
    builder.split();
    builder.flows
}

fn inline_layout(doc: &BaseDocument, id: NodeId) -> Option<&TextLayout> {
    doc.get_node(id)?
        .element_data()?
        .inline_layout_data
        .as_deref()
}

fn unselectable(doc: &BaseDocument, id: NodeId) -> bool {
    doc.nodes[id]
        .primary_styles()
        .is_some_and(|s| s.clone_user_select() == UserSelect::None)
}

/// One Parley layout being aligned.
struct Layer<'a> {
    aligner: Aligner<'a>,
    geometry: LayoutGeometry,
    /// A nested inline root draws on the line of its box in the outer root.
    line: Option<u32>,
}

impl<'a> Layer<'a> {
    fn new(node: &crate::Node, text: &'a TextLayout, line: Option<u32>) -> Self {
        Self {
            aligner: Aligner::new(&text.text),
            geometry: LayoutGeometry::new(&text.layout, node),
            line,
        }
    }

    fn edges(&self, start: usize, end: usize) -> Edges {
        let mut edges = self.geometry.text(start, end);
        if let Some(line) = self.line {
            edges.line = line;
        }
        edges
    }
}

/// Content of a `user-select: none` element is one opaque unit.
struct Opaque {
    element: NodeId,
    edges: Option<Edges>,
}

enum Step {
    Node { id: NodeId, generated: bool },
    LeaveOpaque(NodeId),
    LeaveLayer,
}

struct Builder<'a> {
    doc: &'a BaseDocument,
    order: &'a TreeOrder,
    root: NodeId,
    flows: Vec<Flow>,
    flow: Flow,
    layers: Vec<Layer<'a>>,
    /// The last unit is collapsible white space, which hangs and takes no
    /// caret position when the line ends after it.
    hanging_space: bool,
    opaque: Option<Opaque>,
}

impl Builder<'_> {
    /// An inside list marker precedes the root's content in its layout.
    fn marker(&mut self, root: NodeId) {
        let Some(item) = self.doc.nodes[root]
            .element_data()
            .and_then(|e| e.list_item_data.as_deref())
        else {
            return;
        };
        if !matches!(item.position, ListItemLayoutPosition::Inside) {
            return;
        }
        let text = match &item.marker {
            Marker::Char(ch) => format!("{ch} "),
            Marker::String(text) => text.clone(),
        };
        let style = TextStyle::of(None);
        for ch in text.chars() {
            self.align(ch, style);
        }
    }

    /// Visit content iteratively: page markup controls the nesting depth.
    fn walk(&mut self, root: NodeId) {
        let mut pending = Vec::new();
        push_children(self.doc, root, &mut pending, false);
        while let Some(step) = pending.pop() {
            match step {
                Step::Node { id, generated } => self.node(id, generated, &mut pending),
                Step::LeaveOpaque(id) => self.leave_opaque(id),
                Step::LeaveLayer => {
                    self.layers.pop();
                }
            }
        }
    }

    fn node(&mut self, id: NodeId, generated: bool, pending: &mut Vec<Step>) {
        let node = &self.doc.nodes[id];
        let element = match &node.data {
            NodeData::Text(_) => return self.text(id, generated),
            NodeData::Element(element) | NodeData::AnonymousBlock(element) => element,
            _ => return,
        };
        let kind = inline_element(element);
        if kind == InlineElement::Hidden {
            return;
        }
        let display = node.display_style().unwrap_or(Display::inline());
        match (display.outside(), display.inside()) {
            (DisplayOutside::None, DisplayInside::None) => {}
            (DisplayOutside::None, DisplayInside::Contents) => {
                let children = node.flat_children().iter().rev();
                pending.extend(children.map(|&id| Step::Node { id, generated }));
            }
            (DisplayOutside::Inline, DisplayInside::Flow) => match kind {
                InlineElement::Atom => self.atom(id),
                InlineElement::Break => self.forced_break(id),
                InlineElement::Span | InlineElement::Hidden => self.span(id, generated, pending),
            },
            _ => self.inline_box(id, kind, pending),
        }
    }

    fn span(&mut self, id: NodeId, generated: bool, pending: &mut Vec<Step>) {
        if self.opaque.is_none() && !generated && unselectable(self.doc, id) {
            self.opaque = Some(Opaque {
                element: id,
                edges: None,
            });
            pending.push(Step::LeaveOpaque(id));
        }
        push_children(self.doc, id, pending, generated);
    }

    /// An inline-level box whose own inline content continues this flow
    /// becomes a nested layer; a replaced element or control is an atom; any
    /// other box ends this flow, its content forming flows of its own.
    fn inline_box(&mut self, id: NodeId, kind: InlineElement, pending: &mut Vec<Step>) {
        if is_out_of_flow(&self.doc.nodes[id]) {
            return self.split();
        }
        if kind == InlineElement::Atom {
            return self.atom(id);
        }
        let Some(inner) = flattened_root(self.doc, id) else {
            return self.split();
        };
        let Some(text) = inline_layout(self.doc, inner) else {
            return self.split();
        };
        let line = self.layers.last().and_then(|layer| {
            let edge = layer.geometry.inline_box(id.as_u64())?;
            Some(layer.line.unwrap_or(edge.line))
        });
        self.layers
            .push(Layer::new(&self.doc.nodes[inner], text, line));
        pending.push(Step::LeaveLayer);
        push_children(self.doc, inner, pending, false);
    }

    fn text(&mut self, id: NodeId, generated: bool) {
        let node = &self.doc.nodes[id];
        let Some(data) = node.text_data() else {
            return;
        };
        let parent = node
            .flat_parent_id()
            .and_then(|parent| self.doc.nodes[parent].primary_styles());
        let style = TextStyle::of(parent.as_deref().map(|s| &**s));
        if generated {
            // Generated content takes layout text but no caret positions.
            for ch in data.content.as_str_lossy().chars() {
                self.align(ch, style);
            }
            return;
        }
        let selectable = style.visible && self.opaque.is_none();
        let mut offset = 0;
        for ch in char::decode_utf16(data.content.to_utf16().iter().copied()) {
            let ch = ch.unwrap_or(char::REPLACEMENT_CHARACTER);
            self.character(Boundary { node: id, offset }, ch, style, selectable);
            offset += ch.len_utf16();
        }
        let end = Boundary { node: id, offset };
        self.entry(end, self.flow.len(), selectable);
    }

    /// Record one DOM character: a new unit when it starts a rendered
    /// grapheme, part of the last unit inside one, or a position only.
    fn character(&mut self, point: Boundary, ch: char, style: TextStyle, selectable: bool) {
        let Some(range) = self.align(ch, style) else {
            return self.entry(point, self.flow.len(), selectable);
        };
        let Some(layer) = self.layers.last() else {
            return;
        };
        let edges = layer.edges(range.start, range.end);
        let text = layer.aligner.slice(range.clone());
        let continues = !layer.aligner.starts_grapheme(range.start);
        if !selectable {
            self.entry(point, self.flow.len(), false);
            if style.visible {
                self.extend_opaque(edges);
            }
            return;
        }
        if continues && !self.flow.is_empty() {
            self.entry(point, self.flow.len() - 1, false);
            if self.flow.extend_text(text, edges.trailing) {
                return;
            }
        }
        self.entry(point, self.flow.len(), true);
        if text == "\n" {
            self.drop_hanging_space();
            self.push_unit(UnitKind::Break, edges, text);
            return;
        }
        self.push_unit(UnitKind::Text, edges, text);
        self.hanging_space = style.collapsible(ch);
    }

    fn atom(&mut self, id: NodeId) {
        let Some(layer) = self.layers.last() else {
            return;
        };
        let Some(edge) = layer.geometry.inline_box(id.as_u64()) else {
            return;
        };
        let (leading, trailing) = if self.flow.rtl() {
            (edge.right, edge.left)
        } else {
            (edge.left, edge.right)
        };
        let edges = Edges {
            leading,
            trailing,
            line: layer.line.unwrap_or(edge.line),
        };
        if self.opaque.is_some() {
            return self.extend_opaque(edges);
        }
        let around = sibling_points(self.doc, id);
        if let Some((before, _)) = around {
            self.entry(before, self.flow.len(), true);
        }
        self.push_unit(UnitKind::Atom, edges, "\u{FFFC}");
        if let Some((_, after)) = around {
            self.entry(after, self.flow.len(), true);
        }
    }

    /// A `<br>` is a unit of its own; the line ends before it, so collapsible
    /// white space ahead of it hangs. The position after it belongs to the
    /// following content.
    fn forced_break(&mut self, id: NodeId) {
        let Some(range) = self.align('\n', TextStyle::preserved()) else {
            return;
        };
        let Some(edges) = self.layers.last().map(|l| l.edges(range.start, range.end)) else {
            return;
        };
        if self.opaque.is_some() {
            return self.extend_opaque(edges);
        }
        self.drop_hanging_space();
        let around = sibling_points(self.doc, id);
        if let Some((before, _)) = around {
            self.entry(before, self.flow.len(), true);
        }
        self.push_unit(UnitKind::Break, edges, "\n");
        if let Some((_, after)) = around {
            self.entry(after, self.flow.len(), false);
        }
    }

    fn leave_opaque(&mut self, id: NodeId) {
        if self.opaque.as_ref().is_none_or(|o| o.element != id) {
            return;
        }
        if let Some(edges) = self.opaque.take().and_then(|o| o.edges) {
            self.push_unit(UnitKind::Atom, edges, "\u{FFFC}");
        }
    }

    fn extend_opaque(&mut self, edges: Edges) {
        if let Some(opaque) = &mut self.opaque {
            opaque.edges = Some(match opaque.edges {
                Some(first) => Edges {
                    trailing: edges.trailing,
                    ..first
                },
                None => edges,
            });
        }
    }

    fn align(&mut self, ch: char, style: TextStyle) -> Option<std::ops::Range<usize>> {
        self.layers.last_mut()?.aligner.char(ch, style)
    }

    fn push_unit(&mut self, kind: UnitKind, edges: Edges, text: &str) {
        self.flow.push_unit(kind, edges, text);
        self.hanging_space = false;
    }

    /// Remove a final collapsible space before a line end.
    fn drop_hanging_space(&mut self) {
        if std::mem::take(&mut self.hanging_space) {
            self.flow.pop_unit();
        }
    }

    /// End the current flow at an embedded box; a space before it is kept.
    fn split(&mut self) {
        self.hanging_space = false;
        let rtl = self.flow.rtl();
        let flow = std::mem::replace(&mut self.flow, Flow::new(self.root, rtl));
        if flow.first_key().is_some() {
            self.flows.push(flow.finish());
        }
    }

    fn entry(&mut self, point: Boundary, index: usize, candidate: bool) {
        let key = if self.doc.nodes[point.node].is_text_node() {
            self.order.text(point.node, point.offset)
        } else {
            self.order.key(self.doc, point)
        };
        if let Some(key) = key {
            self.flow.push_entry(Entry {
                key,
                point,
                index,
                candidate,
            });
        }
    }
}

/// Children in layout construction order, pushed for a stack walk.
fn push_children(doc: &BaseDocument, id: NodeId, pending: &mut Vec<Step>, generated: bool) {
    let node = &doc.nodes[id];
    if let Some(after) = node.after() {
        pending.push(Step::Node {
            id: after,
            generated: true,
        });
    }
    let children = node.flat_children().iter().rev();
    pending.extend(children.map(|&id| Step::Node { id, generated }));
    if let Some(before) = node.before() {
        pending.push(Step::Node {
            id: before,
            generated: true,
        });
    }
}

/// The boundary points before and after `id` in its DOM parent.
fn sibling_points(doc: &BaseDocument, id: NodeId) -> Option<(Boundary, Boundary)> {
    let parent = doc.nodes[id].parent?;
    let index = doc.nodes[parent].children.position(id)?;
    Some((
        Boundary {
            node: parent,
            offset: index,
        },
        Boundary {
            node: parent,
            offset: index + 1,
        },
    ))
}
