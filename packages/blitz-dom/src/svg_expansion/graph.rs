//! The elements usvg builds from a parsed document, and what each names.
//!
//! usvg's `svgtree` builds the SVG elements it knows, skipping any other
//! element with its subtree, and copies each `<use>` target where the
//! `<use>` stands. Converting the tree then builds again what an element
//! names: a clip path, mask or filter for each element it applies to, a
//! pattern or gradient for each element it paints, and a marker for each
//! vertex it marks.

mod build;

use super::styles::{Declarations, Styles};
use build::{Builder, first_ids};
use std::collections::HashMap;
use usvg::roxmltree::{self, Node};

use super::XLINK_NS;

/// Elements whose `href` usvg follows besides `<use>` and `<tref>`, with the
/// elements it follows it to (`None` for any).
const HREFS: [(&str, Option<&[&str]>); 6] = [
    (
        "linearGradient",
        Some(&["linearGradient", "radialGradient"]),
    ),
    (
        "radialGradient",
        Some(&["linearGradient", "radialGradient"]),
    ),
    ("pattern", Some(&["pattern"])),
    ("filter", Some(&["filter"])),
    // A text path follows any shape.
    (
        "textPath",
        Some(&[
            "path", "rect", "circle", "ellipse", "line", "polyline", "polygon",
        ]),
    ),
    ("feImage", None),
];

/// Where an element names another one, by how usvg builds the target.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Slot {
    /// Built once where it is named (clip paths, masks and `href`s).
    Once,
    /// Built for each element it paints, which inherit it.
    Fill,
    Stroke,
    /// Built for each vertex it marks, which inherit it.
    MarkerStart,
    MarkerMid,
    MarkerEnd,
    /// Filter lists inherit only when explicitly requested.
    Filter,
}

impl Slot {
    /// The slots that support inheritance, including explicitly inherited filters.
    pub(super) const INHERITED: [Slot; 6] = [
        Slot::Fill,
        Slot::Stroke,
        Slot::MarkerStart,
        Slot::MarkerMid,
        Slot::MarkerEnd,
        Slot::Filter,
    ];

    /// The slot's place in [`Slot::INHERITED`], if it supports inheritance.
    pub(super) fn inherited_index(self) -> Option<usize> {
        Self::INHERITED.iter().position(|slot| *slot == self)
    }

    /// The slot of a property naming elements, and whether its value is a
    /// paint (with a fallback) rather than a bare reference.
    fn of_property(name: &str) -> Option<(Slot, Value)> {
        Some(match name {
            "fill" => (Slot::Fill, Value::Paint),
            "stroke" => (Slot::Stroke, Value::Paint),
            "marker-start" => (Slot::MarkerStart, Value::Reference),
            "marker-mid" => (Slot::MarkerMid, Value::Reference),
            "marker-end" => (Slot::MarkerEnd, Value::Reference),
            "clip-path" | "mask" => (Slot::Once, Value::Reference),
            "filter" => (Slot::Filter, Value::Filters),
            _ => return None,
        })
    }
}

#[derive(Clone, Copy)]
enum Value {
    Paint,
    Reference,
    Filters,
}

/// What an inheritable property is declared to on one element.
#[derive(Clone, Debug, Default)]
pub(super) struct Declared {
    /// The elements it may name.
    pub targets: Vec<usize>,
    /// Whether it may take the inherited value: explicit `inherit`, a
    /// context paint (which a `<use>` passes on), or an undeclared property
    /// that inherits by default (all slots here except filters).
    pub inherits: bool,
    /// Whether it may be a context paint, which a marked shape passes on.
    pub context: bool,
}

/// One element usvg builds.
pub(super) struct Element<'a, 'input> {
    pub node: Node<'a, 'input>,
    pub tag: &'a str,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub declarations: Declarations<'a>,
    /// The elements it names that usvg builds once each time it builds
    /// this one: its clip paths, masks, filters and `href` target.
    pub once: Vec<usize>,
    pub inherited: [Declared; Slot::INHERITED.len()],
    /// The target its `<use>` copies, unless usvg skips it as recursive.
    pub instance: Option<usize>,
    /// The `<use>` elements that copy it.
    pub instanced_by: Vec<usize>,
    /// Whether usvg builds it only as part of a `<use>` copy.
    pub copied_only: bool,
}

/// The elements usvg builds from a document, in document order.
pub(super) struct Graph<'a, 'input> {
    pub elements: Vec<Element<'a, 'input>>,
    /// The bytes of `<tref>` target text, by element.
    pub tref_text: HashMap<usize, usize>,
}

/// A document refused before usvg builds it: nested too deep, or with
/// references forming a cycle.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Refused;

impl<'a, 'input> Graph<'a, 'input> {
    /// The elements of `doc` and their references. Refuses a document whose
    /// elements nest deeper than `max_depth` (usvg recurses once per level),
    /// or that declares more references than [`super::MAX_DECLARATIONS_PER_NODE`]
    /// allows.
    pub(super) fn of(
        doc: &'a roxmltree::Document<'input>,
        styles: &Styles<'a>,
        max_depth: usize,
    ) -> Result<Self, Refused> {
        let mut builder = Builder {
            styles,
            elements: Vec::new(),
            index: HashMap::new(),
            declarations: 0,
            max_declarations: super::nodes(doc).saturating_mul(super::MAX_DECLARATIONS_PER_NODE),
            max_depth,
        };
        builder.subtree(doc.root_element(), false)?;
        // usvg copies a `<use>` target wherever it stands, also where it
        // builds nothing itself (below an unknown element, in text or in
        // `<use>` content), so those targets are built as copies alone.
        let ids = first_ids(doc);
        let mut next = 0;
        while next < builder.elements.len() {
            let element = &builder.elements[next];
            next += 1;
            if element.tag != "use" {
                continue;
            }
            let Some(target) = href(element.node).and_then(|id| ids.get(id).copied()) else {
                continue;
            };
            if !builder.index.contains_key(&target.id()) {
                builder.subtree(target, true)?;
            }
        }
        let mut graph = Self {
            elements: builder.elements,
            tref_text: HashMap::new(),
        };
        // A value may name many elements (a filter list), so the references
        // kept share the declarations' allowance.
        let mut allowance = builder.max_declarations - builder.declarations;
        graph.link(&ids, &builder.index, &mut allowance)?;
        Ok(graph)
    }

    /// Resolve each element's references as usvg does.
    fn link(
        &mut self,
        first: &HashMap<&'a str, Node<'a, 'input>>,
        index: &HashMap<roxmltree::NodeId, usize>,
        allowance: &mut usize,
    ) -> Result<(), Refused> {
        let ids = Ids::of(first, &self.elements);
        let mut copied_text: HashMap<roxmltree::NodeId, usize> = HashMap::new();
        for at in 0..self.elements.len() {
            let (node, tag) = (self.elements[at].node, self.elements[at].tag);
            let (mut once, inherited) = self.declared_references(at, &ids);
            once.extend(self.href_target(at, &ids));
            let kept = once.len() + inherited.iter().map(|d| d.targets.len()).sum::<usize>();
            *allowance = allowance.checked_sub(kept).ok_or(Refused)?;
            let instance = match tag {
                "use" => ids
                    .first(node)
                    .and_then(|target| index.get(&target.id()).copied()),
                _ => None,
            };
            if tag == "tref" {
                let text = ids.first(node).map_or(0, |target| {
                    *copied_text
                        .entry(target.id())
                        .or_insert_with(|| text_bytes(target))
                });
                self.tref_text.insert(at, text);
            }
            let element = &mut self.elements[at];
            element.once = once;
            element.inherited = inherited;
            element.instance = instance;
        }
        self.drop_recursive_uses();
        Ok(())
    }

    /// What element `at`'s properties name: the targets built once, and the
    /// inherited slots it declares.
    fn declared_references(
        &self,
        at: usize,
        ids: &Ids,
    ) -> (Vec<usize>, [Declared; Slot::INHERITED.len()]) {
        let mut once = Vec::new();
        let mut inherited: [Declared; Slot::INHERITED.len()] = Default::default();
        for (slot, declared) in Slot::INHERITED.iter().zip(&mut inherited) {
            declared.inherits = *slot != Slot::Filter;
        }
        let mut declared_slots = [false; Slot::INHERITED.len()];
        for (name, value) in self.elements[at].declarations.iter() {
            let Some((slot, kind)) = Slot::of_property(name) else {
                continue;
            };
            // usvg drops a reference to an element containing it.
            let targets = named(value, kind)
                .into_iter()
                .filter_map(|id| ids.last(id))
                .filter(|&target| !self.contains(target, at));
            let Some(position) = slot.inherited_index() else {
                once.extend(targets);
                continue;
            };
            let declared = &mut inherited[position];
            if !declared_slots[position] {
                declared_slots[position] = true;
                declared.inherits = false;
            }
            declared.targets.extend(targets);
            let value = value.trim();
            if value == "inherit" {
                declared.inherits = true;
            }
            if value.starts_with("context-") {
                declared.inherits = true;
                declared.context = true;
            }
        }
        (once, inherited)
    }

    /// The element that element `at`'s `href` names, for the elements whose
    /// `href` usvg follows besides `<use>` and `<tref>`.
    fn href_target(&self, at: usize, ids: &Ids) -> Option<usize> {
        let element = &self.elements[at];
        let (_, accepted) = HREFS.iter().find(|(name, _)| *name == element.tag)?;
        let target = ids.last(href(element.node)?)?;
        let accepted = !self.contains(target, at)
            && accepted.is_none_or(|tags| tags.contains(&self.elements[target].tag));
        accepted.then_some(target)
    }

    /// Drop the `<use>` copies usvg skips as recursive: those of a target
    /// containing a `<use>` of itself or of the copying `<use>`.
    fn drop_recursive_uses(&mut self) {
        let mut users: HashMap<usize, Vec<usize>> = HashMap::new();
        for at in 0..self.elements.len() {
            if let Some(target) = self.elements[at].instance {
                users.entry(target).or_default().push(at);
            }
        }
        let inside = |graph: &Self, target: usize, of: usize| {
            users
                .get(&of)
                .is_some_and(|uses| uses.iter().any(|&inner| graph.contains(target, inner)))
        };
        let self_recursive: HashMap<usize, bool> = users
            .keys()
            .map(|&target| (target, inside(self, target, target)))
            .collect();
        for at in 0..self.elements.len() {
            let Some(target) = self.elements[at].instance else {
                continue;
            };
            if self_recursive[&target] || inside(self, target, at) {
                self.elements[at].instance = None;
            } else {
                self.elements[target].instanced_by.push(at);
            }
        }
    }

    /// Whether `inner` is `outer` or below it.
    pub(super) fn contains(&self, outer: usize, inner: usize) -> bool {
        let mut at = Some(inner);
        while let Some(element) = at {
            if element == outer {
                return true;
            }
            at = self.elements[element].parent;
        }
        false
    }
}

/// How usvg looks IDs up: `<use>` and `<tref>` among all elements, the
/// first winning; other references among built elements, the last winning.
struct Ids<'m, 'a, 'input> {
    first: &'m HashMap<&'a str, Node<'a, 'input>>,
    last: HashMap<&'a str, usize>,
}

impl<'m, 'a, 'input> Ids<'m, 'a, 'input> {
    /// Copies keep no IDs, so only elements built where they stand count.
    fn of(first: &'m HashMap<&'a str, Node<'a, 'input>>, elements: &[Element<'a, 'input>]) -> Self {
        let mut last = HashMap::new();
        for (at, element) in elements.iter().enumerate() {
            if element.copied_only {
                continue;
            }
            if let Some(id) = element.node.attribute("id") {
                last.insert(id, at);
            }
        }
        Self { first, last }
    }

    /// The element `node`'s `href` names, as `<use>` and `<tref>` find it.
    fn first(&self, node: Node<'a, 'input>) -> Option<Node<'a, 'input>> {
        self.first.get(href(node)?).copied()
    }

    /// The built element `id` names, as other references find it.
    fn last(&self, id: &str) -> Option<usize> {
        self.last.get(id).copied()
    }
}

/// The local IDs `value` names as a property of `kind`, as usvg reads it.
fn named(value: &str, kind: Value) -> Vec<&str> {
    match kind {
        Value::Paint => match svgtypes::Paint::from_str(value) {
            Ok(svgtypes::Paint::FuncIRI(id, _)) => vec![id],
            _ => Vec::new(),
        },
        Value::Reference => svgtypes::FuncIRI::from_str(value)
            .map(|iri| vec![iri.0])
            .unwrap_or_default(),
        Value::Filters => svgtypes::FilterValueListParser::from(value)
            .filter_map(|filter| match filter {
                Ok(svgtypes::FilterValue::Url(id)) => Some(id),
                _ => None,
            })
            .collect(),
    }
}

/// The ID an element's `href` names, preferring the unprefixed one.
fn href<'a>(node: Node<'a, '_>) -> Option<&'a str> {
    let value = node
        .attributes()
        .find(|a| a.name() == "href" && a.namespace().is_none())
        .or_else(|| {
            node.attributes()
                .find(|a| a.name() == "href" && a.namespace() == Some(XLINK_NS))
        })?
        .value();
    svgtypes::IRI::from_str(value).ok().map(|iri| iri.0)
}

/// The bytes of character data below `node`, which a `<tref>` copies.
pub(super) fn text_bytes(node: Node) -> usize {
    node.descendants()
        .filter(Node::is_text)
        .filter_map(|text| text.text())
        .map(str::len)
        .fold(0, usize::saturating_add)
}
