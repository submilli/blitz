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

use super::elements::{
    HIDDEN_CONTAINERS, attribute_reference, bounded_numbers, fragment, href, included, markable,
    middle_vertices, takes_paint, unmarked_shape,
};
use super::lookup::local_urls;
use super::scc::components;
use super::{MAX_ATTRIBUTE_LEN, MAX_ELEMENT_NODES, MAX_EXPANDED_WEIGHT, WEIGHT_BYTES};
use crate::canvas_svg_filter::CanvasSvgLimit;
use crate::node::ElementData;
use crate::{BaseDocument, NodeId};
use std::collections::{HashMap, HashSet};
use style::properties::ComputedValues;
use style::values::computed::url::ComputedUrl;
use style::values::generics::svg::SVGPaintKind;
use style::values::generics::url::UrlOrNone;

/// Distinct IDs one reference may look up.
const MAX_ID_LOOKUPS: usize = 256;
/// Elements nested in the subtree and through its references: the parser
/// recurses once per level.
const MAX_NESTING: usize = 128;
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

    /// The slot's place in [`Slot::ATTRIBUTES`].
    fn attribute_index(slot: Slot) -> Option<usize> {
        Self::ATTRIBUTES
            .iter()
            .position(|(_, other)| *other == slot)
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

/// The attribute references, in `Slot::ATTRIBUTES` order, an element without
/// styles passes to its children.
type AttributeReferences = [Option<usize>; Slot::ATTRIBUTES.len()];
const NO_REFERENCES: AttributeReferences = [None; Slot::ATTRIBUTES.len()];

/// The element whose attribute references are being passed on.
struct Passing<'a> {
    id: NodeId,
    tag: &'a str,
    element: &'a ElementData,
    depth: usize,
    /// The source it instances, for a `<use>`.
    instance: Option<usize>,
}

/// A reference found while walking one source.
struct Found {
    element: NodeId,
    slot: Slot,
    target: usize,
    /// How many times the parser builds the target for this reference.
    builds: usize,
    /// For a paint or marker reference on a `<use>`, the instanced source,
    /// whose content inherits it: it is built for each of that content's
    /// painted elements or marked vertices instead.
    through: Option<usize>,
    /// Whether the target is built per use (a pattern or marker).
    multiplies: bool,
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
    /// Elements taking paint, which a paint inherited from a `<use>` is
    /// built for.
    painted_elements: usize,
    /// Elements taking markers, which an inherited start or end marker is
    /// built for.
    marked_elements: usize,
    /// Their middle vertices, which an inherited middle marker is built for.
    middle_vertices: usize,
}

struct Builder<'a> {
    doc: &'a BaseDocument,
    /// The URLs, without fragments, a local `url(#id)` resolves against.
    local: HashSet<String>,
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
            painted_elements: 0,
            marked_elements: 0,
            middle_vertices: 0,
        };
        // A resource never paints where it stands; an instanced symbol does.
        // Elements without styles inherit references from their ancestors'
        // attributes, as the parser resolves them (`Slot::ATTRIBUTES` order).
        let mut pending = vec![(root, kind == Kind::Resource, 1, NO_REFERENCES)];
        while let Some((id, hidden, depth, inherited)) = pending.pop() {
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
            if !bounded_numbers(tag, element, style.is_none()) {
                return Err(CanvasSvgLimit);
            }
            walked.weight = walked.weight.saturating_add(self.weight(id));
            walked.depth = walked.depth.max(depth);
            walked.self_dependent |=
                !hidden && super::viewport::self_dependent(tag, element, style);
            if takes_paint(tag) {
                walked.painted_elements += 1;
            }
            if markable(tag) {
                walked.marked_elements += 1;
                walked.middle_vertices = walked
                    .middle_vertices
                    .saturating_add(middle_vertices(tag, element));
            }
            // A `<use>`'s target comes first: its other references are
            // inherited by the target's content.
            let mut instance = None;
            let mut own = NO_REFERENCES;
            for (slot, id_ref, target_kind) in self.references(tag, element, style) {
                let Some(target) = self.target(&id_ref, target_kind)? else {
                    continue;
                };
                if target_kind == Kind::Instance {
                    instance = Some(target);
                }
                if let Some(index) = Slot::attribute_index(slot) {
                    own[index] = Some(target);
                }
                walked.found.push(Found {
                    element: id,
                    slot,
                    target,
                    builds: self.builds(tag, element, slot, target),
                    through: instance.filter(|_| slot != Slot::Href),
                    multiplies: self.multiplies(target),
                    depth,
                    visible_instance: !hidden && target_kind == Kind::Instance,
                });
            }
            let inherited = if style.is_some() {
                NO_REFERENCES
            } else {
                let passing = Passing {
                    id,
                    tag,
                    element,
                    depth,
                    instance,
                };
                self.inherit_attributes(&mut walked, &passing, own, inherited)
            };
            pending.extend(
                node.children
                    .iter()
                    .rev()
                    .map(|child| (*child, hidden, depth + 1, inherited)),
            );
        }
        Ok(walked)
    }

    /// The attribute references an element without styles passes to its
    /// children: its own, else its ancestors'. Those it takes from ancestors
    /// are built for it too, as the parser inherits them.
    fn inherit_attributes(
        &self,
        walked: &mut Walked,
        passing: &Passing<'_>,
        own: AttributeReferences,
        inherited: AttributeReferences,
    ) -> AttributeReferences {
        let mut passed = inherited;
        for (index, (_, slot)) in Slot::ATTRIBUTES.iter().enumerate() {
            if own[index].is_some() {
                passed[index] = own[index];
                continue;
            }
            let Some(target) = inherited[index] else {
                continue;
            };
            let builds = self.builds(passing.tag, passing.element, *slot, target);
            // A `<use>` passes it on to its content, charged later.
            if builds > 0 || passing.instance.is_some() {
                walked.found.push(Found {
                    element: passing.id,
                    slot: *slot,
                    target,
                    builds,
                    through: passing.instance,
                    multiplies: self.multiplies(target),
                    depth: passing.depth,
                    visible_instance: false,
                });
            }
        }
        passed
    }

    /// One for the element, one per `WEIGHT_BYTES` of its attributes, which
    /// the parser copies into every build, and one per two bytes of its text,
    /// whose glyphs it flattens.
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
        // The parser shapes and flattens every character of text content;
        // other text is whitespace it skips.
        let text_content = matches!(
            node.element_data()
                .map(|element| element.name.local.as_ref()),
            Some("text" | "tspan" | "textPath")
        );
        let text_weight = if text_content {
            text / 2
        } else {
            text / WEIGHT_BYTES
        };
        1 + attributes / WEIGHT_BYTES + text_weight
    }

    /// How many times the parser builds `target` for one reference: once per
    /// `<use>`, once per element a pattern paints, once for a start or end
    /// marker and once per middle vertex (bounded by the path's numbers).
    /// Gradients and text paths add none: they are built once where defined.
    fn builds(&self, tag: &str, element: &ElementData, slot: Slot, target: usize) -> usize {
        if self.sources[target].kind == Kind::Instance {
            return 1;
        }
        match (self.target_tag(target), slot) {
            ("pattern", _) => usize::from(takes_paint(tag)),
            ("marker", Slot::MarkerMid) => middle_vertices(tag, element),
            ("marker", Slot::MarkerStart | Slot::MarkerEnd) => usize::from(markable(tag)),
            _ => 0,
        }
    }

    /// Whether `target` is a pattern or marker, built per element or vertex.
    fn multiplies(&self, target: usize) -> bool {
        matches!(self.target_tag(target), "pattern" | "marker")
    }

    fn target_tag(&self, target: usize) -> &str {
        let node = self.sources[target].node;
        self.doc.nodes[node]
            .element_data()
            .map_or("", |target| target.name.local.as_ref())
    }

    /// The elements `element` names, with how each is used: from computed
    /// paint and marker properties, or from its attributes when it has no
    /// styles.
    fn references(
        &self,
        tag: &str,
        element: &ElementData,
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
        // Blink draws markers on paths, lines, polylines and polygons only;
        // other shapes are written without them, and containers pass them on.
        let marks = !unmarked_shape(tag);
        for (slot, id) in named {
            if let Some(id) = id
                && (marks || !slot.is_marker())
            {
                found.push((slot, id, Kind::Resource));
            }
        }
        found
    }

    /// The element ID a computed `url()` names, if it is in this document.
    /// Computed URLs are absolute, so a local `url(#id)` is recognized by
    /// the URL it was resolved against.
    fn local_fragment(&self, url: &url::Url) -> Option<String> {
        let document = &url[..url::Position::AfterQuery];
        let local = document.len() <= MAX_ATTRIBUTE_LEN && self.local.contains(document);
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
    fn finish(mut self) -> Result<Plan, CanvasSvgLimit> {
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
        self.inherit_through_uses(&order, &kept);
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

    /// Charge paint and marker references on `<use>`s for what the
    /// instanced content builds from them: each painted element, marked
    /// element or middle vertex, its nested instances' included. Targets
    /// come first in `order`.
    fn inherit_through_uses(&mut self, order: &[usize], kept: &impl Fn(&Found) -> bool) {
        let count = self.sources.len();
        let mut painted = vec![0usize; count];
        let mut marked = vec![0usize; count];
        let mut middles = vec![0usize; count];
        for &source in order {
            let walked = &self.walked[source];
            let mut totals = [
                walked.painted_elements,
                walked.marked_elements,
                walked.middle_vertices,
            ];
            for found in walked.found.iter().filter(|found| kept(found)) {
                if found.slot == Slot::Href && self.sources[found.target].kind == Kind::Instance {
                    let target = found.target;
                    let nested = [painted[target], marked[target], middles[target]];
                    for (total, nested) in totals.iter_mut().zip(nested) {
                        *total = total.saturating_add(nested);
                    }
                }
            }
            [painted[source], marked[source], middles[source]] = totals;
        }
        for walked in &mut self.walked {
            for found in walked.found.iter_mut().filter(|found| found.multiplies) {
                let Some(instance) = found.through else {
                    continue;
                };
                let inherited = match found.slot {
                    Slot::Fill | Slot::Stroke => painted[instance],
                    Slot::MarkerMid => middles[instance],
                    Slot::MarkerStart | Slot::MarkerEnd => marked[instance],
                    Slot::Href => 0,
                };
                found.builds = found.builds.max(inherited);
            }
        }
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
                .map(|found| {
                    let through = found.through.map_or(0, |instance| nesting[instance]);
                    found
                        .depth
                        .saturating_add(through)
                        .saturating_add(nesting[found.target])
                })
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
