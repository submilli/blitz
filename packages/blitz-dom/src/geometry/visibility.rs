//! Conservative visibility evidence from resolved effects and paint topology.
//! No pointer hit testing: pointer-events does not change occlusion.
use crate::{BaseDocument, BoundingRect, Node, NodeId};
use euclid::default::{Rect, Size2D};
use kurbo::Affine;
use style::computed_values::visibility::T as Visibility;
use style::values::computed::{CSSPixelLength, Rotate};
use style::values::generics::transform::{Scale, Translate};

/// Shared work admission for one rendering update, across all observers/targets.
/// Exhaustion means visibility cannot be guaranteed; it never means visible.
pub struct VisibilityBudget {
    remaining: usize,
}
impl Default for VisibilityBudget {
    fn default() -> Self {
        Self::new(262_144)
    }
}
impl VisibilityBudget {
    /// A caller may choose a smaller budget, but cannot exceed the engine ceiling.
    pub fn new(units: usize) -> Self {
        Self {
            remaining: units.min(262_144),
        }
    }
    fn reserve(&mut self, units: usize) -> bool {
        if units >= self.remaining {
            self.remaining = 0;
            return false;
        }
        self.remaining -= units;
        true
    }
    fn admit(&mut self) -> bool {
        if self.remaining == 0 {
            return false;
        }
        self.remaining -= 1;
        true
    }
}

impl BaseDocument {
    /// Intersection Observer visibility after layout. The caller supplies the
    /// clipped intersection, and shares admission across the whole update.
    /// https://w3c.github.io/IntersectionObserver/#visibility
    pub fn intersection_observer_visibility(
        &self,
        target: NodeId,
        hit: BoundingRect,
        budget: &mut VisibilityBudget,
    ) -> bool {
        if ![hit.x, hit.y, hit.width, hit.height]
            .into_iter()
            .all(f64::is_finite)
            || hit.width <= 0.0
            || hit.height <= 0.0
            || !self.is_connected(target)
            || !self.visibility_box_is_rendered(target, budget)
        {
            return false;
        }
        if !self.observer_effects_allow_visibility(target, budget) {
            return false;
        }
        let Some(root) = self.try_root_element() else {
            return false;
        };
        // Non-atomic inline boxes participate through their inline root's runs.
        let Some(paint_target) = self.visibility_inline_root(target, budget) else {
            return false;
        };
        let paint_target = paint_target.unwrap_or(target);
        let mut stack = vec![root.id];
        let mut found = false;
        while let Some(id) = stack.pop() {
            if !budget.admit() {
                return false;
            }
            let Some(node) = self.get_node(id) else {
                return false;
            };
            if node
                .primary_styles()
                .is_some_and(|s| s.clone_opacity() == 0.0)
                || !self.visibility_box_is_rendered(id, budget)
            {
                continue;
            }
            if id == paint_target {
                found = true;
            }
            if found && id != paint_target {
                let Some(related) = self.visibility_descendant_of(id, target, budget) else {
                    return false;
                };
                if !related && self.observer_ink_may_occlude(node, hit, budget) {
                    return false;
                }
            }
            // Reverse push gives the painter's background/negative/normal/positive
            // ordering. Hoisted children retain the owning stacking context's order.
            let count = node.paint_children.borrow().as_ref().map_or(0, |c| c.len())
                + node
                    .stacking_context
                    .as_ref()
                    .map_or(0, |sc| sc.children.len());
            if count > budget.remaining || stack.len() + count > 16_384 {
                return false;
            }
            if let Some(sc) = &node.stacking_context {
                stack.extend(sc.pos_z_hoisted_children().rev().map(|c| c.node_id));
            }
            stack.extend(node.paint_children.borrow().iter().flatten().rev().copied());
            if let Some(sc) = &node.stacking_context {
                stack.extend(sc.neg_z_hoisted_children().rev().map(|c| c.node_id));
            }
        }
        found && budget.remaining > 0
    }

    fn observer_effects_allow_visibility(
        &self,
        target: NodeId,
        budget: &mut VisibilityBudget,
    ) -> bool {
        let mut current = Some(target);
        let mut matrix = Affine::IDENTITY;
        while let Some(id) = current {
            if !budget.admit() {
                return false;
            }
            let Some(node) = self.get_node(id) else {
                return false;
            };
            if let Some(style) = node.primary_styles() {
                if style.clone_opacity() != 1.0
                    || (id == target && style.clone_visibility() != Visibility::Visible)
                    || !style.get_effects().filter.0.is_empty()
                    || !self.visibility_transform_is_2d(node, budget)
                    || matches!(
                        style.clone_clip_path(),
                        style::values::computed::basic_shape::ClipPath::Url(_)
                    )
                {
                    return false;
                }
            }
            if let Some(transform) = node.transform().as_deref() {
                matrix = *transform * matrix;
            }
            current = node.flat_parent_id();
        }
        let [a, b, c, d, _, _] = matrix.as_coeffs();
        a.is_finite() && d.is_finite() && a >= 1.0 && a == d && b == 0.0 && c == 0.0
    }

    fn visibility_box_is_rendered(&self, id: NodeId, budget: &mut VisibilityBudget) -> bool {
        let mut current = Some(id);
        while let Some(id) = current {
            if !budget.admit() {
                return false;
            }
            let Some(node) = self.get_node(id) else {
                return false;
            };
            if node.display_style().is_some_and(|d| d.is_none()) {
                return false;
            }
            current = node.flat_parent_id();
        }
        self.get_node(id).is_some_and(|n| n.has_boxes())
    }

    fn visibility_transform_is_2d(&self, node: &Node, budget: &mut VisibilityBudget) -> bool {
        let Some(style) = node.primary_styles() else {
            return true;
        };
        let box_style = style.get_box();
        if matches!(box_style.rotate, Rotate::Rotate3D(x, y, _, _) if x != 0.0 || y != 0.0)
            || matches!(box_style.scale, Scale::Scale(_, _, z) if z != 1.0)
            || matches!(box_style.translate, Translate::Translate(_, _, z) if z.px() != 0.0)
        {
            return false;
        }
        if box_style.transform.0.is_empty() {
            return true;
        }
        for _ in box_style.transform.0.iter() {
            if !budget.admit() {
                return false;
            }
        }
        let size = node.unrounded_layout().size;
        let reference = Rect::from_size(Size2D::new(
            CSSPixelLength::new(size.width),
            CSSPixelLength::new(size.height),
        ));
        box_style
            .transform
            .to_transform_3d_matrix(Some(&reference))
            .is_ok_and(|(m, _)| m.is_2d())
    }

    fn visibility_descendant_of(
        &self,
        id: NodeId,
        ancestor: NodeId,
        budget: &mut VisibilityBudget,
    ) -> Option<bool> {
        let mut current = Some(id);
        while let Some(id) = current {
            if !budget.admit() {
                return None;
            }
            if id == ancestor {
                return Some(true);
            }
            current = self.get_node(id)?.flat_parent_id();
        }
        Some(false)
    }

    fn visibility_inline_root(
        &self,
        id: NodeId,
        budget: &mut VisibilityBudget,
    ) -> Option<Option<NodeId>> {
        let mut current = Some(id);
        while let Some(id) = current {
            if !budget.admit() {
                return None;
            }
            let node = self.get_node(id)?;
            if node.flags.is_inline_root() {
                return Some(Some(id));
            }
            current = node.layout_parent.get();
        }
        Some(None)
    }

    fn visibility_geometry_depth(
        &self,
        id: NodeId,
        budget: &mut VisibilityBudget,
    ) -> Option<usize> {
        let mut current = Some(id);
        let mut flat_depth = 0;
        while let Some(id) = current {
            if !budget.admit() {
                return None;
            }
            if let Some(style) = self.get_node(id)?.primary_styles()
                && let style::values::computed::basic_shape::ClipPath::Shape(shape, _) =
                    style.clone_clip_path()
                && !budget.reserve(
                    crate::basic_shape_work(&shape)
                        .saturating_add(4096)
                        .saturating_mul(4),
                )
            {
                return None;
            }
            flat_depth += 1;
            current = self.get_node(id)?.flat_parent_id();
        }
        let mut current = Some(id);
        let mut box_depth = 0;
        while let Some(id) = current {
            if !budget.admit() {
                return None;
            }
            box_depth += 1;
            current = self.get_node(id)?.containing_block();
        }
        Some(flat_depth.max(box_depth))
    }

    fn observer_ink_may_occlude(
        &self,
        node: &Node,
        hit: BoundingRect,
        budget: &mut VisibilityBudget,
    ) -> bool {
        let Some(style) = node.primary_styles() else {
            return false;
        };
        if style.clone_visibility() != Visibility::Visible {
            return false;
        }
        // Ink-expanding effects cannot be certified from border-box geometry.
        // Failing closed is explicitly permitted by the visibility algorithm.
        if !style.get_effects().filter.0.is_empty() || !style.get_effects().box_shadow.0.is_empty()
        {
            return true;
        }
        // Chrome treats overlapping painted boxes as possible occluders even
        // with a fully transparent background. Only zero-opacity subtrees are
        // omitted above; pointer-events and background alpha grant no evidence.
        // Account for containing clips with the same engine geometry as intersections.
        // Text-fragment enumeration is not a box walk. Without separately
        // admitted fragment evidence we cannot guarantee an inline occluder clear.
        let Some(inline_root) = self.visibility_inline_root(node.id, budget) else {
            return true;
        };
        if inline_root.is_some_and(|id| id != node.id) {
            return true;
        }
        let Some(depth) = self.visibility_geometry_depth(node.id, budget) else {
            return true;
        };
        // A clip at each ancestor may project through every transform, each of
        // which resolves its absolute origin. Reserve a cubic upper bound on
        // those nested walks (including flat-tree eligibility and new inset clips)
        // before entering ordinary geometry. No uncharged fragment allocation runs.
        let units = depth.saturating_add(1).saturating_pow(3).saturating_mul(16);
        if !budget.reserve(units) {
            return true;
        }
        let geometry = self.intersection_observer_geometry(node.id, None, [(0.0, false); 4]);
        let rect = geometry.hit;
        rect.width > 0.0
            && rect.height > 0.0
            && rect.x < hit.x + hit.width
            && rect.x + rect.width > hit.x
            && rect.y < hit.y + hit.height
            && rect.y + rect.height > hit.y
    }
}
