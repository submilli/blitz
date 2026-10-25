//! Style type admission and disassociation shared by active and inert trees.
use super::*;
use crate::node::SpecialElementData;

impl BaseDocument {
    pub(crate) fn style_element_accepts_css(&self, owner: NodeId) -> bool {
        // HTML's style block algorithm accepts only an empty type or an exact
        // ASCII case-insensitive text/css token; whitespace is significant.
        self.null_attribute(owner, "type").is_none_or(|attribute| {
            let value = attribute.value.as_str_lossy();
            value.is_empty() || value.eq_ignore_ascii_case("text/css")
        })
    }

    pub(crate) fn clear_owner_stylesheet(&mut self, owner: NodeId) {
        let retained = self
            .nodes
            .get_mut(owner)
            .and_then(|node| node.element_data_mut())
            .is_some_and(|element| {
                if matches!(element.special_data, SpecialElementData::Stylesheet(_)) {
                    element.special_data.take();
                    true
                } else {
                    false
                }
            });
        self.remove_shadow_stylesheet(owner);
        let attached = self.nodes_to_stylesheet.remove(&owner);
        let changed = retained || attached.is_some();
        if let Some(sheet) = attached {
            let guard = self.guard.read();
            self.stylist.remove_stylesheet(sheet, &guard);
            self.stylist
                .force_stylesheet_origins_dirty(style::stylesheets::OriginSet::all());
        }
        if changed {
            self.stylesheet_generation += 1;
        }
    }
}
