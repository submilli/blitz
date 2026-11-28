//! HTML editing state shared by the DOM bindings and CSS UI selectors.
use crate::{BaseDocument, ElementData, Node, NodeData, NodeId, local_name, ns};
use std::sync::atomic::Ordering;
use style_dom::ElementState;

const DESIGN_MODE: u64 = 1;
const ATTRIBUTE: u64 = 2;
const FLAGS: u64 = DESIGN_MODE | ATTRIBUTE;
pub(crate) const GENERATION_STEP: u64 = FLAGS + 1;

impl BaseDocument {
    pub fn design_mode(&self, document: NodeId) -> bool {
        self.get_node(document)
            .is_some_and(|node| matches!(&node.data, NodeData::Document(data) if data.design_mode))
    }

    pub fn set_design_mode(&mut self, document: NodeId, enabled: bool) {
        let Some(node) = self.get_node_mut(document) else {
            return;
        };
        let NodeData::Document(data) = &mut node.data else {
            return;
        };
        if data.design_mode == enabled {
            return;
        }
        data.design_mode = enabled;
        if let Some(root) = self.document_element_node(document).map(|root| root.id) {
            self.restyle_subtree_of(root);
        }
    }
}

impl ElementData {
    pub(crate) fn content_editable_keyword(&self) -> Option<bool> {
        let value = self.attr(local_name!("contenteditable"))?;
        if value.is_empty()
            || value.eq_ignore_ascii_case("true")
            || value.eq_ignore_ascii_case("plaintext-only")
        {
            Some(true)
        } else if value.eq_ignore_ascii_case("false") {
            Some(false)
        } else {
            None
        }
    }
}

impl Node {
    /// Memoize the inherited states separately: only designMode crosses a shadow root.
    /// Mutable arena access invalidates the generation. During read-only queries and
    /// style traversal each ancestor path ends at the nearest already resolved node.
    pub fn is_content_editable(&self) -> bool {
        let generation = self.tree().editing_generation;
        let mut path = smallvec::SmallVec::<[&Node; 16]>::new();
        let mut current = self;
        let mut flags = loop {
            let cached = current.editing_cache.load(Ordering::Relaxed);
            if cached & !FLAGS == generation {
                break cached & FLAGS;
            }
            path.push(current);
            if let NodeData::Document(data) = &current.data {
                break if data.design_mode { DESIGN_MODE } else { 0 };
            }
            let Some(parent) = current
                .parent
                .or_else(|| current.shadow_root_data.as_ref().map(|root| root.host))
            else {
                break 0;
            };
            current = self.with(parent);
        };
        for node in path.into_iter().rev() {
            if node.shadow_root_data.is_some() {
                flags &= !ATTRIBUTE;
            } else if let Some(enabled) = node
                .element_data()
                .filter(|element| element.name.ns == ns!(html))
                .and_then(ElementData::content_editable_keyword)
            {
                if enabled {
                    flags |= ATTRIBUTE
                } else {
                    flags &= !ATTRIBUTE
                }
            }
            node.editing_cache
                .store(generation | flags, Ordering::Relaxed);
        }
        flags != 0
    }

    pub(crate) fn effective_element_state(&self) -> ElementState {
        let state = self.control_state();
        state | self.editing_state(state.contains(ElementState::DISABLED))
    }

    fn editing_state(&self, disabled: bool) -> ElementState {
        let Some(element) = self.element_data().filter(|e| e.name.ns == ns!(html)) else {
            return ElementState::empty();
        };
        let writable = match element.name.local.as_ref() {
            "input" | "textarea" => {
                let kind = element.attr(local_name!("type")).unwrap_or("");
                let accepts_text = element.name.local == local_name!("textarea")
                    || ![
                        "hidden", "range", "color", "checkbox", "radio", "file", "submit", "image",
                        "reset", "button",
                    ]
                    .iter()
                    .any(|token| kind.eq_ignore_ascii_case(token));
                accepts_text && !element.has_attr(local_name!("readonly")) && !disabled
            }
            _ => self.is_content_editable(),
        };
        if writable {
            ElementState::READWRITE
        } else {
            ElementState::READONLY
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DocumentConfig, QualName};

    #[test]
    fn pointer_snapshots_preserve_cached_ancestry() {
        let mut doc = BaseDocument::new(DocumentConfig::default());
        let element = doc
            .mutate()
            .try_create_element(
                QualName::new(None, ns!(html), local_name!("div")),
                Vec::new(),
            )
            .unwrap();
        let generation = doc.nodes.editing_generation;
        assert!(!doc.nodes[element].is_content_editable());
        doc.nodes.mark_pending_snapshot(element);
        doc.nodes
            .set_pointer_state(element, ElementState::HOVER, true);
        assert_eq!(doc.nodes.editing_generation, generation);
        assert!(doc.nodes[element].is_hovered());
        doc.mutate().set_attribute(
            element,
            QualName::new(None, ns!(), local_name!("contenteditable")),
            "true",
        );
        assert!(doc.nodes[element].is_content_editable());
    }
}

// The Wasm guest budgets DOM arena slots across the full document capacity.
// Editability shares the existing stable identity allocation to preserve this bound.
#[cfg(target_arch = "wasm32")]
const _: () = assert!(std::mem::size_of::<Node>() <= 360);
