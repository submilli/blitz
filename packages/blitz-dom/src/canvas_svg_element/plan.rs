//! What one element reference serializes: the subtree, the copies its
//! references need, and which references cannot be kept.
//!
//! A reference whose target's expansion leads back to an element containing
//! it is cyclic, and is left out as Blink leaves out a `<use>` that would
//! instantiate itself. The rest form a graph without cycles, so the size of
//! every `<use>` expansion is known before the parser builds it.

use super::{HIDDEN_CONTAINERS, MAX_ELEMENT_NODES, MAX_EXPANDED_NODES};
use crate::canvas_svg_filter::CanvasSvgLimit;
use crate::{BaseDocument, NodeId};
use std::collections::HashMap;
use style::values::generics::svg::SVGPaintKind;

/// Where an element names another element.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Slot {
    Href,
    Fill,
    Stroke,
    MarkerStart,
    MarkerMid,
    MarkerEnd,
}

impl Slot {
    pub(super) const MARKERS: [(&str, Slot); 3] = [
        ("marker-start", Slot::MarkerStart),
        ("marker-mid", Slot::MarkerMid),
        ("marker-end", Slot::MarkerEnd),
    ];
}

/// How a serialized subtree is used.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Kind {
    /// The referenced element itself.
    Root,
    /// A `<use>` target, which inherits from each `<use>`.
    Instance,
    /// A paint server, marker or text path, styled where it stands.
    Resource,
}

/// One serialized subtree.
#[derive(Debug)]
pub(super) struct Source {
    pub node: NodeId,
    pub kind: Kind,
}

/// A reference found while walking one source.
struct Owned {
    element: NodeId,
    slot: Slot,
    target: usize,
    /// A `<use>` outside hidden containers, whose content Blink lays out.
    visible_instance: bool,
}

/// The subtrees to serialize and the references kept between them.
pub(super) struct Plan {
    /// The referenced element first, then the copies its kept references
    /// reach.
    pub sources: Vec<Source>,
    references: HashMap<(NodeId, Slot), usize>,
    /// Whether Blink would map the viewport onto the subregion
    /// (`HasVisibleViewportDependence` in `svg_fe_image.cc`).
    pub viewport_dependent: bool,
}

impl Plan {
    pub(super) fn new(doc: &BaseDocument, root: NodeId) -> Result<Self, CanvasSvgLimit> {
        let mut builder = Builder {
            doc,
            ids: None,
            sources: vec![Source {
                node: root,
                kind: Kind::Root,
            }],
            index: HashMap::new(),
            owned: Vec::new(),
            nodes: Vec::new(),
            self_dependent: Vec::new(),
            visited: 0,
        };
        let mut next = 0;
        while next < builder.sources.len() {
            builder.walk(next)?;
            next += 1;
        }
        builder.finish()
    }

    /// The copy `element` names at `slot`, unless it is missing or cyclic.
    pub(super) fn reference(&self, element: NodeId, slot: Slot) -> Option<usize> {
        self.references.get(&(element, slot)).copied()
    }
}

struct Builder<'a> {
    doc: &'a BaseDocument,
    /// Elements by ID in the root's tree, read on the first reference.
    ids: Option<HashMap<String, NodeId>>,
    sources: Vec<Source>,
    index: HashMap<(NodeId, Kind), usize>,
    /// Per source, as walked.
    owned: Vec<Vec<Owned>>,
    nodes: Vec<usize>,
    self_dependent: Vec<bool>,
    /// Elements walked across all sources.
    visited: usize,
}

impl Builder<'_> {
    /// Walk source `index`, recording its references and copies to make.
    fn walk(&mut self, index: usize) -> Result<(), CanvasSvgLimit> {
        let Source { node: root, kind } = self.sources[index];
        let mut owned = Vec::new();
        let mut nodes = 0;
        let mut dependent = false;
        // A resource never paints where it stands; an instanced symbol does.
        let mut pending = vec![(root, kind == Kind::Resource)];
        while let Some((id, hidden)) = pending.pop() {
            let Some(tag) = included(self.doc, id) else {
                continue;
            };
            nodes += 1;
            self.visited += 1;
            let node = &self.doc.nodes[id];
            if self.visited > MAX_ELEMENT_NODES
                || pending.len() + node.children.len() > MAX_ELEMENT_NODES * 2
            {
                return Err(CanvasSvgLimit);
            }
            let instanced_symbol = id == root && kind == Kind::Instance;
            let hidden = hidden || (HIDDEN_CONTAINERS.contains(&tag) && !instanced_symbol);
            let element = node.element_data().expect("included nodes are elements");
            let style = node.primary_styles().expect("included nodes are styled");
            dependent |= !hidden && super::viewport::self_dependent(tag, element, &style);
            for (slot, id_ref, target_kind) in references(tag, element, &style) {
                let Some(target) = self.target(&id_ref, target_kind)? else {
                    continue;
                };
                owned.push(Owned {
                    element: id,
                    slot,
                    target,
                    visible_instance: !hidden && target_kind == Kind::Instance,
                });
            }
            pending.extend(node.children.iter().rev().map(|child| (*child, hidden)));
        }
        self.owned.push(owned);
        self.nodes.push(nodes);
        self.self_dependent.push(dependent);
        Ok(())
    }

    /// The copy of the element `id` names, made when first needed. A target
    /// that is not a rendered SVG element is no reference.
    fn target(&mut self, id: &str, kind: Kind) -> Result<Option<usize>, CanvasSvgLimit> {
        if self.ids.is_none() {
            self.ids = Some(self.doc.svg_ids(self.sources[0].node)?);
        }
        let ids = self.ids.as_ref().expect("IDs were just read");
        let Some(&node) = ids.get(id) else {
            return Ok(None);
        };
        if included(self.doc, node).is_none() {
            return Ok(None);
        }
        let next = self.sources.len();
        let index = *self.index.entry((node, kind)).or_insert(next);
        if index == next {
            self.sources.push(Source { node, kind });
        }
        Ok(Some(index))
    }

    /// Drop cyclic references, then bound the expansion and resolve the
    /// viewport dependence over the graph left.
    fn finish(self) -> Result<Plan, CanvasSvgLimit> {
        let edges: Vec<Vec<usize>> = self
            .owned
            .iter()
            .map(|owned| owned.iter().map(|owned| owned.target).collect())
            .collect();
        let (component, order) = components(&edges);
        // The same element is walked once per source containing it; any of
        // those sources reached back from its target makes it cyclic.
        let mut cyclic: HashMap<(NodeId, Slot), bool> = HashMap::new();
        for (source, owned) in self.owned.iter().enumerate() {
            for owned in owned {
                let back = component[source] == component[owned.target];
                *cyclic.entry((owned.element, owned.slot)).or_default() |= back;
            }
        }
        let kept = |owned: &Owned| cyclic.get(&(owned.element, owned.slot)) == Some(&false);

        // Only copies kept references reach are written.
        let mut reached = vec![false; self.sources.len()];
        reached[0] = true;
        let mut pending = vec![0];
        while let Some(source) = pending.pop() {
            for owned in self.owned[source].iter().filter(|owned| kept(owned)) {
                if !reached[owned.target] {
                    reached[owned.target] = true;
                    pending.push(owned.target);
                }
            }
        }

        // Components come after every component they reach, so each source's
        // kept targets are known before it.
        let mut expanded = vec![0usize; self.sources.len()];
        let mut dependent = self.self_dependent.clone();
        for members in &order {
            for &source in members {
                for owned in self.owned[source].iter().filter(|owned| kept(owned)) {
                    if self.sources[owned.target].kind == Kind::Instance {
                        expanded[source] = expanded[source].saturating_add(expanded[owned.target]);
                    }
                    dependent[source] |= owned.visible_instance && dependent[owned.target];
                }
                expanded[source] = expanded[source].saturating_add(self.nodes[source]);
            }
        }
        let total = self
            .sources
            .iter()
            .zip(&expanded)
            .zip(&reached)
            .filter(|((source, _), reached)| **reached && source.kind != Kind::Instance)
            .fold(0usize, |total, ((_, expanded), _)| {
                total.saturating_add(*expanded)
            });
        if total > MAX_EXPANDED_NODES {
            return Err(CanvasSvgLimit);
        }
        // Renumber the reached sources in order, so copy IDs stay dense.
        let mut renumbered = vec![None; self.sources.len()];
        let mut sources = Vec::new();
        for (index, source) in self.sources.into_iter().enumerate() {
            if reached[index] {
                renumbered[index] = Some(sources.len());
                sources.push(source);
            }
        }
        let references = self
            .owned
            .iter()
            .flatten()
            .filter(|owned| kept(owned))
            .filter_map(|owned| Some(((owned.element, owned.slot), renumbered[owned.target]?)))
            .collect();
        Ok(Plan {
            sources,
            references,
            viewport_dependent: dependent[0],
        })
    }
}

/// The tag of `id` when it enters serialized markup: a styled SVG element
/// that paints or is referenced. Scripts, styles, event handlers and the
/// effects Blink leaves out (clips, masks and filters) never enter, nor names
/// the HTML parser accepts but XML does not.
pub(super) fn included(doc: &BaseDocument, id: NodeId) -> Option<&str> {
    let node = &doc.nodes[id];
    let element = node.element_data()?;
    let tag = element.name.local.as_ref();
    let skipped = matches!(
        tag,
        "script"
            | "style"
            | "clipPath"
            | "mask"
            | "filter"
            | "foreignObject"
            | "title"
            | "desc"
            | "metadata"
    );
    if element.name.ns != markup5ever::ns!(svg)
        || skipped
        || !super::markup::xml_name(tag)
        || node.primary_styles().is_none()
    {
        return None;
    }
    Some(tag)
}

/// The elements `element` names, with how each is used. `url()` paints are
/// computed absolute, so any fragment names a local element.
fn references(
    tag: &str,
    element: &crate::node::ElementData,
    style: &style::properties::ComputedValues,
) -> Vec<(Slot, String, Kind)> {
    let mut found = Vec::new();
    let href_kind = match tag {
        "use" => Some(Kind::Instance),
        "linearGradient" | "radialGradient" | "pattern" | "textPath" => Some(Kind::Resource),
        _ => None,
    };
    if let Some(kind) = href_kind
        && let Some(id) = href(element).and_then(|href| fragment(href.strip_prefix('#')?))
    {
        found.push((Slot::Href, id, kind));
    }
    for (slot, paint) in [
        (Slot::Fill, style.clone_fill()),
        (Slot::Stroke, style.clone_stroke()),
    ] {
        if let SVGPaintKind::PaintServer(url) = &paint.kind
            && let Some(id) = url.url().and_then(|url| fragment(url.fragment()?))
        {
            found.push((slot, id, Kind::Resource));
        }
    }
    for (name, slot) in Slot::MARKERS {
        let value = element.attr(markup5ever::LocalName::from(name));
        if let Some(id) = value.and_then(marker_fragment) {
            found.push((slot, id, Kind::Resource));
        }
    }
    found
}

/// The `href` an element uses: SVG 2's, else XLink's.
pub(super) fn href(element: &crate::node::ElementData) -> Option<&str> {
    let svg = element
        .attrs()
        .iter()
        .find(|attr| attr.name.ns == markup5ever::ns!() && attr.name.local.as_ref() == "href");
    let xlink = || {
        element.attrs().iter().find(|attr| {
            attr.name.ns == markup5ever::ns!(xlink) && attr.name.local.as_ref() == "href"
        })
    };
    let attr = svg.or_else(xlink)?;
    (attr.value.utf16_len_bound() <= 4096).then(|| attr.value.as_str_lossy())
}

/// The fragment of a marker property's `url(#id)`.
fn marker_fragment(value: &str) -> Option<String> {
    if value.len() > 4096 {
        return None;
    }
    let svgtypes::FuncIRI(id) = svgtypes::FuncIRI::from_str(value).ok()?;
    fragment(id)
}

fn fragment(id: &str) -> Option<String> {
    let id = percent_decode(id);
    (!id.is_empty()).then_some(id)
}

/// IDs in URL fragments are percent-encoded; element IDs are not.
fn percent_decode(id: &str) -> String {
    percent_encoding::percent_decode_str(id)
        .decode_utf8_lossy()
        .into_owned()
}

/// Strongly connected components of `edges` by Tarjan's algorithm, iterative
/// so that page-made chains cannot exhaust the stack. Returns each vertex's
/// component, and the components in Tarjan's order: each after every
/// component it reaches.
fn components(edges: &[Vec<usize>]) -> (Vec<usize>, Vec<Vec<usize>>) {
    const UNSEEN: usize = usize::MAX;
    let count = edges.len();
    let mut index = vec![UNSEEN; count];
    let mut low = vec![0; count];
    let mut on_stack = vec![false; count];
    let mut stack = Vec::new();
    let mut component = vec![UNSEEN; count];
    let mut order: Vec<Vec<usize>> = Vec::new();
    let mut next = 0;
    for start in 0..count {
        if index[start] != UNSEEN {
            continue;
        }
        let mut calls = vec![(start, 0)];
        index[start] = next;
        low[start] = next;
        next += 1;
        stack.push(start);
        on_stack[start] = true;
        while let Some((vertex, edge)) = calls.last_mut() {
            let vertex = *vertex;
            if let Some(&target) = edges[vertex].get(*edge) {
                *edge += 1;
                if index[target] == UNSEEN {
                    index[target] = next;
                    low[target] = next;
                    next += 1;
                    stack.push(target);
                    on_stack[target] = true;
                    calls.push((target, 0));
                } else if on_stack[target] {
                    low[vertex] = low[vertex].min(index[target]);
                }
                continue;
            }
            calls.pop();
            if let Some((parent, _)) = calls.last() {
                low[*parent] = low[*parent].min(low[vertex]);
            }
            if low[vertex] == index[vertex] {
                let mut members = Vec::new();
                loop {
                    let member = stack.pop().expect("a root's component is on the stack");
                    on_stack[member] = false;
                    component[member] = order.len();
                    members.push(member);
                    if member == vertex {
                        break;
                    }
                }
                order.push(members);
            }
        }
    }
    (component, order)
}

#[cfg(test)]
mod tests {
    use super::components;

    #[test]
    fn components_follow_what_they_reach() {
        // 0 → 1 ⇄ 2 → 3, and 4 alone with a loop.
        let edges = vec![vec![1], vec![2], vec![1, 3], vec![], vec![4]];
        let (component, order) = components(&edges);
        assert_eq!(component[1], component[2]);
        assert_ne!(component[0], component[1]);
        assert!(component[3] < component[1] && component[1] < component[0]);
        assert_eq!(order.iter().map(Vec::len).sum::<usize>(), 5);
        assert_eq!(order[component[4]], vec![4]);
    }

    #[test]
    fn long_chains_do_not_recurse() {
        let edges: Vec<Vec<usize>> = (0..100_000)
            .map(|vertex| vec![vertex + 1])
            .chain([vec![]])
            .collect();
        let (component, order) = components(&edges);
        assert_eq!(order.len(), 100_001);
        assert!(component[100_000] < component[0]);
    }
}
