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
    /// Running an upgrade constructor after super() has returned.
    Precustomized,
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
    FormAssociated {
        element: NodeId,
        form: Option<NodeId>,
    },
    FormDisabled {
        element: NodeId,
        disabled: bool,
    },
    FormReset(NodeId),
    /// The node document changed.
    Adopted {
        element: NodeId,
        old_document: NodeId,
        new_document: NodeId,
    },
    /// An attribute of a custom (not merely undefined) element changed.
    AttributeChanged {
        element: NodeId,
        name: String,
        namespace: Option<String>,
        old_value: Option<crate::DomString>,
        new_value: Option<crate::DomString>,
    },
}

impl CustomElementReaction {
    pub fn target(&self) -> NodeId {
        match self {
            Self::Connected(id) | Self::Disconnected(id) | Self::FormReset(id) => *id,
            Self::FormAssociated { element, .. }
            | Self::FormDisabled { element, .. }
            | Self::Adopted { element, .. }
            | Self::AttributeChanged { element, .. } => *element,
        }
    }

    /// All nodes retained by a queued or transferred reaction, including callback arguments.
    pub fn referenced_nodes(&self) -> impl Iterator<Item = NodeId> {
        let (first, second) = match self {
            Self::Adopted {
                old_document,
                new_document,
                ..
            } => (Some(*old_document), Some(*new_document)),
            Self::FormAssociated { form, .. } => (*form, None),
            _ => (None, None),
        };
        [Some(self.target()), first, second].into_iter().flatten()
    }
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
        // Live form collections depend on upgrade state as well as topology.
        self.dom_generation = self.dom_generation.wrapping_add(1);
        if self.nodes[node_id].flags.is_in_document() {
            self.restyle_subtree_of(node_id);
        }
    }

    /// Queue the pre-construction lifecycle snapshot for an upgrade.
    pub fn upgrade_reactions(&mut self, id: NodeId) -> Vec<CustomElementReaction> {
        let mut reactions = Vec::new();
        let mut bytes = 0usize;
        if let Some(element) = self.nodes[id].element_data() {
            for attr in element.attrs().iter() {
                let retained =
                    attr.name.local.len() + attr.name.ns.len() + attr.value.retained_bytes();
                if reactions.len() >= 4096
                    || retained > (8 * 1024 * 1024usize).saturating_sub(bytes)
                {
                    self.custom_element_reaction_overflow = true;
                    break;
                }
                bytes += retained;
                reactions.push(CustomElementReaction::AttributeChanged {
                    element: id,
                    name: attr.name.local.to_string(),
                    namespace: (attr.name.ns != markup5ever::ns!())
                        .then(|| attr.name.ns.to_string()),
                    old_value: None,
                    new_value: Some(attr.value.clone()),
                });
            }
        }
        if self.is_connected(id) {
            if reactions.len() < 4096 {
                reactions.push(CustomElementReaction::Connected(id));
            } else {
                self.custom_element_reaction_overflow = true;
            }
        }
        reactions
    }

    pub fn enqueue_custom_element_reactions(&mut self, reactions: Vec<CustomElementReaction>) {
        for reaction in reactions {
            self.record_custom_element_reaction(reaction);
        }
    }

    /// A failed upgrade discards reactions remaining for that element.
    pub fn discard_custom_element_reactions(&mut self, id: NodeId) {
        if let Some(reactions) = &mut self.custom_element_reactions {
            reactions.retain(|reaction| reaction.target() != id);
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
    fn upgrade_snapshot_precedes_constructor_mutations() {
        let mut doc = BaseDocument::new(crate::DocumentConfig::default());
        doc.set_custom_element_reactions(true);
        let id = doc
            .mutate()
            .try_create_element(
                crate::QualName::new(None, markup5ever::ns!(html), "x-test".into()),
                vec![],
            )
            .unwrap();
        doc.mutate().set_attribute(
            id,
            crate::QualName::new(None, markup5ever::ns!(), "a".into()),
            "old",
        );
        let snapshot = doc.upgrade_reactions(id);
        doc.set_custom_element_state(id, CustomElementState::Precustomized);
        doc.mutate().set_attribute(
            id,
            crate::QualName::new(None, markup5ever::ns!(), "a".into()),
            "new",
        );
        assert!(doc.take_custom_element_reactions().is_empty());
        doc.set_custom_element_state(id, CustomElementState::Custom);
        doc.enqueue_custom_element_reactions(snapshot);
        assert!(
            matches!(&doc.take_custom_element_reactions()[..], [CustomElementReaction::AttributeChanged { new_value: Some(value), .. }] if value.as_str_lossy() == "old")
        );
    }

    #[test]
    fn adoption_reactions_follow_tree_order_and_retain_documents() {
        let mut doc = BaseDocument::new(crate::DocumentConfig::default());
        let (root, child, other) = {
            let mut m = doc.mutate();
            let root = m
                .try_create_element(
                    crate::QualName::new(None, markup5ever::ns!(html), "x-root".into()),
                    vec![],
                )
                .unwrap();
            let child = m
                .try_create_element(
                    crate::QualName::new(None, markup5ever::ns!(html), "x-child".into()),
                    vec![],
                )
                .unwrap();
            m.append_children(root, &[child]);
            (root, child, m.try_create_document_node().unwrap())
        };
        let old = doc.node_document(root);
        doc.set_custom_element_state(root, CustomElementState::Custom);
        doc.set_custom_element_state(child, CustomElementState::Custom);
        doc.set_custom_element_reactions(true);
        doc.mutate().set_node_document(root, other);
        assert_eq!(
            doc.take_custom_element_reactions(),
            vec![
                CustomElementReaction::Adopted {
                    element: root,
                    old_document: old,
                    new_document: other
                },
                CustomElementReaction::Adopted {
                    element: child,
                    old_document: old,
                    new_document: other
                },
            ]
        );
        doc.mutate().set_node_document(root, old);
        doc.reclaim_detached_nodes(&[]);
        assert!(doc.get_node(other).is_some());
        assert!(doc.get_node(root).is_some());
    }

    #[test]
    fn upgrade_snapshots_obey_reaction_limits() {
        let mut doc = BaseDocument::new(crate::DocumentConfig::default());
        let attrs = (0..5000)
            .map(|i| crate::Attribute {
                name: crate::QualName::new(None, markup5ever::ns!(), format!("a{i}").into()),
                value: "v".into(),
            })
            .collect();
        let id = doc
            .mutate()
            .try_create_element(
                crate::QualName::new(None, markup5ever::ns!(html), "x-many".into()),
                attrs,
            )
            .unwrap();
        assert_eq!(doc.upgrade_reactions(id).len(), 4096);
        assert!(doc.take_custom_element_reaction_overflow());
        let attrs = vec![crate::Attribute {
            name: crate::QualName::new(None, markup5ever::ns!(), "a".into()),
            value: "x".repeat(8 * 1024 * 1024).into(),
        }];
        let large = doc
            .mutate()
            .try_create_element(
                crate::QualName::new(None, markup5ever::ns!(html), "x-large".into()),
                attrs,
            )
            .unwrap();
        assert!(doc.upgrade_reactions(large).is_empty());
        assert!(doc.take_custom_element_reaction_overflow());
    }

    #[test]
    fn cloned_builtin_identity_without_literal_is_stays_upgradeable() {
        let mut doc = BaseDocument::new(crate::DocumentConfig::default());
        let id = doc
            .mutate()
            .try_create_element(
                crate::QualName::new(None, markup5ever::ns!(html), "button".into()),
                vec![],
            )
            .unwrap();
        doc.get_node_mut(id)
            .unwrap()
            .element_data_mut()
            .unwrap()
            .custom_element_is = Some("x-button".into());
        doc.set_custom_element_state(id, CustomElementState::Custom);
        let copy = doc.mutate().try_clone_node(id, false).unwrap();
        assert_eq!(
            doc.custom_element_state(copy),
            CustomElementState::Undefined
        );
        assert_eq!(
            doc.get_node(copy)
                .unwrap()
                .element_data()
                .unwrap()
                .custom_element_is
                .as_deref(),
            Some("x-button")
        );
    }

    #[test]
    fn later_literal_is_does_not_change_cloned_builtin_identity() {
        let mut doc = BaseDocument::new(crate::DocumentConfig::default());
        let id = doc
            .mutate()
            .try_create_element(
                crate::QualName::new(None, markup5ever::ns!(html), "button".into()),
                vec![],
            )
            .unwrap();
        doc.mutate().set_attribute(
            id,
            crate::QualName::new(None, markup5ever::ns!(), "is".into()),
            "x-button",
        );
        let copy = doc.mutate().try_clone_node(id, false).unwrap();
        assert_eq!(
            doc.custom_element_state(copy),
            CustomElementState::Uncustomized
        );
        assert_eq!(
            doc.get_node(copy)
                .unwrap()
                .element_data()
                .unwrap()
                .custom_element_is,
            None
        );
    }

    #[test]
    fn failed_form_upgrade_retains_internals_without_native_participation() {
        let mut doc = BaseDocument::new(crate::DocumentConfig::default());
        let (form, id) = {
            let mut m = doc.mutate();
            let form = m
                .try_create_element(
                    crate::QualName::new(None, markup5ever::ns!(html), "form".into()),
                    vec![],
                )
                .unwrap();
            let id = m
                .try_create_element(
                    crate::QualName::new(None, markup5ever::ns!(html), "x-failed".into()),
                    vec![],
                )
                .unwrap();
            m.append_children(form, &[id]);
            (form, id)
        };
        doc.set_custom_element_state(id, CustomElementState::Precustomized);
        doc.register_form_associated(id);
        assert_eq!(doc.custom_element_form_owner(id), None);
        doc.set_custom_element_state(id, CustomElementState::Failed);
        assert!(doc.is_form_associated_custom(id));
        assert!(doc.form_controls(form).is_empty());
        assert_eq!(doc.form_owner(id), None);
        assert!(!doc.will_validate(id));
        assert!(!doc.is_labelable_control(id));
        assert!(doc.custom_element_labels(id).is_empty());
        assert!(
            !doc.get_node(id)
                .unwrap()
                .element_data()
                .unwrap()
                .can_be_disabled()
        );
    }

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
