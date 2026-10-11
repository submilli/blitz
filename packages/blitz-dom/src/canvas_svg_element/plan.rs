//! What one element reference serializes: the subtree, the copies its
//! references need, and which references cannot be kept.
//!
//! A reference whose target's expansion leads back to an element containing
//! it is cyclic, and is left out as Blink leaves out a `<use>` that would
//! instantiate itself. The rest form a graph without cycles, so the size of
//! every `<use>` expansion is known before the parser builds it.

use super::elements::{href, included};
use super::scc::components;
use super::{
    HIDDEN_CONTAINERS, MAX_ATTRIBUTE_LEN, MAX_ELEMENT_NODES, MAX_EXPANDED_WEIGHT, WEIGHT_BYTES,
};
use crate::canvas_svg_filter::CanvasSvgLimit;
use crate::{BaseDocument, NodeId};
use std::collections::HashMap;
use style::properties::ComputedValues;
use style::values::computed::url::ComputedUrl;
use style::values::generics::svg::SVGPaintKind;
use style::values::generics::url::UrlOrNone;

/// Distinct IDs one reference may look up, each a bounded document scan.
const MAX_ID_LOOKUPS: usize = 256;
/// `<use>`s nested in one instance chain: the parser recurses once per level.
const MAX_USE_DEPTH: usize = 64;

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
    /// The marker properties, as attributes and style names.
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
        let local = [
            doc.document_url(root),
            doc.document_base_url(root),
            doc.nodes[root].svg_presentation_base(),
        ];
        let mut builder = Builder {
            doc,
            local,
            ids: HashMap::new(),
            sources: vec![Source {
                node: root,
                kind: Kind::Root,
            }],
            index: HashMap::new(),
            owned: Vec::new(),
            weights: Vec::new(),
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
    /// The URLs a local `url(#id)` resolves against: the document's, its
    /// base, and the one presentation attributes use.
    local: [url::Url; 3],
    /// Elements by ID in the root's tree, looked up on first use.
    ids: HashMap<String, Option<NodeId>>,
    sources: Vec<Source>,
    index: HashMap<(NodeId, Kind), usize>,
    /// Per source, as walked.
    owned: Vec<Vec<Owned>>,
    /// Per source, its elements and their content (see `MAX_EXPANDED_WEIGHT`).
    weights: Vec<usize>,
    self_dependent: Vec<bool>,
    /// Elements walked across all sources.
    visited: usize,
}

impl Builder<'_> {
    /// Walk source `index`, recording its references and copies to make.
    fn walk(&mut self, index: usize) -> Result<(), CanvasSvgLimit> {
        let Source { node: root, kind } = self.sources[index];
        let mut owned = Vec::new();
        let mut weight = 0usize;
        let mut dependent = false;
        // A resource never paints where it stands; an instanced symbol does.
        let mut pending = vec![(root, kind == Kind::Resource)];
        while let Some((id, hidden)) = pending.pop() {
            let Some(tag) = included(self.doc, id) else {
                continue;
            };
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
            let style = node.primary_styles();
            let style = style.as_deref().map(|style| &**style);
            if let Some(style) = style {
                bound_dashes(style)?;
            }
            weight = weight.saturating_add(self.weight(id));
            dependent |= !hidden && super::viewport::self_dependent(tag, element, style);
            for (slot, id_ref, target_kind) in self.references(tag, element, style) {
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
        self.weights.push(weight);
        self.self_dependent.push(dependent);
        Ok(())
    }

    /// One for the element, and one per `WEIGHT_BYTES` of its attributes and
    /// text, which the parser copies into every instance.
    fn weight(&self, id: NodeId) -> usize {
        let node = &self.doc.nodes[id];
        let attributes = node.element_data().map_or(0, |element| {
            element
                .attrs()
                .iter()
                .map(|attr| attr.value.utf16_len_bound())
                .sum()
        });
        let text: usize = node
            .children
            .iter()
            .filter_map(|child| self.doc.nodes[*child].text_data())
            .map(|text| text.content.utf16_len_bound())
            .sum();
        1 + attributes.saturating_add(text) / WEIGHT_BYTES
    }

    /// The elements `element` names, with how each is used: from computed
    /// paint and marker properties, or from its attributes when it has no
    /// styles.
    fn references(
        &self,
        tag: &str,
        element: &crate::node::ElementData,
        style: Option<&ComputedValues>,
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
        let named: Vec<(Slot, Option<String>)> = match style {
            Some(style) => vec![
                (Slot::Fill, paint_server(&style.clone_fill().kind)),
                (Slot::Stroke, paint_server(&style.clone_stroke().kind)),
                (Slot::MarkerStart, marker(&style.clone_marker_start())),
                (Slot::MarkerMid, marker(&style.clone_marker_mid())),
                (Slot::MarkerEnd, marker(&style.clone_marker_end())),
            ]
            .into_iter()
            .map(|(slot, url)| (slot, url.and_then(|url| self.local_fragment(&url))))
            .collect(),
            None => [("fill", Slot::Fill), ("stroke", Slot::Stroke)]
                .into_iter()
                .chain(Slot::MARKERS)
                .map(|(name, slot)| {
                    let value = element.attr(markup5ever::LocalName::from(name));
                    (slot, value.and_then(attribute_fragment))
                })
                .collect(),
        };
        for (slot, id) in named {
            if let Some(id) = id {
                found.push((slot, id, Kind::Resource));
            }
        }
        found
    }

    /// The element ID a computed `url()` names, if it is in this document.
    /// Computed URLs are absolute, so a local `url(#id)` is recognized by
    /// the URL it was resolved against.
    fn local_fragment(&self, url: &url::Url) -> Option<String> {
        let local = self
            .local
            .iter()
            .any(|local| url[..url::Position::AfterQuery] == local[..url::Position::AfterQuery]);
        local.then(|| fragment(url.fragment()?)).flatten()
    }

    /// The copy of the element `id` names, made when first needed. A target
    /// that is not an SVG element entering markup is no reference.
    fn target(&mut self, id: &str, kind: Kind) -> Result<Option<usize>, CanvasSvgLimit> {
        let node = match self.ids.get(id) {
            Some(node) => *node,
            None => {
                if self.ids.len() >= MAX_ID_LOOKUPS {
                    return Err(CanvasSvgLimit);
                }
                let node = self.doc.find_svg_reference(self.sources[0].node, id)?;
                self.ids.insert(id.to_owned(), node);
                node
            }
        };
        let Some(node) = node.filter(|node| included(self.doc, *node).is_some()) else {
            return Ok(None);
        };
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
        let cyclic = self.cyclic_references();
        let kept = |owned: &Owned| cyclic.get(&(owned.element, owned.slot)) == Some(&false);
        let reached = self.reached(&kept);
        let (expanded, depth, dependent) = self.expansion_and_dependence(&kept);
        if depth > MAX_USE_DEPTH {
            return Err(CanvasSvgLimit);
        }
        // The parser expands every copy it is given, wherever it stands.
        let total = expanded
            .iter()
            .zip(&reached)
            .filter(|(_, reached)| **reached)
            .fold(0usize, |total, (expanded, _)| {
                total.saturating_add(*expanded)
            });
        if total > MAX_EXPANDED_WEIGHT {
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

    /// Whether each reference is cyclic. The same element is walked once per
    /// source containing it; any of those sources reached back from its
    /// target makes it cyclic.
    fn cyclic_references(&self) -> HashMap<(NodeId, Slot), bool> {
        let edges: Vec<Vec<usize>> = self.owned.iter().map(|owned| targets(owned)).collect();
        let (component, _) = components(&edges);
        let mut cyclic: HashMap<(NodeId, Slot), bool> = HashMap::new();
        for (source, owned) in self.owned.iter().enumerate() {
            for owned in owned {
                let back = component[source] == component[owned.target];
                *cyclic.entry((owned.element, owned.slot)).or_default() |= back;
            }
        }
        cyclic
    }

    /// The sources kept references reach from the root: only those are written.
    fn reached(&self, kept: &impl Fn(&Owned) -> bool) -> Vec<bool> {
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
        reached
    }

    /// Each source's weight once its kept `<use>`s expand, the deepest
    /// `<use>` nesting, and whether each source's visible content depends on
    /// the viewport. Without cyclic references the graph has no cycles, and
    /// Tarjan's order puts each component after every component it reaches,
    /// so a source's targets come first.
    fn expansion_and_dependence(
        &self,
        kept: &impl Fn(&Owned) -> bool,
    ) -> (Vec<usize>, usize, Vec<bool>) {
        let kept_edges: Vec<Vec<usize>> = self
            .owned
            .iter()
            .map(|owned| {
                owned
                    .iter()
                    .filter(|owned| kept(owned))
                    .map(|owned| owned.target)
                    .collect()
            })
            .collect();
        let (_, order) = components(&kept_edges);
        let mut expanded = vec![0usize; self.sources.len()];
        let mut depths = vec![0usize; self.sources.len()];
        let mut dependent = self.self_dependent.clone();
        for source in order.into_iter().flatten() {
            let mut weight = self.weights[source];
            for owned in self.owned[source].iter().filter(|owned| kept(owned)) {
                if self.sources[owned.target].kind == Kind::Instance {
                    weight = weight.saturating_add(expanded[owned.target]);
                    depths[source] = depths[source].max(depths[owned.target] + 1);
                }
                dependent[source] |= owned.visible_instance && dependent[owned.target];
            }
            expanded[source] = weight;
        }
        let depth = depths.into_iter().max().unwrap_or(0);
        (expanded, depth, dependent)
    }
}

fn targets(owned: &[Owned]) -> Vec<usize> {
    owned.iter().map(|owned| owned.target).collect()
}

/// Bound a computed `stroke-dasharray`, which stylesheets do not limit: every
/// element walked compares and may serialize it.
fn bound_dashes(style: &ComputedValues) -> Result<(), CanvasSvgLimit> {
    use style::values::generics::svg::SVGStrokeDashArray;
    const MAX_DASHES: usize = 1024;
    match &style.get_inherited_svg().stroke_dasharray {
        SVGStrokeDashArray::Values(values) if values.len() > MAX_DASHES => Err(CanvasSvgLimit),
        _ => Ok(()),
    }
}

fn paint_server(
    kind: &SVGPaintKind<style::values::computed::Color, ComputedUrl>,
) -> Option<url::Url> {
    match kind {
        SVGPaintKind::PaintServer(url) => url.url().map(|url| (**url).clone()),
        _ => None,
    }
}

fn marker(value: &UrlOrNone<ComputedUrl>) -> Option<url::Url> {
    match value {
        UrlOrNone::Url(url) => url.url().map(|url| (**url).clone()),
        UrlOrNone::None => None,
    }
}

/// The fragment of a `url(#id)` attribute (a paint, with any fallback, or a
/// marker) of an element without styles.
fn attribute_fragment(value: &str) -> Option<String> {
    if value.len() > MAX_ATTRIBUTE_LEN {
        return None;
    }
    let svgtypes::FuncIRI(id) = svgtypes::FuncIRI::from_str(value.trim_start()).ok()?;
    fragment(id)
}

/// An ID from a URL fragment, where IDs are percent-encoded.
fn fragment(id: &str) -> Option<String> {
    if id.is_empty() || id.len() > MAX_ATTRIBUTE_LEN {
        return None;
    }
    Some(
        percent_encoding::percent_decode_str(id)
            .decode_utf8_lossy()
            .into_owned(),
    )
}
