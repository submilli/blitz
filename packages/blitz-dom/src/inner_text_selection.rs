//! DOM endpoints projected through the same rendered-text rules as innerText.
use super::{RenderedText, Visit};
use crate::{BaseDocument, DomString, NodeId, ranges::Boundary};
use std::cmp::Ordering;
use style::computed_values::visibility::T as Visibility;
use style::values::computed::UserSelect;

impl BaseDocument {
    /// Render a DOM selection after styles/layout have been resolved. Range
    /// stringification deliberately remains raw DOM character data.
    pub fn dom_selection_text(&self, start: Boundary, end: Boundary) -> DomString {
        if start.compare(self, end) != Some(Ordering::Less) {
            return DomString::new();
        }
        let mut output = RenderedText::default();
        self.selection_prefix_context(start, &mut output);
        let mut pending = Vec::new();
        self.enqueue_inner_text_children(self.tree_root(start.node), &mut pending);
        while let Some(visit) = pending.pop() {
            match visit {
                Visit::Node(id) => {
                    if self.selection_intersects(id, start, end) && self.selection_node_visible(id)
                    {
                        self.collect_inner_text_node(
                            id,
                            &mut pending,
                            &mut output,
                            Some([start, end]),
                        );
                    }
                }
                Visit::EndAtomic => {
                    output.space = false;
                    output.line_start = false;
                }
                Visit::Break(count) => output.require_break(count),
                Visit::Tab => {
                    output.space = false;
                    output.flush_breaks();
                    output.units.push(9);
                }
            }
        }
        self.selection_trailing_space(end, &mut output);
        DomString::from_utf16(output.units)
    }

    // A sliced Text node still participates in its original whitespace run.
    // Preserve layout context without including unselected prefix/suffix text.
    fn selection_prefix_context(&self, point: Boundary, output: &mut RenderedText) {
        let node = &self.nodes[point.node];
        let Some(text) = node.text_data() else {
            return;
        };
        let Some(style) = node.parent.and_then(|id| self.nodes[id].primary_styles()) else {
            return;
        };
        let units = text.content.to_utf16();
        let prefix = DomString::from_utf16(units[..point.offset.min(units.len())].to_vec());
        output.text(
            &prefix,
            style.clone_white_space_collapse(),
            style.clone_text_transform(),
            style.clone__x_lang().0.as_ref(),
        );
        output.units.clear();
        output.space = false;
        output.required_breaks = 0;
    }

    fn selection_trailing_space(&self, point: Boundary, output: &mut RenderedText) {
        if !output.space {
            return;
        }
        let node = &self.nodes[point.node];
        let Some(text) = node.text_data() else {
            return;
        };
        let Some(style) = node.parent.and_then(|id| self.nodes[id].primary_styles()) else {
            return;
        };
        let units = text.content.to_utf16();
        let suffix = DomString::from_utf16(units[point.offset.min(units.len())..].to_vec());
        let mut continued = output.clone();
        continued.text(
            &suffix,
            style.clone_white_space_collapse(),
            style.clone_text_transform(),
            style.clone__x_lang().0.as_ref(),
        );
        if continued.units.get(output.units.len()) == Some(&32) {
            output.units.push(32);
        }
    }

    /// Selection containment uses visible endpoints and the target's outside
    /// boundaries. Overlapping Text nodes count even for full containment.
    /// <https://w3c.github.io/selection-api/#dom-selection-containsnode>
    pub fn dom_selection_contains(
        &self,
        start: Boundary,
        end: Boundary,
        target: NodeId,
        partial: bool,
    ) -> bool {
        let Some([before, after]) = self.selection_outside(target) else {
            return false;
        };
        let Some([start, end]) = self.visible_selection_points(start, end) else {
            return false;
        };
        let starts_before = start
            .compare(self, before)
            .is_some_and(|o| o != Ordering::Greater);
        let ends_after = end
            .compare(self, after)
            .is_some_and(|o| o != Ordering::Less);
        if starts_before && ends_after {
            return true;
        }
        let overlap = start.compare(self, after) == Some(Ordering::Less)
            && end.compare(self, before) == Some(Ordering::Greater);
        overlap && (partial || self.nodes[target].is_text_node())
    }

    fn selection_outside(&self, id: NodeId) -> Option<[Boundary; 2]> {
        let parent = self.get_node(id)?.parent?;
        let index = self.nodes[parent].children.position(id)?;
        Some([
            Boundary {
                node: parent,
                offset: index,
            },
            Boundary {
                node: parent,
                offset: index + 1,
            },
        ])
    }

    fn selection_intersects(&self, id: NodeId, start: Boundary, end: Boundary) -> bool {
        self.selection_outside(id).is_some_and(|[before, after]| {
            start.compare(self, after) == Some(Ordering::Less)
                && end.compare(self, before) == Some(Ordering::Greater)
        })
    }

    fn selection_node_visible(&self, id: NodeId) -> bool {
        if !self.inner_text_is_rendered(id) {
            return false;
        }
        let node = &self.nodes[id];
        let style = node
            .primary_styles()
            .or_else(|| node.parent.and_then(|p| self.nodes[p].primary_styles()));
        style.is_some_and(|s| s.clone_user_select() != UserSelect::None)
    }

    pub(super) fn selected_text_slice(
        &self,
        id: NodeId,
        content: &DomString,
        points: [Boundary; 2],
    ) -> DomString {
        let units = content.to_utf16();
        let start = if points[0].node == id {
            points[0].offset.min(units.len())
        } else {
            0
        };
        let end = if points[1].node == id {
            points[1].offset.min(units.len())
        } else {
            units.len()
        };
        DomString::from_utf16(units[start.min(end)..end].to_vec())
    }

    fn visible_selection_points(&self, start: Boundary, end: Boundary) -> Option<[Boundary; 2]> {
        let root = self.tree_root(start.node);
        let mut pending = vec![root];
        let mut first = None;
        let mut last = None;
        while let Some(id) = pending.pop() {
            let node = &self.nodes[id];
            if id == root {
                pending.extend(node.children.iter().rev().copied());
                continue;
            }
            if !self.selection_node_visible(id) {
                continue;
            }
            if let Some(text) = node.text_data() {
                let style = node.parent.and_then(|p| self.nodes[p].primary_styles())?;
                if style.clone_visibility() != Visibility::Visible {
                    continue;
                }
                let length = text.content.to_utf16().len();
                if length == 0 {
                    continue;
                }
                let before = Boundary {
                    node: id,
                    offset: 0,
                };
                let after = Boundary {
                    node: id,
                    offset: length,
                };
                if start
                    .compare(self, after)
                    .is_some_and(|o| o != Ordering::Greater)
                    && end
                        .compare(self, before)
                        .is_some_and(|o| o != Ordering::Less)
                {
                    let from = if start.node == id { start } else { before };
                    let to = if end.node == id { end } else { after };
                    first.get_or_insert(from);
                    last = Some(to);
                }
            } else if !self.inner_text_replaced(id) {
                pending.extend(node.children.iter().rev().copied());
            }
        }
        if let (Some(first), Some(last)) = (first, last) {
            return Some([first, last]);
        }
        self.inner_text_is_rendered(start.node)
            .then_some([start, end])
    }
}
