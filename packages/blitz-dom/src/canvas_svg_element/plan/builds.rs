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
    /// (bounded by the path's numbers), and once per text path, which
    /// converts the shape it follows each time it is built. Gradients add
    /// none: they are built once where defined.
    pub(super) fn builds(
        &self,
        tag: &str,
        element: &ElementData,
        slot: Slot,
        target: usize,
        paints: usize,
    ) -> usize {
        if self.sources[target].kind == Kind::Instance {
            return 1;
        }
        match (self.target_tag(target), slot) {
            // A text path converts the shape it follows each time.
            _ if tag == "textPath" && slot == Slot::Href => 1,
            // A pattern's template is built once per build of the pattern.
            ("pattern", Slot::Href) => 1,
            ("pattern", _) => paints,
            ("marker", Slot::MarkerMid) => middle_vertices(tag, element),
            ("marker", Slot::MarkerStart | Slot::MarkerEnd) => usize::from(markable(tag)),
            _ => 0,
        }
    }

    /// How many times the parser paints `tag` with a paint server: once per
    /// shape, and for text up to three times per character of its own text
    /// (per chunk, span and outline the parser fills), twelve when it may be
    /// decorated (each of three decorations adds as many).
    pub(super) fn paints(&self, id: NodeId, tag: &str, text: bool, decorated: bool) -> usize {
        if text {
            let characters: usize = self.doc.nodes[id]
                .children
                .iter()
                .filter_map(|child| self.doc.nodes[*child].text_data())
                .map(|text| text.content.utf16_len_bound())
                .sum();
            let per_character = if decorated { 12 } else { 3 };
            return characters.saturating_add(1).saturating_mul(per_character);
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

    /// Charge paint and marker references for what is built from them out
    /// of sight of the element naming them, in `order` (targets first):
    ///
    /// - a `<use>`'s, once per element, marked element or middle vertex of
    ///   the instanced content, which inherits them;
    /// - a `text-decoration` element's paint, once per decoration painted on
    ///   text in its source.
    ///
    /// A source's context paints come to include its instances', all of an
    /// instance's paints when its `<use>` passes a context paint down, and
    /// those of the markers its context-painted elements pass it on to.
    pub(super) fn inherit_through_uses(&mut self, order: &[usize], kept: &impl Fn(&Found) -> bool) {
        let mut totals = vec![[0usize; 4]; self.sources.len()];
        for &source in order {
            let walked = &self.walked[source];
            let mut total = [
                walked.painted_elements,
                walked.marked_elements,
                walked.middle_vertices,
                walked.decorated_paints,
            ];
            let mut context = walked.context_painted;
            for found in walked.found.iter().filter(|found| kept(found)) {
                if found.slot != Slot::Href || self.sources[found.target].kind != Kind::Instance {
                    continue;
                }
                let nested = &totals[found.target];
                for (total, nested) in total.iter_mut().zip(nested) {
                    *total = total.saturating_add(*nested);
                }
                context = context.saturating_add(self.walked[found.target].context_painted);
                if found.context {
                    context = context.saturating_add(nested[0]);
                }
            }
            totals[source] = total;
            for found in &mut self.walked[source].found {
                if let Some(instance) = found.through.filter(|_| found.multiplies) {
                    let [painted, marked, middles, _] = totals[instance];
                    let inherited = match found.slot {
                        Slot::Fill | Slot::Stroke => painted,
                        Slot::MarkerMid => middles,
                        Slot::MarkerStart | Slot::MarkerEnd => marked,
                        Slot::Href => 0,
                    };
                    found.builds = found.builds.max(inherited);
                }
                let paint = matches!(found.slot, Slot::Fill | Slot::Stroke);
                if found.decorator && paint && found.multiplies {
                    found.builds = found.builds.saturating_add(total[3]);
                }
            }
            let walked = &self.walked[source];
            for found in walked
                .found
                .iter()
                .filter(|found| kept(found) && found.context && found.slot.is_marker())
            {
                let passed = self.walked[found.target].context_painted;
                context = context.saturating_add(found.builds.saturating_mul(passed));
            }
            if walked.context_decorator {
                context = context.saturating_add(total[3]);
            }
            self.walked[source].context_painted = context;
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

/// Whether the parser shapes `tag`'s text: text, its spans and paths, and
/// links `in_text` (elsewhere a link is a group).
pub(super) fn text_content(tag: &str, in_text: bool) -> bool {
    matches!(tag, "text" | "tspan" | "textPath") || (tag == "a" && in_text)
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
