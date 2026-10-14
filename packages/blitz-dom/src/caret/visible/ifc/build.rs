//! Builds an inline formatting context's offset mapping by walking its
//! content in the order layout construction pushed it to Parley (see
//! `layout/construct.rs`), aligning DOM characters with the laid-out text.

use super::{Ifc, Unit, UnitKind, lines};
use crate::caret::align::{Aligner, TextStyle};
use crate::caret::content::{
    Content, Visit, children_for_stack, classify, inside_marker, is_forced_break,
};
use crate::node::TextLayout;
use crate::{BaseDocument, NodeId};
use std::collections::{HashMap, HashSet};

/// What one content code unit came from, for the line items.
#[derive(Clone, Copy, Debug)]
pub(super) struct Source {
    /// The node whose layout object draws it: a Text node, a `<br>`, an
    /// atomic inline, or a generated Text node.
    pub(super) owner: NodeId,
    pub(super) generated: bool,
    pub(super) kind: SourceKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SourceKind {
    Text { collapsible: bool },
    Break,
    Atomic,
}

/// The mapping as built, with what the line pass needs to place items.
pub(super) struct Mapping {
    pub(super) text: Vec<u16>,
    pub(super) units: Vec<Unit>,
    pub(super) sources: Vec<Source>,
    /// The content offset of each laid-out character, by its layout byte.
    pub(super) bytes: HashMap<usize, usize>,
    /// The content offset of each atomic inline.
    pub(super) atoms: HashMap<NodeId, usize>,
    /// The content offset each float sits at.
    pub(super) floats: HashMap<NodeId, usize>,
}

/// The formatting context of the laid-out inline root `root`.
pub(in crate::caret::visible) fn build(doc: &BaseDocument, root: NodeId) -> Option<Ifc> {
    let node = doc.get_node(root)?;
    let layout = node.element_data()?.inline_layout_data.as_deref()?;
    let mut builder = Builder::new(doc, layout);
    builder.marker(root);
    builder.children(root, false, true);
    builder.drop_trailing_space();
    let Builder {
        mapping,
        ranges,
        members,
        ..
    } = builder;
    let (items, lines) = lines::build(doc, node, layout, &mapping);
    Some(Ifc::new(
        root,
        mapping.text,
        mapping.units,
        ranges,
        members,
        items,
        lines,
    ))
}

struct Builder<'a> {
    doc: &'a BaseDocument,
    aligner: Aligner<'a>,
    mapping: Mapping,
    ranges: HashMap<NodeId, std::ops::Range<usize>>,
    members: HashSet<NodeId>,
    /// A collapsible space ending the content, which a following forced
    /// break or the context's end removes.
    trailing_space: Option<TrailingSpace>,
}

/// Where a collapsible space ending the content was laid out.
#[derive(Clone, Copy)]
struct TrailingSpace {
    /// Its content offset.
    offset: usize,
    /// Its byte in the laid-out text, the key of [`Mapping::bytes`].
    byte: usize,
}

impl<'a> Builder<'a> {
    fn new(doc: &'a BaseDocument, layout: &'a TextLayout) -> Self {
        Self {
            doc,
            aligner: Aligner::new(&layout.text),
            mapping: Mapping {
                text: Vec::new(),
                units: Vec::new(),
                sources: Vec::new(),
                bytes: HashMap::new(),
                atoms: HashMap::new(),
                floats: HashMap::new(),
            },
            ranges: HashMap::new(),
            members: HashSet::new(),
            trailing_space: None,
        }
    }

    /// An inside list marker precedes the root's content in its layout. A
    /// symbol marker's space is its end margin: like the space between
    /// items, it is no item's, so a point over it is nearest the symbol or
    /// the content after it.
    fn marker(&mut self, root: NodeId) {
        let Some(marker) = inside_marker(self.doc, root) else {
            return;
        };
        let style = TextStyle::of(None);
        let start = self.mapping.text.len();
        // Every character aligns with the layout; the margin maps no content.
        let content = marker.text.chars().count() - usize::from(marker.trailing_margin);
        for (index, ch) in marker.text.chars().enumerate() {
            let range = self.aligner.char(ch, style);
            if let Some(range) = range.filter(|_| index < content) {
                self.push_content(
                    ch,
                    range.start,
                    root,
                    true,
                    SourceKind::Text { collapsible: false },
                );
            }
        }
        self.push_generated_unit(start);
    }

    /// Visit a node's content, iteratively: page markup controls the depth.
    /// `pseudos` includes its `::before` and `::after`.
    fn children(&mut self, id: NodeId, generated: bool, pseudos: bool) {
        let mut pending: Vec<Visit> =
            children_for_stack(self.doc, id, generated, pseudos).collect();
        while let Some(Visit { id, generated }) = pending.pop() {
            self.node(id, generated, &mut pending);
        }
    }

    fn node(&mut self, id: NodeId, generated: bool, pending: &mut Vec<Visit>) {
        match classify(self.doc, id) {
            Content::Text => self.text(id, generated),
            Content::Skip => {}
            Content::Contents => {
                pending.extend(children_for_stack(self.doc, id, generated, false));
            }
            Content::Atom => self.atomic(id, generated),
            Content::Break => self.forced_break(id),
            Content::Span => {
                self.members.insert(id);
                pending.extend(children_for_stack(self.doc, id, generated, true));
            }
            Content::Box(_) if crate::caret::order::is_out_of_flow(&self.doc.nodes[id]) => {
                self.members.insert(id);
                let floating = self.doc.nodes[id]
                    .primary_styles()
                    .is_some_and(|style| style.clone_float().is_floating());
                if floating {
                    self.mapping.floats.insert(id, self.mapping.text.len());
                }
            }
            Content::Box(_) => self.atomic(id, generated),
        }
    }

    fn text(&mut self, id: NodeId, generated: bool) {
        let node = &self.doc.nodes[id];
        let Some(data) = node.text_data() else {
            return;
        };
        let parent = node
            .flat_parent_id()
            .or(node.parent)
            .and_then(|parent| self.doc.nodes[parent].primary_styles());
        let style = TextStyle::of(parent.as_deref().map(|s| &**s));
        let content_start = self.mapping.text.len();
        let mut offset = 0;
        for ch in char::decode_utf16(data.content.to_utf16().iter().copied()) {
            let ch = ch.unwrap_or(char::REPLACEMENT_CHARACTER);
            let dom = offset..offset + ch.len_utf16();
            offset = dom.end;
            let collapsible = style.collapsible(ch);
            let Some(range) = self.aligner.char(ch, style) else {
                if !generated {
                    self.push_unit(id, UnitKind::Collapsed, dom, self.mapping.text.len());
                }
                continue;
            };
            let start = self.mapping.text.len();
            let laid_out = self.aligner.slice(range.clone()).to_owned();
            let kind = if is_forced_break(&laid_out) {
                self.drop_trailing_space();
                SourceKind::Break
            } else {
                SourceKind::Text { collapsible }
            };
            for (byte, laid) in laid_out.char_indices() {
                self.push_content(laid, range.start + byte, id, generated, kind);
            }
            if generated {
                continue;
            }
            let length = self.mapping.text.len() - start;
            let unit_kind = if length == dom.len() {
                UnitKind::Identity
            } else {
                UnitKind::Variable
            };
            self.push_unit(id, unit_kind, dom, start);
            if collapsible && kind != SourceKind::Break {
                self.trailing_space = Some(TrailingSpace {
                    offset: start,
                    byte: range.start,
                });
            }
        }
        if generated {
            self.push_generated_unit(content_start);
        }
    }

    /// An atomic inline is one object replacement character.
    fn atomic(&mut self, id: NodeId, generated: bool) {
        self.members.insert(id);
        self.trailing_space = None;
        let start = self.mapping.text.len();
        self.mapping.atoms.insert(id, start);
        self.mapping.text.push(0xFFFC);
        self.mapping.sources.push(Source {
            owner: id,
            generated,
            kind: SourceKind::Atomic,
        });
        if generated {
            return self.push_generated_unit(start);
        }
        self.push_unit(id, UnitKind::Identity, 0..1, start);
    }

    /// A `<br>` is a preserved line feed, before which a collapsible space
    /// is removed.
    fn forced_break(&mut self, id: NodeId) {
        let Some(range) = self.aligner.char('\n', TextStyle::preserved()) else {
            return;
        };
        self.drop_trailing_space();
        let start = self.mapping.text.len();
        self.push_content('\n', range.start, id, false, SourceKind::Break);
        self.push_unit(id, UnitKind::Identity, 0..1, start);
    }

    fn push_content(
        &mut self,
        ch: char,
        byte: usize,
        owner: NodeId,
        generated: bool,
        kind: SourceKind,
    ) {
        if !matches!(kind, SourceKind::Text { collapsible: true }) {
            self.trailing_space = None;
        }
        let offset = self.mapping.text.len();
        self.mapping.bytes.insert(byte, offset);
        let mut units = [0; 2];
        for &unit in ch.encode_utf16(&mut units).iter() {
            self.mapping.text.push(unit);
            self.mapping.sources.push(Source {
                owner,
                generated,
                kind,
            });
        }
    }

    /// Record a DOM unit ending the content, merging it into the previous
    /// unit of the node when they continue each other.
    fn push_unit(
        &mut self,
        node: NodeId,
        kind: UnitKind,
        dom: std::ops::Range<usize>,
        start: usize,
    ) {
        let content = start..self.mapping.text.len();
        if let Some(last) = self.mapping.units.last_mut()
            && last.node == Some(node)
            && last.kind == kind
            && kind != UnitKind::Variable
            && last.dom.end == dom.start
            && last.content.end == content.start
        {
            last.dom.end = dom.end;
            last.content.end = content.end;
            return;
        }
        let index = self.mapping.units.len();
        self.mapping.units.push(Unit {
            node: Some(node),
            kind,
            dom,
            content,
        });
        self.ranges
            .entry(node)
            .and_modify(|range| range.end = index + 1)
            .or_insert(index..index + 1);
    }

    fn push_generated_unit(&mut self, start: usize) {
        let end = self.mapping.text.len();
        if end == start {
            return;
        }
        self.mapping.units.push(Unit {
            node: None,
            kind: UnitKind::Identity,
            dom: 0..end - start,
            content: start..end,
        });
    }

    /// Remove a collapsible space ending the content before a forced break
    /// or the context's end (Blink's `RemoveTrailingCollapsibleSpace`).
    fn drop_trailing_space(&mut self) {
        let Some(TrailingSpace { offset, byte }) = self.trailing_space.take() else {
            return;
        };
        if offset + 1 != self.mapping.text.len() {
            return;
        }
        self.mapping.text.pop();
        self.mapping.sources.pop();
        self.mapping.bytes.remove(&byte);
        let Some(index) = self
            .mapping
            .units
            .iter()
            .rposition(|unit| unit.content.contains(&offset))
        else {
            return;
        };
        let unit = self.mapping.units[index].clone();
        let Some(node) = unit.node else {
            return;
        };
        let space_dom = unit.dom.end - 1;
        let collapsed = Unit {
            node: Some(node),
            kind: UnitKind::Collapsed,
            dom: space_dom..unit.dom.end,
            content: offset..offset,
        };
        let kept = Unit {
            dom: unit.dom.start..space_dom,
            content: unit.content.start..offset,
            ..unit
        };
        let mut replacement = Vec::new();
        if !kept.dom.is_empty() {
            replacement.push(kept);
        }
        replacement.push(collapsed);
        // Units after the space (collapsed ones, of this node or later
        // ones) keep their DOM order; shift the ranges that reach past the
        // space by any added unit. Only units pushed after the space follow
        // it, and each space is dropped once, so the shifts stay linear.
        let added = replacement.len() - 1;
        let mut shifted: HashSet<NodeId> = self.mapping.units[index + 1..]
            .iter()
            .filter_map(|unit| unit.node)
            .collect();
        shifted.insert(node);
        self.mapping.units.splice(index..=index, replacement);
        for node in shifted {
            let Some(range) = self.ranges.get_mut(&node) else {
                continue;
            };
            if range.start > index {
                range.start += added;
            }
            if range.end > index {
                range.end += added;
            }
        }
    }
}
