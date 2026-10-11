//! How many times the parser builds each copy: per `<use>`, per element a
//! pattern paints (per character of text), per marker build and vertex, and
//! for references a `<use>` passes to its content.

use super::{Builder, Found, Kind, Slot};
use crate::NodeId;
use crate::node::ElementData;
use std::collections::HashMap;
use style::properties::ComputedValues;
use style::values::generics::svg::SVGPaintKind;

use super::super::elements::{markable, middle_vertices, takes_paint};

impl Builder<'_> {
    /// How many times the parser builds `target` for one reference: once per
    /// `<use>`, once per paint of the element a pattern paints (see
    /// [`Self::paints`]) and once per build of a pattern using it as its
    /// template, once for a start or end marker and once per middle vertex
    /// (bounded by the path's numbers). Gradients and text paths add none:
    /// they are built once where defined.
    pub(super) fn builds(
        &self,
        id: NodeId,
        tag: &str,
        element: &ElementData,
        slot: Slot,
        target: usize,
    ) -> usize {
        if self.sources[target].kind == Kind::Instance {
            return 1;
        }
        match (self.target_tag(target), slot) {
            // A pattern's template is built once per build of the pattern.
            ("pattern", Slot::Href) => 1,
            ("pattern", _) => self.paints(id, tag),
            ("marker", Slot::MarkerMid) => middle_vertices(tag, element),
            ("marker", Slot::MarkerStart | Slot::MarkerEnd) => usize::from(markable(tag)),
            _ => 0,
        }
    }

    /// How many times the parser paints `tag` with a paint server: once per
    /// shape, and for text up to three times per character of its own text
    /// (per chunk, span and outline the parser fills).
    pub(super) fn paints(&self, id: NodeId, tag: &str) -> usize {
        if self.text_content(id, tag) {
            let characters: usize = self.doc.nodes[id]
                .children
                .iter()
                .filter_map(|child| self.doc.nodes[*child].text_data())
                .map(|text| text.content.utf16_len_bound())
                .sum();
            return characters.saturating_add(1).saturating_mul(3);
        }
        usize::from(takes_paint(tag))
    }

    /// Whether `target` is a pattern or marker, built per element or vertex.
    pub(super) fn multiplies(&self, target: usize) -> bool {
        matches!(self.target_tag(target), "pattern" | "marker")
    }

    fn target_tag(&self, target: usize) -> &str {
        let node = self.sources[target].node;
        self.doc.nodes[node]
            .element_data()
            .map_or("", |target| target.name.local.as_ref())
    }

    /// Whether the parser shapes `id`'s text: text, its spans and paths, and
    /// links within text (elsewhere a link is a group).
    pub(super) fn text_content(&self, id: NodeId, tag: &str) -> bool {
        const TEXT: [&str; 3] = ["text", "tspan", "textPath"];
        if TEXT.contains(&tag) {
            return true;
        }
        if tag != "a" {
            return false;
        }
        let mut current = self.doc.nodes[id].parent;
        // Text and its spans are shallow; a deeper link is not in text.
        for _ in 0..64 {
            let Some(parent) = current else {
                return false;
            };
            let Some(element) = self.doc.nodes[parent].element_data() else {
                return false;
            };
            if element.name.ns != markup5ever::ns!(svg) {
                return false;
            }
            if TEXT.contains(&element.name.local.as_ref()) {
                return true;
            }
            current = self.doc.nodes[parent].parent;
        }
        false
    }

    /// Charge paint and marker references on `<use>`s for what the
    /// instanced content builds from them: each painted element, marked
    /// element or middle vertex, its nested instances' included. Targets
    /// come first in `order`.
    pub(super) fn inherit_through_uses(&mut self, order: &[usize], kept: &impl Fn(&Found) -> bool) {
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

    /// Charge a marked element's pattern for the marker content that paints
    /// with it (`context-fill`, `context-stroke`): once per marker build and
    /// context-painted element.
    pub(super) fn charge_context_paints(&mut self, kept: &impl Fn(&Found) -> bool) {
        let context: Vec<usize> = self
            .walked
            .iter()
            .map(|walked| walked.context_painted)
            .collect();
        for walked in &mut self.walked {
            let mut extra: HashMap<NodeId, usize> = HashMap::new();
            for found in walked
                .found
                .iter()
                .filter(|found| kept(found) && found.slot.is_marker())
            {
                let builds = found.builds.saturating_mul(context[found.target]);
                let total = extra.entry(found.element).or_default();
                *total = total.saturating_add(builds);
            }
            for found in &mut walked.found {
                let paint = matches!(found.slot, Slot::Fill | Slot::Stroke) && found.multiplies;
                if let Some(extra) = extra.get(&found.element).filter(|_| paint) {
                    found.builds = found.builds.saturating_add(*extra);
                }
            }
        }
    }

    /// How many times the parser builds each source: the root once, every
    /// other source reached once where it is defined, plus its builds through
    /// each kept reference. Sources come before their targets in reverse
    /// `order`.
    pub(super) fn builds_of_sources(
        &self,
        order: &[usize],
        kept: &impl Fn(&Found) -> bool,
    ) -> Vec<usize> {
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
}

/// Whether an element paints with its context element's fill or stroke.
pub(super) fn context_painted(element: &ElementData, style: Option<&ComputedValues>) -> bool {
    match style {
        Some(style) => {
            let svg = style.get_inherited_svg();
            [&svg.fill.kind, &svg.stroke.kind].iter().any(|kind| {
                matches!(
                    kind,
                    SVGPaintKind::ContextFill | SVGPaintKind::ContextStroke
                )
            })
        }
        None => ["fill", "stroke"].iter().any(|name| {
            element
                .attr(markup5ever::LocalName::from(*name))
                .is_some_and(|value| value.trim_start().starts_with("context-"))
        }),
    }
}
