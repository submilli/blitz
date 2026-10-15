//! What editing asks of layout (Blink's `LayoutObject` predicates), over a
//! laid-out document, with the inline formatting contexts it builds on the
//! way. One context serves one `modify()` call.

use super::ifc::{self, Ifc};
use super::position::Position;
use super::tree::Tree;
use crate::layout::construct::{InlineElement, inline_element};
use crate::node::NodeData;
use crate::{BaseDocument, NodeId};
use icu_segmenter::GraphemeClusterSegmenter;
use markup5ever::local_name;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use style::computed_values::visibility::T as Visibility;
use style::values::computed::{Display, UserSelect};
use style::values::specified::box_::{DisplayInside, DisplayOutside};

/// The kind of a node's layout object.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Text,
    Br,
    /// An inline box around its content (`LayoutInline`).
    Inline,
    /// An atomic inline-level box: a replaced element, a control, or an
    /// inline-level box with a formatting context of its own.
    Atomic {
        block_flow: bool,
    },
    /// A block-level box, including floats and positioned boxes.
    Block {
        block_flow: bool,
        flex_or_grid: bool,
    },
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Object {
    pub(super) kind: Kind,
    pub(super) visible: bool,
    pub(super) selectable: bool,
    /// A box that contains its children's boxes (`LayoutBlock`), unlike
    /// text, inline boxes and replaced elements.
    pub(super) block_container: bool,
}

impl Object {
    pub(super) fn is_text(self) -> bool {
        matches!(self.kind, Kind::Text | Kind::Br)
    }

    /// An atomic inline: a replaced element or an inline-level block
    /// container, laid out as one box in its line.
    pub(super) fn is_atomic(self) -> bool {
        matches!(self.kind, Kind::Atomic { .. })
    }

    /// `IsInline`: laid out in a line, which floats and positioned boxes
    /// are not.
    pub(super) fn is_inline(self) -> bool {
        matches!(
            self.kind,
            Kind::Text | Kind::Br | Kind::Inline | Kind::Atomic { .. }
        )
    }

    pub(super) fn is_block_flow(self) -> bool {
        matches!(
            self.kind,
            Kind::Atomic { block_flow: true }
                | Kind::Block {
                    block_flow: true,
                    ..
                }
        )
    }

    /// A block flow, or a flex or grid container (`IsLayoutBlockFlow ||
    /// IsFlexibleBox || IsLayoutGrid`): a box that holds a caret when it is
    /// empty.
    pub(super) fn is_block_flow_or_flex_grid(self) -> bool {
        self.is_block_flow()
            || matches!(
                self.kind,
                Kind::Block {
                    flex_or_grid: true,
                    ..
                }
            )
    }
}

#[derive(Clone, Copy)]
struct CachedObject {
    object: Option<Object>,
    renders_children: bool,
}

pub(super) struct Context<'a> {
    pub(super) tree: Tree<'a>,
    objects: RefCell<HashMap<NodeId, CachedObject>>,
    /// Formatting contexts by inline root.
    ifcs: RefCell<HashMap<NodeId, Option<Rc<Ifc>>>>,
    graphemes: RefCell<HashMap<NodeId, Rc<Vec<usize>>>>,
    /// Whether a box has rendered content with height, by box.
    rendered: RefCell<HashMap<NodeId, bool>>,
    /// The candidate a scan for the next (`true`) or previous candidate
    /// found from a non-candidate node start it passed.
    candidates: RefCell<HashMap<(Position, bool), Option<Position>>>,
    /// Whether an inline box holds no content (see `canonical`), by box.
    empty_inlines: RefCell<HashMap<NodeId, bool>>,
    /// Whether an element's used `user-select` is `none`, by element.
    unselectable: RefCell<HashMap<NodeId, bool>>,
    /// The nearest laid-out inline root above a node, by node.
    inline_roots: RefCell<HashMap<NodeId, Option<NodeId>>>,
    /// Steps left for this call's walks over the tree (see [`Self::spend`]),
    /// shared by the call's contexts.
    budget: Rc<Cell<u32>>,
    /// For a DOM-tree context, a flat-tree context over the same document:
    /// Blink finds the most backward and forward positions of a DOM-tree
    /// position in the flat tree.
    flat: Option<Box<Context<'a>>>,
}

/// Steps one `modify()` call may take through positions and leaves. Blink's
/// line steps canonicalize at each leaf they pass, so their work can grow
/// with the square of a line's length; past this the call gives up and the
/// selection stays.
const WORK_BUDGET: u32 = 4_000_000;

impl<'a> Context<'a> {
    pub(super) fn new(doc: &'a BaseDocument) -> Self {
        Self::with_tree(Tree::new(doc), Rc::new(Cell::new(WORK_BUDGET)), None)
    }

    /// A context over the DOM tree, spending `self`'s budget.
    pub(super) fn dom(&self) -> Self {
        let doc = self.tree.doc();
        let budget = self.budget.clone();
        let flat = Self::with_tree(Tree::new(doc), budget.clone(), None);
        Self::with_tree(Tree::dom(doc), budget, Some(Box::new(flat)))
    }

    /// The flat-tree twin of a DOM-tree context.
    pub(super) fn flat(&self) -> Option<&Self> {
        self.flat.as_deref()
    }

    fn with_tree(tree: Tree<'a>, budget: Rc<Cell<u32>>, flat: Option<Box<Self>>) -> Self {
        Self {
            tree,
            objects: RefCell::default(),
            ifcs: RefCell::default(),
            graphemes: RefCell::default(),
            rendered: RefCell::default(),
            candidates: RefCell::default(),
            empty_inlines: RefCell::default(),
            unselectable: RefCell::default(),
            inline_roots: RefCell::default(),
            budget,
            flat,
        }
    }

    /// Take one step of the call's work budget; `false` once it is spent,
    /// which ends every walk.
    pub(super) fn spend(&self) -> bool {
        let left = self.budget.get();
        if left == 0 {
            return false;
        }
        self.budget.set(left - 1);
        true
    }

    /// The candidate a scan found from `passed`, a non-candidate position.
    pub(super) fn remembered_candidate(
        &self,
        passed: Position,
        forward: bool,
    ) -> Option<Option<Position>> {
        self.candidates.borrow().get(&(passed, forward)).copied()
    }

    /// Remember a scan's result for the non-candidate positions it passed:
    /// the scan from any of them finds the same candidate.
    pub(super) fn remember_candidate(
        &self,
        passed: Vec<Position>,
        forward: bool,
        found: Option<Position>,
    ) {
        if self.exhausted() {
            return;
        }
        let mut candidates = self.candidates.borrow_mut();
        for position in passed {
            candidates.insert((position, forward), found);
        }
    }

    /// Whether the work budget ran out, so results are incomplete.
    pub(super) fn exhausted(&self) -> bool {
        self.budget.get() == 0
    }

    /// A memoized `HasRenderedNonAnonymousDescendantsWithHeight`.
    pub(super) fn rendered_with_height(
        &self,
        node: NodeId,
        compute: impl FnOnce() -> bool,
    ) -> bool {
        if let Some(&found) = self.rendered.borrow().get(&node) {
            return found;
        }
        let found = compute();
        self.rendered.borrow_mut().insert(node, found);
        found
    }

    /// Whether an inline box is known to hold no content.
    pub(super) fn empty_inline(&self, node: NodeId) -> Option<bool> {
        self.empty_inlines.borrow().get(&node).copied()
    }

    pub(super) fn remember_empty_inline(&self, node: NodeId, empty: bool) {
        self.empty_inlines.borrow_mut().insert(node, empty);
    }

    pub(super) fn doc(&self) -> &'a BaseDocument {
        self.tree.doc()
    }

    /// A node's layout object, `None` when it is not rendered.
    pub(super) fn object(&self, node: NodeId) -> Option<Object> {
        if let Some(found) = self.objects.borrow().get(&node) {
            return found.object;
        }
        // Rendering depends on every flat ancestor. Fill the memo from the
        // root down without putting page-controlled depth on the call stack.
        let mut path = Vec::new();
        let mut current = Some(node);
        while let Some(id) = current {
            if self.objects.borrow().contains_key(&id) {
                break;
            }
            if !self.spend() {
                return None;
            }
            path.push(id);
            current = self.tree.node(id).flat_parent_id();
        }
        for id in path.into_iter().rev() {
            let data = self.tree.node(id);
            let parent_renders = data.flat_parent_id().is_some_and(|parent| {
                self.tree.is_document(parent)
                    || self
                        .objects
                        .borrow()
                        .get(&parent)
                        .is_some_and(|found| found.renders_children)
            });
            let object = self.compute_object(id, parent_renders);
            let contents = data
                .display_style()
                .is_some_and(|d| d.inside() == DisplayInside::Contents);
            self.objects.borrow_mut().insert(
                id,
                CachedObject {
                    object,
                    renders_children: object.is_some() || (contents && parent_renders),
                },
            );
        }
        self.objects
            .borrow()
            .get(&node)
            .and_then(|found| found.object)
    }

    pub(super) fn has_layout(&self, node: NodeId) -> bool {
        self.object(node).is_some()
    }

    fn compute_object(&self, node: NodeId, parent_renders: bool) -> Option<Object> {
        let data = self.tree.node(node);
        match &data.data {
            NodeData::Text(_) => {
                self.ifc_of(node)?;
                let parent = data.flat_parent_id()?;
                let style = self.style_object(parent)?;
                Some(Object {
                    kind: Kind::Text,
                    ..style
                })
            }
            NodeData::Element(_) | NodeData::AnonymousBlock(_) => {
                if !parent_renders {
                    return None;
                }
                let style = self.style_object(node)?;
                let kind = self.element_kind(node)?;
                Some(Object {
                    kind,
                    block_container: self.is_block_container(node, kind),
                    ..style
                })
            }
            _ => None,
        }
    }

    /// Visibility and selectability from an element's style; the kind is
    /// filled in by the caller.
    fn style_object(&self, element: NodeId) -> Option<Object> {
        let style = self.tree.node(element).primary_styles()?;
        Some(Object {
            kind: Kind::Inline,
            visible: style.clone_visibility() == Visibility::Visible,
            selectable: !self.unselectable(element),
            block_container: false,
        })
    }

    fn element_kind(&self, node: NodeId) -> Option<Kind> {
        let data = self.tree.node(node);
        let element = data.element_data()?;
        let display = data.display_style().unwrap_or(Display::inline());
        let replaced = is_replaced(element);
        // Even a `display: inline` button is laid out as an inline-block.
        let button_root = data.flags.is_button_root();
        match (display.outside(), display.inside()) {
            (DisplayOutside::None, _) => None,
            _ if self.tree.is_br(node) => Some(Kind::Br),
            (DisplayOutside::Inline, DisplayInside::Flow) if !replaced && !button_root => {
                Some(Kind::Inline)
            }
            (DisplayOutside::Inline, inside) => Some(Kind::Atomic {
                block_flow: !replaced && (inside == DisplayInside::FlowRoot || button_root),
            }),
            (_, inside) => Some(Kind::Block {
                block_flow: !replaced
                    && matches!(
                        inside,
                        DisplayInside::Flow | DisplayInside::FlowRoot | DisplayInside::TableCell
                    ),
                flex_or_grid: matches!(inside, DisplayInside::Flex | DisplayInside::Grid),
            }),
        }
    }

    fn is_block_container(&self, node: NodeId, kind: Kind) -> bool {
        let Some(element) = self.tree.node(node).element_data() else {
            return false;
        };
        matches!(kind, Kind::Atomic { .. } | Kind::Block { .. }) && !is_replaced(element)
    }

    /// Whether an element's used `user-select` is `none`: `auto` uses
    /// `none` inside an unselectable parent.
    /// <https://drafts.csswg.org/css-ui/#valdef-user-select-auto>
    ///
    /// Memoized along the walk, so the elements of a deep tree take linear
    /// time between them.
    fn unselectable(&self, element: NodeId) -> bool {
        let mut path = Vec::new();
        let mut node = Some(element);
        let unselectable = loop {
            let Some(current) = node else {
                break false;
            };
            if let Some(&known) = self.unselectable.borrow().get(&current) {
                break known;
            }
            path.push(current);
            let data = self.tree.node(current);
            match data.primary_styles().map(|s| s.clone_user_select()) {
                Some(UserSelect::None) => break true,
                Some(UserSelect::Auto) | None => node = data.flat_parent_id(),
                Some(_) => break false,
            }
        };
        let mut known = self.unselectable.borrow_mut();
        for node in path {
            known.insert(node, unselectable);
        }
        unselectable
    }

    /// A box's laid-out size; `None` for a node without a box (Text).
    pub(super) fn size(&self, node: NodeId) -> Option<taffy::Size<f32>> {
        let data = self.tree.node(node);
        matches!(
            data.data,
            NodeData::Element(_) | NodeData::AnonymousBlock(_) | NodeData::Document(_)
        )
        .then(|| data.final_layout().size)
    }

    /// A box's height, Blink's `LogicalHeight` in horizontal writing.
    pub(super) fn height(&self, node: NodeId) -> f32 {
        self.size(node).map_or(0.0, |size| size.height)
    }

    /// The formatting context laid out at an inline root.
    pub(super) fn ifc(&self, root: NodeId) -> Option<Rc<Ifc>> {
        if let Some(found) = self.ifcs.borrow().get(&root) {
            return found.clone();
        }
        let built = ifc::build(self.doc(), root).map(Rc::new);
        self.ifcs.borrow_mut().insert(root, built.clone());
        built
    }

    /// The inline formatting context a node takes part in, as text, an
    /// inline box, an atomic inline, a float or a positioned box.
    pub(super) fn ifc_of(&self, node: NodeId) -> Option<Rc<Ifc>> {
        let root = self.inline_root_of(node)?;
        self.ifc(root).filter(|ifc| ifc.has_member(node))
    }

    /// The nearest laid-out inline root above a node in the layout tree,
    /// memoized along the walk like [`Self::unselectable`].
    fn inline_root_of(&self, node: NodeId) -> Option<NodeId> {
        let doc = self.doc();
        let mut path = vec![node];
        let mut current = doc.get_node(node).and_then(|n| n.layout_parent.get());
        let root = loop {
            let Some(id) = current else {
                break None;
            };
            // The memo holds the root strictly above a node, so a node that
            // is itself a root answers for the nodes below it.
            if let Some(&known) = self.inline_roots.borrow().get(&id) {
                break Some(id)
                    .filter(|&id| is_laid_out_inline_root(doc, id))
                    .or(known);
            }
            if is_laid_out_inline_root(doc, id) {
                break Some(id);
            }
            path.push(id);
            current = doc.get_node(id).and_then(|n| n.layout_parent.get());
        };
        let mut known = self.inline_roots.borrow_mut();
        for node in path {
            known.insert(node, root);
        }
        root
    }

    /// The element whose box a formatting context lays out: its root, or
    /// the element around a root that is the element's sole anonymous
    /// block (a button's content).
    pub(super) fn ifc_owner(&self, root: NodeId) -> NodeId {
        let doc = self.doc();
        let Some(data) = doc.get_node(root).filter(|n| n.is_anonymous()) else {
            return root;
        };
        data.layout_parent
            .get()
            .filter(|&parent| crate::caret::order::flattened_root(doc, parent) == Some(root))
            .unwrap_or(root)
    }

    /// The inline formatting context an inline root's own content forms,
    /// for an atomic inline or a block: itself, or its sole anonymous block.
    pub(super) fn own_ifc(&self, node: NodeId) -> Option<Rc<Ifc>> {
        let root = crate::caret::order::flattened_root(self.doc(), node)?;
        self.ifc(root)
    }

    pub(super) fn next_grapheme_boundary(&self, node: NodeId, offset: usize) -> usize {
        if !self.tree.is_text(node) {
            return offset + 1;
        }
        let boundaries = self.graphemes(node);
        let index = boundaries.partition_point(|&b| b <= offset);
        boundaries
            .get(index)
            .copied()
            .unwrap_or_else(|| self.tree.text_length(node))
    }

    pub(super) fn previous_grapheme_boundary(&self, node: NodeId, offset: usize) -> usize {
        if !self.tree.is_text(node) {
            return offset.saturating_sub(1);
        }
        let boundaries = self.graphemes(node);
        let index = boundaries.partition_point(|&b| b < offset);
        index
            .checked_sub(1)
            .and_then(|index| boundaries.get(index))
            .copied()
            .unwrap_or(0)
    }

    fn graphemes(&self, node: NodeId) -> Rc<Vec<usize>> {
        if let Some(found) = self.graphemes.borrow().get(&node) {
            return found.clone();
        }
        let units = self.tree.text_units(node);
        let boundaries: Rc<Vec<usize>> = Rc::new(
            GraphemeClusterSegmenter::new()
                .segment_utf16(&units)
                .collect(),
        );
        self.graphemes.borrow_mut().insert(node, boundaries.clone());
        boundaries
    }
}

/// A replaced element or control, which lays out no children of its own.
/// A `<button>` is atomic in its line but lays its children out, whatever
/// its `display`; an input button's label is not page content.
fn is_replaced(element: &crate::node::ElementData) -> bool {
    inline_element(element) == InlineElement::Atom && element.name.local != local_name!("button")
}

fn is_laid_out_inline_root(doc: &BaseDocument, node: NodeId) -> bool {
    doc.get_node(node).is_some_and(|data| {
        data.flags.is_inline_root()
            && data
                .element_data()
                .is_some_and(|e| e.inline_layout_data.is_some())
    })
}

#[cfg(test)]
#[path = "context_tests.rs"]
mod tests;
