//! What usvg allocates to build and convert a parsed document, bounded
//! before it runs.
//!
//! Each element costs what building it once allocates (its node, attributes,
//! copied declarations, path curves and text), and an element is counted
//! once for each time usvg builds it: once where it stands, again for each
//! `<use>` copy, each element a clip path, mask, filter or `feImage` applies
//! to, each element a paint server paints and each vertex a marker marks.
//! The count follows the references an element inherits too, so a
//! `fill="url(#p)"` on a group counts the pattern for each shape below it.
//!
//! A document whose references form a cycle is refused: usvg breaks some
//! cycles itself and fails on others, and bounding the expansions of a
//! cycle would need every path through it.

use super::MAX_NESTING;
use super::curves::{arc_curves, path_curves};
use super::graph::{Graph, Refused, Slot, text_bytes};
use super::lengths;
use super::styles::Styles;
use std::collections::{HashMap, HashSet};
use usvg::roxmltree;

/// What usvg's `svgtree` allocates for each element it builds and each
/// attribute it keeps.
const BYTES_PER_BUILT_ELEMENT: usize = 64;
const BYTES_PER_ATTRIBUTE: usize = 48;
/// What a converted node allocates, by what it becomes.
const BYTES_PER_NODE: usize = 1024;
const BYTES_PER_STOP: usize = 64;
const BYTES_PER_PRIMITIVE: usize = 1024;
/// A span of text keeps its own style and font, and flattens to its own
/// paths, decorations included.
const BYTES_PER_SPAN: usize = 4096;
/// Each curve of a path, with what computing its stroke bounds allocates.
const BYTES_PER_CURVE: usize = 64;
/// Each byte of text: its copy, layout and flattened glyph outlines.
const BYTES_PER_TEXT_BYTE: usize = 2048;
/// Inherited properties usvg copies, as lists, into every element it paints.
const COPIED_LISTS: [&str; 2] = ["stroke-dasharray", "font-family"];
/// Each byte of such a list: a dash becomes two numbers (an odd list is
/// repeated), a family a string of its own.
const BYTES_PER_LIST_BYTE: usize = 16;
/// Recursive calls of the measure itself. Each nesting level of the document
/// may also walk up through as many levels to find inherited references.
const MAX_FRAMES: usize = 4 * MAX_NESTING;
/// Preflight target walks have their own allowance, before expansion costs.
const MAX_REFERENCE_SCAN_WORK: usize = 2_000_000;

/// What usvg builds and converts a document to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Conversion {
    /// What it allocates, at most.
    pub bytes: usize,
    /// How many levels of elements and references it recurses through.
    pub levels: usize,
    /// CPU work repeated while converting expanded instances.
    pub work: usize,
}

/// What usvg allocates to convert `doc`, or `None` when its elements and
/// references nest deeper than usvg can recurse, form a cycle, or stylesheet
/// parsing/matching exceeds the preflight's work allowance. Expanded matching
/// work is returned for the caller to admit independently of memory.
pub(crate) fn conversion(doc: &roxmltree::Document) -> Option<Conversion> {
    let reference_scan_work = reference_scan_work(doc);
    // Graph construction also visits tref target subtrees. Admit those scans
    // before linking, including targets hidden below unknown XML elements.
    let references = doc
        .descendants()
        .filter(|node| node.has_tag_name("tref"))
        .count();
    if reference_scan_work.saturating_mul(references) > MAX_REFERENCE_SCAN_WORK {
        return None;
    }
    let styles = Styles::of(doc)?;
    let graph = Graph::of(doc, &styles, MAX_NESTING).ok()?;
    if graph.elements.is_empty() {
        return Some(Conversion {
            bytes: 0,
            levels: 0,
            work: 0,
        });
    }
    let mut measure = Measure {
        graph: &graph,
        reference_scan_work,
        built: HashMap::new(),
        filters: HashMap::new(),
        inherited: HashMap::new(),
        lists: HashMap::new(),
        fonts: HashMap::new(),
        viewports: HashMap::new(),
        context_paint: None,
        active: HashSet::new(),
        frames: 0,
        levels: 0,
    };
    let cost = measure.subtree(0).ok()?;
    Some(Conversion {
        // usvg holds the parsed stylesheets while it builds the document.
        bytes: cost
            .bytes
            .saturating_add(super::styles::stylesheet_bytes(doc)),
        levels: cost.levels,
        // usvg collects unique filters with a linear scan of prior filters.
        work: cost
            .work
            .saturating_add(cost.filters.saturating_mul(cost.filters)),
    })
}

/// What building an element allocates, and how deep usvg recurses for it.
#[derive(Clone, Copy, Debug, Default)]
struct Cost {
    bytes: usize,
    levels: usize,
    work: usize,
    filters: usize,
}

impl Cost {
    fn add(&mut self, other: Cost) {
        self.bytes = self.bytes.saturating_add(other.bytes);
        self.levels = self.levels.max(other.levels);
        self.work = self.work.saturating_add(other.work);
        self.filters = self.filters.saturating_add(other.filters);
    }

    fn add_times(&mut self, other: Cost, times: usize) {
        self.bytes = self.bytes.saturating_add(other.bytes.saturating_mul(times));
        self.levels = self.levels.max(other.levels);
        self.work = self.work.saturating_add(other.work.saturating_mul(times));
        self.filters = self
            .filters
            .saturating_add(other.filters.saturating_mul(times));
    }

    fn max(self, other: Cost) -> Cost {
        Cost {
            bytes: self.bytes.max(other.bytes),
            levels: self.levels.max(other.levels),
            work: self.work.max(other.work),
            filters: self.filters.max(other.filters),
        }
    }
}

/// A computation in progress, to find cycles.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Step {
    Subtree(usize),
    Inherited(usize, Slot),
    List(usize, &'static str),
    ContextPaint,
    Filters(usize),
    Font(usize),
    Viewport(usize),
}

struct Measure<'g, 'a, 'input> {
    graph: &'g Graph<'a, 'input>,
    reference_scan_work: usize,
    built: HashMap<usize, Cost>,
    filters: HashMap<usize, usize>,
    inherited: HashMap<(usize, Slot), Cost>,
    lists: HashMap<(usize, &'static str), usize>,
    fonts: HashMap<usize, f64>,
    viewports: HashMap<usize, f64>,
    context_paint: Option<Cost>,
    active: HashSet<Step>,
    frames: usize,
    /// Elements being measured, each below the last.
    levels: usize,
}

impl Measure<'_, '_, '_> {
    /// What building `element` once allocates, with everything below it and
    /// everything it names.
    fn subtree(&mut self, element: usize) -> Result<Cost, Refused> {
        if let Some(cost) = self.built.get(&element) {
            return Ok(*cost);
        }
        self.enter(Step::Subtree(element))?;
        let graph = self.graph;
        let node = &graph.elements[element];
        let mut cost = Cost {
            bytes: self.own_bytes(element)?,
            levels: 0,
            work: node
                .declarations
                .match_work
                .saturating_add(if node.tag == "tref" {
                    self.reference_scan_work
                } else {
                    0
                }),
            filters: self
                .filter_functions(element)?
                .saturating_add(usize::from(node.tag == "filter")),
        };
        for &child in &node.children {
            cost.add(self.subtree(child)?);
        }
        for &target in node.instance.iter().chain(&node.once) {
            cost.add(self.subtree(target)?);
        }
        cost.add(self.inherited(element, Slot::Filter)?);
        if painted(node.tag) {
            cost.add(self.inherited(element, Slot::Fill)?);
            cost.add(self.inherited(element, Slot::Stroke)?);
            if node.tag == "text" {
                // Text clones its paint servers for each chunk, span and
                // flattened path; an absolute position starts a chunk.
                let (paint, clones) = self.text_paints(element)?;
                cost.add_times(paint, clones.saturating_mul(3));
            }
            for property in COPIED_LISTS {
                let bytes = self.inherited_list(element, property)?;
                cost.add(Cost {
                    bytes: bytes.saturating_mul(BYTES_PER_LIST_BYTE),
                    levels: 0,
                    work: 0,
                    filters: 0,
                });
            }
        }
        if markable(node.tag) {
            let vertices = self.curves(element)?.saturating_add(1);
            cost.add(self.inherited(element, Slot::MarkerStart)?);
            cost.add(self.inherited(element, Slot::MarkerEnd)?);
            cost.add_times(self.inherited(element, Slot::MarkerMid)?, vertices);
        }
        cost.levels += 1;
        if cost.levels > MAX_NESTING {
            return Err(Refused);
        }
        self.leave(Step::Subtree(element));
        self.built.insert(element, cost);
        Ok(cost)
    }

    /// The largest cost of what `element` may name at an inherited `slot`:
    /// what it declares, or what an ancestor or a `<use>` copying it passes on.
    fn inherited(&mut self, element: usize, slot: Slot) -> Result<Cost, Refused> {
        if let Some(cost) = self.inherited.get(&(element, slot)) {
            return Ok(*cost);
        }
        let mut cost = Cost::default();
        let Some(position) = slot.inherited_index() else {
            return Ok(cost);
        };
        self.enter(Step::Inherited(element, slot))?;
        let graph = self.graph;
        let declared = &graph.elements[element].inherited[position];
        for &target in &declared.targets {
            let target_cost = self.subtree(target)?;
            if slot == Slot::Filter {
                cost.add(target_cost);
            } else {
                cost = cost.max(target_cost);
            }
        }
        if declared.context {
            cost = cost.max(self.context_paint()?);
        }
        if declared.inherits {
            for source in sources(graph, element) {
                cost = cost.max(self.inherited(source, slot)?);
            }
        }
        self.leave(Step::Inherited(element, slot));
        self.inherited.insert((element, slot), cost);
        Ok(cost)
    }

    /// The largest paint a context paint may stand for: any paint server a
    /// shape names (usvg passes a marked shape's paint into its markers).
    fn context_paint(&mut self) -> Result<Cost, Refused> {
        if let Some(cost) = self.context_paint {
            return Ok(cost);
        }
        self.enter(Step::ContextPaint)?;
        let graph = self.graph;
        let mut cost = Cost::default();
        let paints = [Slot::Fill, Slot::Stroke].map(Slot::inherited_index);
        for node in &graph.elements {
            for position in paints.into_iter().flatten() {
                for &target in &node.inherited[position].targets {
                    cost = cost.max(self.subtree(target)?);
                }
            }
        }
        self.leave(Step::ContextPaint);
        self.context_paint = Some(cost);
        Ok(cost)
    }

    /// The longest value of an inherited list `property` `element` may have:
    /// usvg copies it into each element it paints.
    fn inherited_list(&mut self, element: usize, property: &'static str) -> Result<usize, Refused> {
        if let Some(bytes) = self.lists.get(&(element, property)) {
            return Ok(*bytes);
        }
        self.enter(Step::List(element, property))?;
        let graph = self.graph;
        let mut bytes = 0;
        let (mut declared, mut inherits) = (false, false);
        for value in graph.elements[element].declarations.values(property) {
            bytes = bytes.max(value.len());
            declared = true;
            inherits |= value.trim() == "inherit";
        }
        let inherits = inherits || !declared;
        if inherits {
            for source in sources(graph, element) {
                bytes = bytes.max(self.inherited_list(source, property)?);
            }
        }
        self.leave(Step::List(element, property));
        self.lists.insert((element, property), bytes);
        Ok(bytes)
    }

    /// The largest paint the spans of the `<text>` `element` may name, and
    /// how many chunks and spans usvg may clone it for.
    fn text_paints(&mut self, element: usize) -> Result<(Cost, usize), Refused> {
        let graph = self.graph;
        let (mut paint, mut clones) = (Cost::default(), 1usize);
        let mut pending = vec![element];
        while let Some(at) = pending.pop() {
            let node = &graph.elements[at];
            paint = paint.max(self.inherited(at, Slot::Fill)?);
            paint = paint.max(self.inherited(at, Slot::Stroke)?);
            let positions = ["x", "y"]
                .iter()
                .filter_map(|name| node.node.attribute(*name))
                .map(|list| svgtypes::LengthListParser::from(list).count())
                .fold(0, usize::saturating_add);
            clones = clones.saturating_add(1).saturating_add(positions);
            pending.extend(&node.children);
        }
        Ok((paint, clones))
    }

    /// The bytes of text `element` holds or copies.
    fn text_bytes(&self, element: usize) -> usize {
        let node = &self.graph.elements[element];
        match node.tag {
            "tref" => self.graph.tref_text.get(&element).copied().unwrap_or(0),
            _ => text_bytes(node.node),
        }
    }

    /// What building `element` alone allocates.
    fn own_bytes(&mut self, element: usize) -> Result<usize, Refused> {
        let graph = self.graph;
        let node = &graph.elements[element];
        let attributes = node.node.attributes().len();
        let converted = match node.tag {
            "stop" => BYTES_PER_STOP,
            "text" | "tspan" | "tref" | "textPath" | "a" => BYTES_PER_SPAN,
            // A viewport adds a group and a clip path for its content.
            "svg" | "symbol" | "use" => 2 * BYTES_PER_NODE,
            tag if tag.starts_with("fe") => BYTES_PER_PRIMITIVE,
            _ => BYTES_PER_NODE,
        };
        let text = match node.tag {
            // Spans hold text their `<text>` already counts; a `<tref>`
            // copies its own.
            "text" | "tref" => self.text_bytes(element),
            _ => 0,
        };
        // Each filter function becomes a filter of its own.
        let functions = self.filter_functions(element)?;
        let curves = self.curves(element)?;
        Ok([
            BYTES_PER_BUILT_ELEMENT,
            attributes.saturating_mul(BYTES_PER_ATTRIBUTE),
            node.declarations.copied_bytes,
            converted,
            curves.saturating_mul(BYTES_PER_CURVE),
            text.saturating_mul(BYTES_PER_TEXT_BYTE),
            functions.saturating_mul(BYTES_PER_NODE + BYTES_PER_PRIMITIVE),
        ]
        .into_iter()
        .fold(0, usize::saturating_add))
    }

    /// Filter functions are not inherited by default. Explicit `inherit`
    /// copies the immediate built parent's list, including a use instance.
    fn filter_functions(&mut self, element: usize) -> Result<usize, Refused> {
        if let Some(count) = self.filters.get(&element) {
            return Ok(*count);
        }
        self.enter(Step::Filters(element))?;
        let graph = self.graph;
        let declarations = &graph.elements[element].declarations;
        let mut count = filter_functions(declarations);
        if declarations
            .values("filter")
            .any(|value| value.trim() == "inherit")
        {
            let mut inherited = 0;
            for source in sources(graph, element) {
                inherited = inherited.max(self.filter_functions(source)?);
            }
            count = count.saturating_add(inherited);
        }
        self.leave(Step::Filters(element));
        self.filters.insert(element, count);
        Ok(count)
    }

    /// The curves usvg makes of `element`'s shape.
    fn curves(&mut self, element: usize) -> Result<usize, Refused> {
        let node = self.graph.elements[element].node;
        let attribute = |name| node.attribute(name).unwrap_or("");
        let points = || svgtypes::PointsParser::from(attribute("points")).count();
        Ok(match self.graph.elements[element].tag {
            "path" => path_curves(attribute("d")),
            "polyline" | "polygon" => points(),
            "line" => 1,
            "rect" => 4usize.saturating_add(self.arc(element, &["rx", "ry"])?),
            "circle" => self.arc(element, &["r"])?,
            "ellipse" => self.arc(element, &["rx", "ry"])?,
            _ => 0,
        })
    }

    /// The curves of an arc whose radius is the largest of `radii`, or none
    /// when none is declared.
    fn arc(&mut self, element: usize, radii: &[&str]) -> Result<usize, Refused> {
        let node = self.graph.elements[element].node;
        let mut radius = None::<f64>;
        for name in radii {
            let Some(value) = node.attribute(*name) else {
                continue;
            };
            let relative = value.contains(['e', '%']);
            let (font, viewport) = if relative {
                (self.font(element)?, self.viewport(element)?)
            } else {
                (lengths::DEFAULT_FONT_SIZE, lengths::DEFAULT_VIEWPORT)
            };
            if let Some(bound) = lengths::length(value, font, viewport) {
                radius = Some(radius.unwrap_or(0.0).max(bound));
            }
        }
        Ok(radius.map_or(0, |radius| arc_curves(lengths::radius(radius))))
    }

    /// The largest font size `element` may have.
    fn font(&mut self, element: usize) -> Result<f64, Refused> {
        if let Some(size) = self.fonts.get(&element) {
            return Ok(*size);
        }
        self.enter(Step::Font(element))?;
        let graph = self.graph;
        let mut parent = None::<f64>;
        for source in sources(graph, element) {
            let size = self.font(source)?;
            parent = Some(parent.unwrap_or(0.0).max(size));
        }
        let parent = parent.unwrap_or(lengths::DEFAULT_FONT_SIZE);
        let declarations = &graph.elements[element].declarations;
        let size = declarations
            .values("font-size")
            .map(|value| lengths::font_size(value, parent))
            .fold(None::<f64>, |largest, size| {
                Some(largest.unwrap_or(0.0).max(size))
            })
            .unwrap_or(parent);
        self.leave(Step::Font(element));
        self.fonts.insert(element, size);
        Ok(size)
    }

    /// The largest side of a viewport `element` resolves percentages
    /// against: any size an ancestor or a `<use>` copying it declares.
    fn viewport(&mut self, element: usize) -> Result<f64, Refused> {
        if let Some(side) = self.viewports.get(&element) {
            return Ok(*side);
        }
        self.enter(Step::Viewport(element))?;
        let graph = self.graph;
        let mut side = lengths::DEFAULT_VIEWPORT;
        for source in sources(graph, element) {
            side = side.max(self.viewport(source)?);
            side = side.max(self.declared_size(source)?);
        }
        if graph.elements[element].parent.is_none() {
            side = side.max(self.declared_size(element)?);
        }
        self.leave(Step::Viewport(element));
        self.viewports.insert(element, side);
        Ok(side)
    }

    /// The largest side `element` declares for its content, with its
    /// `width`, `height` or `viewBox`.
    fn declared_size(&mut self, element: usize) -> Result<f64, Refused> {
        let node = self.graph.elements[element].node;
        let mut side = node
            .attribute("viewBox")
            .and_then(lengths::view_box_side)
            .unwrap_or(0.0);
        for name in ["width", "height"] {
            let Some(value) = node.attribute(name) else {
                continue;
            };
            let font = self.font(element)?;
            // A percentage of the enclosing viewport; the root's is usvg's default.
            let outer = match self.graph.elements[element].parent {
                Some(_) => self.viewport(element)?,
                None => lengths::DEFAULT_VIEWPORT,
            };
            side = side.max(lengths::length(value, font, outer).unwrap_or(0.0));
        }
        Ok(side)
    }

    /// Begin `step`, refusing a cycle, or recursion past what usvg's own
    /// would be: levels below the root, or calls of the measure.
    fn enter(&mut self, step: Step) -> Result<(), Refused> {
        self.frames += 1;
        if matches!(step, Step::Subtree(_)) {
            self.levels += 1;
        }
        if self.frames > MAX_FRAMES || self.levels > MAX_NESTING || !self.active.insert(step) {
            return Err(Refused);
        }
        Ok(())
    }

    fn leave(&mut self, step: Step) {
        self.frames -= 1;
        if matches!(step, Step::Subtree(_)) {
            self.levels -= 1;
        }
        self.active.remove(&step);
    }
}

/// Where `element` inherits from: its parent, and each `<use>` copying it.
fn sources(graph: &Graph, element: usize) -> Vec<usize> {
    let node = &graph.elements[element];
    node.parent
        .into_iter()
        .chain(node.instanced_by.iter().copied())
        .collect()
}

/// Elements usvg paints with a fill and a stroke: spans paint their own,
/// unlike the elements Blink paints in `canvas_svg_element::elements`.
fn painted(tag: &str) -> bool {
    is_text(tag)
        || matches!(
            tag,
            "path" | "rect" | "circle" | "ellipse" | "line" | "polyline" | "polygon"
        )
}

/// Elements usvg lays text out in.
fn is_text(tag: &str) -> bool {
    matches!(tag, "text" | "tspan" | "tref" | "textPath" | "a")
}

/// Elements usvg draws markers on.
fn markable(tag: &str) -> bool {
    matches!(tag, "path" | "line" | "polyline" | "polygon")
}

/// Every tref searches the original XML (including unknown elements), then
/// walks its target subtree. Charge ID comparisons by bytes, not just nodes.
fn reference_scan_work(doc: &roxmltree::Document) -> usize {
    doc.descendants().fold(0usize, |work, node| {
        let attributes = node.attributes().fold(0usize, |cost, attr| {
            cost.saturating_add(attr.name().len())
                .saturating_add(if attr.name() == "id" {
                    attr.value().len()
                } else {
                    0
                })
        });
        work.saturating_add(2).saturating_add(attributes)
    })
}

fn filter_functions(declarations: &super::styles::Declarations<'_>) -> usize {
    declarations
        .values("filter")
        .flat_map(svgtypes::FilterValueListParser::from)
        .filter(|function| !matches!(function, Ok(svgtypes::FilterValue::Url(_))))
        .count()
}
