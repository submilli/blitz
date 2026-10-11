//! What one element reference serializes: the subtree, the copies its
//! references need, which references cannot be kept, and how many times the
//! parser builds each copy.
//!
//! A reference whose target's expansion leads back to an element containing
//! it is cyclic, and is left out as Blink leaves out a `<use>` that would
//! instantiate itself. The rest form a graph without cycles, so every
//! expansion is known before the parser builds it: a `<use>` builds its
//! target again, a pattern is built again for each element it paints, and a
//! marker for each vertex of the path it marks.

use super::elements::{HIDDEN_CONTAINERS, attribute_reference, fragment, href, included};
use super::scc::components;
use super::{MAX_ELEMENT_NODES, MAX_EXPANDED_WEIGHT, WEIGHT_BYTES};
use crate::canvas_svg_filter::CanvasSvgLimit;
use crate::{BaseDocument, NodeId};
use std::collections::HashMap;
use style::properties::ComputedValues;
use style::values::computed::url::ComputedUrl;
use style::values::generics::svg::SVGPaintKind;
use style::values::generics::url::UrlOrNone;

/// Distinct IDs one reference may look up.
const MAX_ID_LOOKUPS: usize = 256;
/// Elements nested in the subtree and through its references: the parser
/// recurses once per level.
const MAX_NESTING: usize = 128;
/// Stylesheets whose URLs a local `url(#id)` may have resolved against.
const MAX_STYLESHEET_URLS: usize = 256;
/// Entries of a computed `stroke-dasharray`, which stylesheets do not limit.
const MAX_DASHES: usize = 1024;

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
    /// The presentation attributes that name elements, by slot.
    pub(super) const ATTRIBUTES: [(&str, Slot); 5] = [
        ("fill", Slot::Fill),
        ("stroke", Slot::Stroke),
        ("marker-start", Slot::MarkerStart),
        ("marker-mid", Slot::MarkerMid),
        ("marker-end", Slot::MarkerEnd),
    ];

    /// The slot of a presentation attribute that names elements.
    pub(super) fn of_attribute(name: &str) -> Option<Slot> {
        Self::ATTRIBUTES
            .iter()
            .find(|(attribute, _)| *attribute == name)
            .map(|(_, slot)| *slot)
    }

    fn is_marker(self) -> bool {
        matches!(self, Slot::MarkerStart | Slot::MarkerMid | Slot::MarkerEnd)
    }
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

/// The subtrees to serialize and the references kept between them.
pub(super) struct Plan {
    /// The referenced element first, then the copies its kept references
    /// reach.
    pub sources: Vec<Source>,
    /// How many times the parser builds each source.
    pub builds: Vec<usize>,
    references: HashMap<(NodeId, Slot), usize>,
    /// Whether Blink would map the viewport onto the subregion
    /// (`HasVisibleViewportDependence` in `svg_fe_image.cc`).
    pub viewport_dependent: bool,
}

impl Plan {
    pub(super) fn new(doc: &BaseDocument, root: NodeId) -> Result<Self, CanvasSvgLimit> {
        let mut builder = Builder {
            doc,
            local: local_urls(doc, root),
            ids: HashMap::new(),
            sources: vec![Source {
                node: root,
                kind: Kind::Root,
            }],
            index: HashMap::new(),
            walked: Vec::new(),
            visited: 0,
        };
        let mut next = 0;
        while next < builder.sources.len() {
            let walked = builder.walk(next)?;
            builder.walked.push(walked);
            next += 1;
        }
        builder.finish()
    }

    /// The copy `element` names at `slot`, unless it is missing or cyclic.
    pub(super) fn reference(&self, element: NodeId, slot: Slot) -> Option<usize> {
        self.references.get(&(element, slot)).copied()
    }
}

/// A reference found while walking one source.
struct Found {
    element: NodeId,
    slot: Slot,
    target: usize,
    /// How many times the parser builds the target for this reference.
    builds: usize,
    /// The element's depth in its source.
    depth: usize,
    /// A `<use>` that Blink lays out, whose content's lengths count.
    visible_instance: bool,
}

/// What walking one source found.
struct Walked {
    found: Vec<Found>,
    /// Its elements and their content (see `MAX_EXPANDED_WEIGHT`).
    weight: usize,
    /// Its deepest element.
    depth: usize,
    self_dependent: bool,
}

struct Builder<'a> {
    doc: &'a BaseDocument,
    /// The URLs a local `url(#id)` resolves against.
    local: Vec<url::Url>,
    /// Elements by ID in the root's tree, looked up on first use.
    ids: HashMap<String, Option<NodeId>>,
    sources: Vec<Source>,
    index: HashMap<(NodeId, Kind), usize>,
    /// Per source, as walked.
    walked: Vec<Walked>,
    /// Elements walked across all sources.
    visited: usize,
}

impl Builder<'_> {
    /// Walk source `index`, recording its references and copies to make.
    fn walk(&mut self, index: usize) -> Result<Walked, CanvasSvgLimit> {
        let Source { node: root, kind } = self.sources[index];
        let mut walked = Walked {
            found: Vec::new(),
            weight: 0,
            depth: 0,
            self_dependent: false,
        };
        // A resource never paints where it stands; an instanced symbol does.
        let mut pending = vec![(root, kind == Kind::Resource, 1)];
        while let Some((id, hidden, depth)) = pending.pop() {
            let Some(tag) = included(self.doc, id) else {
                continue;
            };
            self.visited += 1;
            let node = &self.doc.nodes[id];
            // The pending stack also holds text and skipped nodes.
            if self.visited > MAX_ELEMENT_NODES
                || pending.len() + node.children.len() > MAX_ELEMENT_NODES * 2
            {
                return Err(CanvasSvgLimit);
            }
            let element = node.element_data().expect("included nodes are elements");
            let style = node.primary_styles();
            let style = style.as_deref().map(|style| &**style);
            let instanced_symbol = id == root && kind == Kind::Instance;
            // Blink lays out nothing below `display: none`; unstyled elements
            // are there, and render only as a `<use>`'s instance.
            let undisplayed = match style {
                Some(style) => style.clone_display().is_none(),
                None => kind != Kind::Instance,
            };
            let hidden =
                hidden || undisplayed || (HIDDEN_CONTAINERS.contains(&tag) && !instanced_symbol);
            if let Some(style) = style {
                bound_dashes(style)?;
            }
            walked.weight = walked.weight.saturating_add(self.weight(id));
            walked.depth = walked.depth.max(depth);
            walked.self_dependent |=
                !hidden && super::viewport::self_dependent(tag, element, style);
            for (slot, id_ref, target_kind) in self.references(tag, element, style) {
                let Some(target) = self.target(&id_ref, target_kind)? else {
                    continue;
                };
                walked.found.push(Found {
                    element: id,
                    slot,
                    target,
                    builds: self.builds(tag, element, slot, target),
                    depth,
                    visible_instance: !hidden && target_kind == Kind::Instance,
                });
            }
            pending.extend(
                node.children
                    .iter()
                    .rev()
                    .map(|child| (*child, hidden, depth + 1)),
            );
        }
        Ok(walked)
    }

    /// One for the element, and one per `WEIGHT_BYTES` of its attributes and
    /// text, which the parser copies into every build.
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

    /// How many times the parser builds `target` for one reference: once per
    /// `<use>`, once per element a pattern paints, once per vertex a marker
    /// marks (bounded by the path's text). Gradients and text paths are
    /// built once wherever they are used.
    fn builds(
        &self,
        tag: &str,
        element: &crate::node::ElementData,
        slot: Slot,
        target: usize,
    ) -> usize {
        let Source { node, kind } = self.sources[target];
        if kind == Kind::Instance {
            return 1;
        }
        let target_tag = self.doc.nodes[node]
            .element_data()
            .map_or("", |target| target.name.local.as_ref());
        match target_tag {
            "pattern" => 1,
            "marker" if slot.is_marker() => vertices(tag, element),
            _ => 0,
        }
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
            Some(style) => {
                let svg = style.get_inherited_svg();
                let urls = [
                    (Slot::Fill, paint_server(&svg.fill.kind)),
                    (Slot::Stroke, paint_server(&svg.stroke.kind)),
                    (Slot::MarkerStart, marker(&svg.marker_start)),
                    (Slot::MarkerMid, marker(&svg.marker_mid)),
                    (Slot::MarkerEnd, marker(&svg.marker_end)),
                ];
                urls.into_iter()
                    .map(|(slot, url)| (slot, url.and_then(|url| self.local_fragment(url))))
                    .collect()
            }
            None => Slot::ATTRIBUTES
                .iter()
                .map(|(name, slot)| {
                    let value = element.attr(markup5ever::LocalName::from(*name));
                    (*slot, value.and_then(attribute_reference).map(|(id, _)| id))
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
                let node = self.doc.svg_element_by_id(self.sources[0].node, id)?;
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

    /// Drop cyclic references, then bound the expansion and nesting and
    /// resolve the viewport dependence over the graph left.
    fn finish(self) -> Result<Plan, CanvasSvgLimit> {
        let cyclic = self.cyclic_references();
        let kept = |found: &Found| cyclic.get(&(found.element, found.slot)) == Some(&false);
        let kept_edges: Vec<Vec<usize>> = self
            .walked
            .iter()
            .map(|walked| {
                let kept = walked.found.iter().filter(|found| kept(found));
                kept.map(|found| found.target).collect()
            })
            .collect();
        // Without cyclic references the graph has no cycles, and Tarjan's
        // order puts each component, here one source, after every one it
        // reaches.
        let (_, order) = components(&kept_edges);
        let order: Vec<usize> = order.into_iter().flatten().collect();
        let builds = self.builds_of_sources(&order, &kept);
        let weight = builds
            .iter()
            .zip(&self.walked)
            .fold(0usize, |total, (builds, walked)| {
                total.saturating_add(builds.saturating_mul(walked.weight))
            });
        if weight > MAX_EXPANDED_WEIGHT || self.nesting(&order, &kept) > MAX_NESTING {
            return Err(CanvasSvgLimit);
        }
        let dependent = self.dependence(&order, &kept);
        // Write only what is built, renumbered so copy IDs stay dense.
        let mut renumbered = vec![None; self.sources.len()];
        let mut sources = Vec::new();
        let mut kept_builds = Vec::new();
        for (index, source) in self.sources.into_iter().enumerate() {
            if builds[index] > 0 {
                renumbered[index] = Some(sources.len());
                sources.push(source);
                kept_builds.push(builds[index]);
            }
        }
        let references = self
            .walked
            .iter()
            .flat_map(|walked| &walked.found)
            .filter(|found| kept(found))
            .filter_map(|found| Some(((found.element, found.slot), renumbered[found.target]?)))
            .collect();
        Ok(Plan {
            sources,
            builds: kept_builds,
            references,
            viewport_dependent: dependent[0],
        })
    }

    /// Whether each reference is cyclic. The same element is walked once per
    /// source containing it; any of those sources reached back from its
    /// target makes it cyclic.
    fn cyclic_references(&self) -> HashMap<(NodeId, Slot), bool> {
        let edges: Vec<Vec<usize>> = self
            .walked
            .iter()
            .map(|walked| walked.found.iter().map(|found| found.target).collect())
            .collect();
        let (component, _) = components(&edges);
        let mut cyclic: HashMap<(NodeId, Slot), bool> = HashMap::new();
        for (source, walked) in self.walked.iter().enumerate() {
            for found in &walked.found {
                let back = component[source] == component[found.target];
                *cyclic.entry((found.element, found.slot)).or_default() |= back;
            }
        }
        cyclic
    }

    /// How many times the parser builds each source: the root once, every
    /// other source reached once where it is defined, plus its builds through
    /// each kept reference. Sources come before their targets in reverse
    /// `order`.
    fn builds_of_sources(&self, order: &[usize], kept: &impl Fn(&Found) -> bool) -> Vec<usize> {
        let mut builds = vec![0usize; self.sources.len()];
        builds[0] = 1;
        for &source in order.iter().rev() {
            if builds[source] == 0 {
                continue;
            }
            for found in self.walked[source].found.iter().filter(|found| kept(found)) {
                let through = builds[source].saturating_mul(found.builds);
                let defined = usize::from(builds[found.target] == 0);
                builds[found.target] = builds[found.target]
                    .saturating_add(through)
                    .saturating_add(defined);
            }
        }
        builds
    }

    /// The deepest nesting the parser recurses through: elements within a
    /// source, then on into each kept reference's target.
    fn nesting(&self, order: &[usize], kept: &impl Fn(&Found) -> bool) -> usize {
        let mut nesting = vec![0usize; self.sources.len()];
        for &source in order {
            let walked = &self.walked[source];
            nesting[source] = walked
                .found
                .iter()
                .filter(|found| kept(found))
                .map(|found| found.depth.saturating_add(nesting[found.target]))
                .fold(walked.depth, usize::max);
        }
        nesting.into_iter().max().unwrap_or(0)
    }

    /// Whether each source's visible content depends on the viewport,
    /// through the `<use>`s Blink lays out.
    fn dependence(&self, order: &[usize], kept: &impl Fn(&Found) -> bool) -> Vec<bool> {
        let mut dependent: Vec<bool> = self
            .walked
            .iter()
            .map(|walked| walked.self_dependent)
            .collect();
        for &source in order {
            for found in self.walked[source].found.iter().filter(|found| kept(found)) {
                dependent[source] |= found.visible_instance && dependent[found.target];
            }
        }
        dependent
    }
}

impl BaseDocument {
    /// The first element with ID `id` in `owner`'s tree (its shadow tree, or
    /// its document), through the document's ID index; a scan settles ties.
    fn svg_element_by_id(&self, owner: NodeId, id: &str) -> Result<Option<NodeId>, CanvasSvgLimit> {
        const MAX_CANDIDATES: usize = 16;
        let Some(candidates) = self.nodes_to_id.get(id) else {
            return Ok(None);
        };
        if candidates.len() > MAX_CANDIDATES {
            return self.find_svg_reference(owner, id);
        }
        let tree = self.tree_scope(owner)?;
        let mut found = None;
        for &candidate in candidates {
            if self.tree_scope(candidate)? == tree {
                if found.is_some() {
                    return self.find_svg_reference(owner, id);
                }
                found = Some(candidate);
            }
        }
        Ok(found)
    }

    /// The root of `node`'s tree: its shadow root, else its document.
    fn tree_scope(&self, node: NodeId) -> Result<NodeId, CanvasSvgLimit> {
        const MAX_ANCESTORS: usize = 4096;
        let mut top = node;
        for _ in 0..MAX_ANCESTORS {
            let Some(parent) = self.nodes[top].parent else {
                return Ok(if self.nodes[top].is_shadow_root() {
                    top
                } else {
                    self.node_document(node)
                });
            };
            top = parent;
        }
        Err(CanvasSvgLimit)
    }
}

/// The URLs a local `url(#id)` may be resolved against: the document's, its
/// base, the presentation attributes' and the document's stylesheets'.
fn local_urls(doc: &BaseDocument, root: NodeId) -> Vec<url::Url> {
    let mut urls = vec![
        doc.document_url(root),
        doc.document_base_url(root),
        doc.nodes[root].svg_presentation_base(),
    ];
    let guard = doc.guard.read();
    let sheets = doc.nodes_to_stylesheet.values().take(MAX_STYLESHEET_URLS);
    for sheet in sheets {
        if let Some(contents) = sheet.0.contents.try_read_with(&guard) {
            urls.push((*contents.url_data.0).clone());
        }
    }
    urls
}

/// How many vertices a marker may be drawn at on `element`, bounded by the
/// text of its points.
fn vertices(tag: &str, element: &crate::node::ElementData) -> usize {
    let points = |name: &str| {
        element
            .attr(markup5ever::LocalName::from(name))
            .map_or(0, |value| value.len() / 2 + 1)
    };
    match tag {
        "line" => 2,
        "path" => points("d"),
        "polyline" | "polygon" => points("points"),
        _ => 0,
    }
}

/// Bound a computed `stroke-dasharray`: every element walked compares and
/// may serialize it.
fn bound_dashes(style: &ComputedValues) -> Result<(), CanvasSvgLimit> {
    use style::values::generics::svg::SVGStrokeDashArray;
    match &style.get_inherited_svg().stroke_dasharray {
        SVGStrokeDashArray::Values(values) if values.len() > MAX_DASHES => Err(CanvasSvgLimit),
        _ => Ok(()),
    }
}

fn paint_server(
    kind: &SVGPaintKind<style::values::computed::Color, ComputedUrl>,
) -> Option<&url::Url> {
    match kind {
        SVGPaintKind::PaintServer(url) => url.url().map(|url| &**url),
        _ => None,
    }
}

fn marker(value: &UrlOrNone<ComputedUrl>) -> Option<&url::Url> {
    match value {
        UrlOrNone::Url(url) => url.url().map(|url| &**url),
        UrlOrNone::None => None,
    }
}
