//! Custom element support in the DOM: element state (behind `:defined`) and
//! the reactions an embedder's registry runs.
//!
//! The registry itself (definitions, constructors, upgrades) lives with the
//! embedder's script engine. The DOM tracks which elements could be custom
//! elements and, while reactions are recorded, reports when they are
//! connected, disconnected or have attributes changed.

use crate::{BaseDocument, NodeId};

/// An element's custom element state, per the DOM standard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CustomElementState {
    /// A built-in element (and custom elements' `:defined` twin).
    #[default]
    Uncustomized,
    /// A valid custom element name with no definition (yet).
    Undefined,
    /// Its constructor threw during an upgrade.
    Failed,
    /// Upgraded or constructed from its definition.
    Custom,
}

impl CustomElementState {
    /// The state of a new element: undefined for a valid custom element name
    /// in the HTML namespace, uncustomized otherwise.
    pub fn initial(name: &markup5ever::QualName) -> Self {
        if name.ns == markup5ever::ns!(html) && is_valid_custom_element_name(&name.local) {
            Self::Undefined
        } else {
            Self::Uncustomized
        }
    }

    /// Whether `:defined` matches.
    pub fn is_defined(self) -> bool {
        matches!(self, Self::Uncustomized | Self::Custom)
    }
}

/// Something a custom element's definition must be told about.
#[derive(Debug, Clone, PartialEq)]
pub enum CustomElementReaction {
    /// The element became connected (inserted into the document).
    Connected(NodeId),
    /// The element stopped being connected.
    Disconnected(NodeId),
    /// An attribute of a custom (not merely undefined) element changed.
    AttributeChanged {
        element: NodeId,
        name: String,
        namespace: Option<String>,
        old_value: Option<crate::DomString>,
        new_value: Option<crate::DomString>,
    },
}

/// Whether `name` is a valid custom element name: starts with an ASCII lower
/// case letter, contains a hyphen, has no ASCII upper case letters, and is
/// not one of the reserved names.
pub fn is_valid_custom_element_name(name: &str) -> bool {
    const RESERVED: &[&str] = &[
        "annotation-xml",
        "color-profile",
        "font-face",
        "font-face-src",
        "font-face-uri",
        "font-face-format",
        "font-face-name",
        "missing-glyph",
    ];
    name.starts_with(|c: char| c.is_ascii_lowercase())
        && name.contains('-')
        && !name.chars().any(|c| c.is_ascii_uppercase())
        && name.chars().all(|c| {
            matches!(c, '-' | '.' | '_' | '0'..='9' | 'a'..='z' | '\u{B7}')
                || matches!(c as u32, 0xC0..=0xD6 | 0xD8..=0xF6 | 0xF8..=0x37D | 0x37F..=0x1FFF | 0x200C..=0x200D
                    | 0x203F..=0x2040 | 0x2070..=0x218F | 0x2C00..=0x2FEF | 0x3001..=0xD7FF | 0xF900..=0xFDCF
                    | 0xFDF0..=0xFFFD | 0x10000..=0xEFFFF)
        })
        && !RESERVED.contains(&name)
}

impl BaseDocument {
    /// Start or stop recording custom element reactions. Stopping discards
    /// pending ones.
    pub fn set_custom_element_reactions(&mut self, on: bool) {
        if !on {
            self.custom_element_reaction_bytes = 0;
            self.custom_element_reaction_overflow = false;
        }
        self.custom_element_reactions =
            on.then(|| self.custom_element_reactions.take().unwrap_or_default());
    }

    /// Reactions since the last call, oldest first.
    pub fn take_custom_element_reactions(&mut self) -> Vec<CustomElementReaction> {
        self.custom_element_reaction_bytes = 0;
        match &mut self.custom_element_reactions {
            Some(reactions) => std::mem::take(reactions),
            None => Vec::new(),
        }
    }

    pub fn custom_element_state(&self, node_id: NodeId) -> CustomElementState {
        self.nodes
            .get(node_id)
            .and_then(|node| node.element_data())
            .map_or(CustomElementState::Uncustomized, |el| {
                el.custom_element_state
            })
    }

    /// Set an element's state (the embedder does this when it upgrades or
    /// constructs it), restyling it for `:defined`.
    pub fn set_custom_element_state(&mut self, node_id: NodeId, state: CustomElementState) {
        let Some(element) = self.nodes[node_id].element_data_mut() else {
            return;
        };
        if element.custom_element_state == state {
            return;
        }
        element.custom_element_state = state;
        if self.nodes[node_id].flags.is_in_document() {
            self.restyle_subtree_of(node_id);
        }
    }

    pub(crate) fn record_custom_element_reaction(&mut self, reaction: CustomElementReaction) {
        if let Some(reactions) = &mut self.custom_element_reactions {
            let bytes = match &reaction {
                CustomElementReaction::AttributeChanged {
                    name,
                    namespace,
                    old_value,
                    new_value,
                    ..
                } => {
                    name.len()
                        + namespace.as_ref().map_or(0, String::len)
                        + old_value
                            .as_ref()
                            .map_or(0, crate::DomString::retained_bytes)
                        + new_value
                            .as_ref()
                            .map_or(0, crate::DomString::retained_bytes)
                }
                _ => 0,
            };
            if reactions.len() >= 4096
                || bytes > (8 * 1024 * 1024usize).saturating_sub(self.custom_element_reaction_bytes)
            {
                self.custom_element_reaction_overflow = true;
                return;
            }
            self.custom_element_reaction_bytes += bytes;
            reactions.push(reaction);
        }
    }

    /// Consume the overflow diagnostic; a bounded queue is never silently complete.
    pub fn take_custom_element_reaction_overflow(&mut self) -> bool {
        std::mem::take(&mut self.custom_element_reaction_overflow)
    }

    pub(crate) fn is_recording_custom_element_reactions(&self) -> bool {
        self.custom_element_reactions.is_some()
    }
}

#[cfg(test)]
mod bounds_tests {
    use super::*;
    #[test]
    fn reaction_count_and_retained_bytes_are_bounded() {
        let mut doc = BaseDocument::new(crate::DocumentConfig::default());
        doc.set_custom_element_reactions(true);
        let id = doc.root_node().id;
        for _ in 0..5000 {
            doc.record_custom_element_reaction(CustomElementReaction::Connected(id));
        }
        assert_eq!(doc.take_custom_element_reactions().len(), 4096);
        assert!(doc.take_custom_element_reaction_overflow());
        assert!(!doc.take_custom_element_reaction_overflow());
        doc.record_custom_element_reaction(CustomElementReaction::AttributeChanged {
            element: id,
            name: "x".into(),
            namespace: None,
            old_value: None,
            new_value: Some("x".repeat(8 * 1024 * 1024).into()),
        });
        assert!(doc.take_custom_element_reactions().is_empty());
        assert!(doc.take_custom_element_reaction_overflow());
        doc.record_custom_element_reaction(CustomElementReaction::Connected(id));
        assert_eq!(doc.take_custom_element_reactions().len(), 1);
    }
}
