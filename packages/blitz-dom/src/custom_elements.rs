//! Custom element support in the DOM: element state (behind `:defined`) and
//! the reactions an embedder's registry runs.
//!
//! The registry itself (definitions, constructors, upgrades) lives with the
//! embedder's script engine. The DOM tracks which elements could be custom
//! elements and, while reactions are recorded, reports when they are
//! connected, disconnected or have attributes changed.

use crate::{BaseDocument, NodeId};

/// The lifecycle callbacks captured from a definition when construction starts.
#[derive(Debug, Default)]
pub struct ReactionCallbacks {
    pub connected: bool,
    pub disconnected: bool,
    pub adopted: bool,
    pub form_associated: bool,
    pub form_disabled: bool,
    pub form_reset: bool,
    pub observed_attributes: std::collections::HashSet<String>,
}

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
    /// The element moved while a form-association callback was queued.
    /// The embedder keeps the form identity alive and resolves it on delivery.
    FormAssociatedFromArena {
        element: NodeId,
        form: u64,
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
    /// The element moved between arenas before its adoption callback ran.
    /// Documents never move, so both are embedder tokens rather than IDs;
    /// the reaction survives further moves of its element.
    AdoptedFromArena {
        element: NodeId,
        old_document: u64,
        new_document: u64,
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
            | Self::FormAssociatedFromArena { element, .. }
            | Self::FormDisabled { element, .. }
            | Self::Adopted { element, .. }
            | Self::AdoptedFromArena { element, .. }
            | Self::AttributeChanged { element, .. } => *element,
        }
    }

    /// The same reaction for nodes rebuilt in another arena. `None` when a
    /// referenced node has no counterpart there; the embedder converts a
    /// same-arena `Adopted` to `AdoptedFromArena` before moving it.
    pub fn remap(self, map: impl Fn(NodeId) -> Option<NodeId>) -> Option<Self> {
        Some(match self {
            Self::Connected(id) => Self::Connected(map(id)?),
            Self::Disconnected(id) => Self::Disconnected(map(id)?),
            Self::FormReset(id) => Self::FormReset(map(id)?),
            Self::FormAssociated { element, form } => Self::FormAssociated {
                element: map(element)?,
                form: match form {
                    Some(form) => Some(map(form)?),
                    None => None,
                },
            },
            Self::FormAssociatedFromArena { element, form } => Self::FormAssociatedFromArena {
                element: map(element)?,
                form,
            },
            Self::FormDisabled { element, disabled } => Self::FormDisabled {
                element: map(element)?,
                disabled,
            },
            // A document never moves between arenas.
            Self::Adopted { .. } => return None,
            Self::AdoptedFromArena {
                element,
                old_document,
                new_document,
            } => Self::AdoptedFromArena {
                element: map(element)?,
                old_document,
                new_document,
            },
            Self::AttributeChanged {
                element,
                name,
                namespace,
                old_value,
                new_value,
            } => Self::AttributeChanged {
                element: map(element)?,
                name,
                namespace,
                old_value,
                new_value,
            },
        })
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

/// Reactions a document queues before reporting overflow, in total.
pub const MAX_QUEUED_REACTIONS: usize = 4096;
/// Retained attribute payload bytes the queued reactions may hold.
const MAX_REACTION_BYTES: usize = 8 * 1024 * 1024;

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

/// Queue position within a script-free batch of native mutations.
pub struct ReactionMark {
    pending: usize,
    observed: usize,
}

impl BaseDocument {
    /// Start or stop recording custom element reactions. Stopping discards
    /// pending ones.
    pub fn set_custom_element_reactions(&mut self, on: bool) {
        if !on {
            self.custom_element_enqueued = None;
            self.custom_element_reaction_bytes = 0;
            self.custom_element_reaction_overflow = false;
        }
        self.custom_element_reactions =
            on.then(|| self.custom_element_reactions.take().unwrap_or_default());
    }

    pub fn custom_element_reaction_mark(&self) -> ReactionMark {
        ReactionMark {
            pending: self.pending_custom_element_reaction_count(),
            observed: self.custom_element_enqueued.as_ref().map_or(0, Vec::len),
        }
    }

    pub fn pending_custom_element_reaction_count(&self) -> usize {
        self.custom_element_reactions
            .as_ref()
            .map_or(0, |q| q.len())
    }

    /// Withdraw the new suffix and its corresponding enqueue observations together.
    /// The embedder merges these reactions with arrivals from other arenas before
    /// returning to script. Earlier notifications keep their positions.
    pub fn take_custom_element_reactions_since(
        &mut self,
        mark: ReactionMark,
    ) -> Vec<CustomElementReaction> {
        if let Some(observed) = &mut self.custom_element_enqueued {
            observed.truncate(mark.observed);
        }
        let Some(queue) = &mut self.custom_element_reactions else {
            return Vec::new();
        };
        let pending: Vec<_> = queue.drain(mark.pending.min(queue.len())..).collect();
        self.custom_element_reaction_bytes = self
            .custom_element_reaction_bytes
            .saturating_sub(pending.iter().map(reaction_bytes).sum());
        pending
    }

    /// Reactions since the last call, oldest first.
    pub fn take_custom_element_reactions(&mut self) -> Vec<CustomElementReaction> {
        self.custom_element_reaction_bytes = 0;
        match &mut self.custom_element_reactions {
            Some(reactions) => reactions.drain(..).collect(),
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

    /// Upgrade snapshots stay pending while author construction can move the node.
    pub fn custom_element_upgrading(&self, id: NodeId) -> bool {
        self.get_node(id)
            .and_then(|n| n.element_data())
            .is_some_and(|e| e.custom_element_upgrading)
    }

    fn wants_custom_element_reaction(&self, reaction: &CustomElementReaction) -> bool {
        let Some(callbacks) = self
            .get_node(reaction.target())
            .and_then(|n| n.element_data())
            .and_then(|e| e.custom_element_callbacks.as_deref())
        else {
            return true;
        };
        match reaction {
            CustomElementReaction::Connected(_) => callbacks.connected,
            CustomElementReaction::Disconnected(_) => callbacks.disconnected,
            CustomElementReaction::Adopted { .. }
            | CustomElementReaction::AdoptedFromArena { .. } => callbacks.adopted,
            CustomElementReaction::FormAssociated { .. }
            | CustomElementReaction::FormAssociatedFromArena { .. } => callbacks.form_associated,
            CustomElementReaction::FormDisabled { .. } => callbacks.form_disabled,
            CustomElementReaction::FormReset(_) => callbacks.form_reset,
            CustomElementReaction::AttributeChanged { name, .. } => {
                callbacks.observed_attributes.contains(name)
            }
        }
    }

    /// Set an element's state (the embedder does this when it upgrades or
    /// constructs it), restyling it for `:defined`.
    pub fn set_custom_element_state(&mut self, node_id: NodeId, state: CustomElementState) {
        // The embedder supplies the ID; an unknown one changes nothing.
        let Some(element) = self
            .get_node_mut(node_id)
            .and_then(|node| node.element_data_mut())
        else {
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
        if self.get_node(id).is_none() {
            return reactions;
        }
        let mut bytes = 0usize;
        if let Some(element) = self.get_node(id).and_then(|node| node.element_data()) {
            for attr in element.attrs().iter() {
                if element
                    .custom_element_callbacks
                    .as_deref()
                    .is_some_and(|c| !c.observed_attributes.contains(attr.name.local.as_ref()))
                {
                    continue;
                }
                let retained =
                    attr.name.local.len() + attr.name.ns.len() + attr.value.retained_bytes();
                if reactions.len() >= MAX_QUEUED_REACTIONS
                    || retained > MAX_REACTION_BYTES.saturating_sub(bytes)
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
        if self.is_connected(id)
            && self.wants_custom_element_reaction(&CustomElementReaction::Connected(id))
        {
            if reactions.len() < MAX_QUEUED_REACTIONS {
                reactions.push(CustomElementReaction::Connected(id));
            } else {
                self.custom_element_reaction_overflow = true;
            }
        }
        reactions
    }

    /// Return unqueued reactions so the embedder can release their arguments.
    /// Capacity refusals also set the overflow flag. Absent callbacks do not.
    pub fn enqueue_custom_element_reactions(
        &mut self,
        reactions: Vec<CustomElementReaction>,
    ) -> Vec<CustomElementReaction> {
        let mut refused = Vec::new();
        for reaction in reactions {
            if !self.wants_custom_element_reaction(&reaction) {
                refused.push(reaction);
            } else if self.admits_reaction(&reaction) {
                self.push_custom_element_reaction(reaction);
            } else {
                self.custom_element_reaction_overflow |= self.custom_element_reactions.is_some();
                refused.push(reaction);
            }
        }
        refused
    }

    /// A failed upgrade discards reactions remaining for that element,
    /// returning them so the embedder can release what they reference.
    pub fn discard_custom_element_reactions(&mut self, id: NodeId) -> Vec<CustomElementReaction> {
        let Some(reactions) = &mut self.custom_element_reactions else {
            return Vec::new();
        };
        let (discarded, kept): (std::collections::VecDeque<_>, std::collections::VecDeque<_>) =
            std::mem::take(reactions)
                .into_iter()
                .partition(|reaction| reaction.target() == id);
        *reactions = kept;
        let discarded = Vec::from(discarded);
        for reaction in &discarded {
            self.custom_element_reaction_bytes = self
                .custom_element_reaction_bytes
                .saturating_sub(reaction_bytes(reaction));
        }
        discarded
    }

    /// The oldest queued reaction. Delivering one at a time lets reactions
    /// that a callback queues follow the ones still pending, as each
    /// element's [reaction queue](https://html.spec.whatwg.org/multipage/custom-elements.html#custom-element-reaction-queue)
    /// orders them, even when a callback moves its element elsewhere.
    ///
    /// With `element`, its oldest reaction comes first: an element's queue
    /// empties before the next element's, as in the element queue.
    pub fn take_next_custom_element_reaction(
        &mut self,
        element: Option<NodeId>,
    ) -> Option<CustomElementReaction> {
        let reactions = self.custom_element_reactions.as_mut()?;
        let index = element
            .and_then(|element| reactions.iter().position(|r| r.target() == element))
            .unwrap_or(0);
        let reaction = reactions.remove(index)?;
        self.custom_element_reaction_bytes = self
            .custom_element_reaction_bytes
            .saturating_sub(reaction_bytes(&reaction));
        Some(reaction)
    }

    /// Enable bounded enqueue observations for an embedder's element queues.
    /// Consume observations between operations; a full observation buffer
    /// refuses additional reactions and reports the ordinary queue overflow.
    pub fn observe_custom_element_enqueues(&mut self) {
        self.custom_element_enqueued.get_or_insert_with(Vec::new);
    }

    /// Elements enqueued since the last observation, in enqueue order. An
    /// embedder uses these to build its [CEReactions] element queues without
    /// consuming each element's pending reactions. Observe before transfers.
    pub fn take_enqueued_custom_element_targets(&mut self) -> Vec<NodeId> {
        self.custom_element_enqueued
            .as_mut()
            .map(std::mem::take)
            .unwrap_or_default()
    }

    /// Whether the element has pending reactions, without consuming one.
    pub fn has_custom_element_reactions_for(&self, element: NodeId) -> bool {
        self.custom_element_reactions
            .as_ref()
            .is_some_and(|reactions| reactions.iter().any(|r| r.target() == element))
    }

    /// Consume only this element's oldest reaction; another element's queue
    /// is never a fallback when the requested queue is empty.
    pub fn take_custom_element_reaction_for(
        &mut self,
        element: NodeId,
    ) -> Option<CustomElementReaction> {
        let reactions = self.custom_element_reactions.as_mut()?;
        let index = reactions.iter().position(|r| r.target() == element)?;
        let reaction = reactions.remove(index)?;
        self.custom_element_reaction_bytes = self
            .custom_element_reaction_bytes
            .saturating_sub(reaction_bytes(&reaction));
        Some(reaction)
    }

    /// Remove reactions whose elements move, retaining all others in place.
    /// Retained elements are not enqueued again into the embedder's current
    /// reaction scope.
    pub fn extract_custom_element_reactions(
        &mut self,
        moving: impl Fn(NodeId) -> bool,
    ) -> Vec<CustomElementReaction> {
        let Some(reactions) = &mut self.custom_element_reactions else {
            return Vec::new();
        };
        let (moved, kept): (std::collections::VecDeque<_>, std::collections::VecDeque<_>) =
            std::mem::take(reactions)
                .into_iter()
                .partition(|r| moving(r.target()));
        *reactions = kept;
        for reaction in &moved {
            self.custom_element_reaction_bytes = self
                .custom_element_reaction_bytes
                .saturating_sub(reaction_bytes(reaction));
        }
        Vec::from(moved)
    }

    /// Keep a queued callback in place when only its form argument changes arena.
    /// Both representations have zero payload bytes and the same target.
    pub fn relocate_queued_form_arguments(
        &mut self,
        destination: impl Fn(NodeId) -> Option<NodeId>,
        mut retain: impl FnMut(NodeId) -> Option<u64>,
    ) {
        let Some(queue) = &mut self.custom_element_reactions else {
            return;
        };
        queue.retain_mut(|reaction| {
            let CustomElementReaction::FormAssociated {
                element,
                form: Some(form),
            } = *reaction
            else {
                return true;
            };
            let Some(new) = destination(form) else {
                return true;
            };
            let Some(token) = retain(new) else {
                self.custom_element_reaction_overflow = true;
                return false;
            };
            *reaction = CustomElementReaction::FormAssociatedFromArena {
                element,
                form: token,
            };
            true
        });
    }

    pub(crate) fn record_custom_element_reaction(&mut self, reaction: CustomElementReaction) {
        if !self.wants_custom_element_reaction(&reaction) {
            return;
        }
        if !self.admits_reaction(&reaction) {
            self.custom_element_reaction_overflow |= self.custom_element_reactions.is_some();
            return;
        }
        self.push_custom_element_reaction(reaction);
    }

    fn push_custom_element_reaction(&mut self, reaction: CustomElementReaction) {
        if let Some(reactions) = &mut self.custom_element_reactions {
            self.custom_element_reaction_bytes += reaction_bytes(&reaction);
            if let Some(enqueued) = &mut self.custom_element_enqueued {
                enqueued.push(reaction.target());
            }
            reactions.push_back(reaction);
        }
    }

    fn admits_reaction(&self, reaction: &CustomElementReaction) -> bool {
        self.custom_element_reactions
            .as_ref()
            .is_some_and(|reactions| {
                self.custom_element_enqueued
                    .as_ref()
                    .is_none_or(|targets| targets.len() < MAX_QUEUED_REACTIONS)
                    && reactions.len() < MAX_QUEUED_REACTIONS
                    && reaction_bytes(reaction)
                        <= MAX_REACTION_BYTES.saturating_sub(self.custom_element_reaction_bytes)
            })
    }

    /// Consume the overflow diagnostic; a bounded queue is never silently complete.
    pub fn take_custom_element_reaction_overflow(&mut self) -> bool {
        std::mem::take(&mut self.custom_element_reaction_overflow)
    }

    pub(crate) fn is_recording_custom_element_reactions(&self) -> bool {
        self.custom_element_reactions.is_some()
    }
}

/// Retained payload bytes a queued reaction counts against the queue bound.
fn reaction_bytes(reaction: &CustomElementReaction) -> usize {
    match reaction {
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
    }
}

#[cfg(test)]
mod bounds_tests {
    use super::*;

    #[test]
    fn destroying_a_target_releases_its_reactions_and_enqueue_observations() {
        let mut doc = BaseDocument::new(Default::default());
        doc.set_custom_element_reactions(true);
        doc.observe_custom_element_enqueues();
        let node = doc.mutate().create_element(
            markup5ever::QualName::new(None, markup5ever::ns!(html), "x-gone".into()),
            vec![],
        );
        doc.enqueue_custom_element_reactions(vec![CustomElementReaction::Connected(node)]);
        doc.mutate().remove_and_drop_node(node);
        assert!(doc.take_enqueued_custom_element_targets().is_empty());
        assert!(doc.take_custom_element_reactions().is_empty());
        assert_eq!(doc.custom_element_reaction_bytes, 0);
    }

    #[test]
    fn reactions_are_taken_oldest_first_and_release_their_bytes() {
        let mut doc = BaseDocument::new(crate::DocumentConfig::default());
        doc.set_custom_element_reactions(true);
        let element = doc.mutate().create_element(
            markup5ever::QualName::new(None, markup5ever::ns!(html), "x-a".into()),
            vec![],
        );
        let changed = || CustomElementReaction::AttributeChanged {
            element,
            name: "a".into(),
            namespace: None,
            old_value: None,
            new_value: Some("x".repeat(3 << 20).into()),
        };
        doc.enqueue_custom_element_reactions(vec![
            CustomElementReaction::Connected(element),
            changed(),
        ]);
        assert!(matches!(
            doc.take_next_custom_element_reaction(None),
            Some(CustomElementReaction::Connected(_))
        ));
        // Taking the large reaction frees its bytes for another one.
        assert!(doc.take_next_custom_element_reaction(None).is_some());
        assert!(
            doc.enqueue_custom_element_reactions(vec![changed(), changed()])
                .is_empty()
        );
        assert!(!doc.take_custom_element_reaction_overflow());
        // A third payload no longer fits: it is handed back, not lost.
        let refused = doc.enqueue_custom_element_reactions(vec![changed()]);
        assert_eq!(refused.len(), 1);
        assert!(doc.take_custom_element_reaction_overflow());
        assert_eq!(doc.take_custom_element_reactions().len(), 2);
        assert!(doc.take_next_custom_element_reaction(None).is_none());
    }

    #[test]
    fn withdrawing_a_batch_preserves_earlier_enqueue_observations_and_payloads() {
        use CustomElementReaction as R;
        let mut doc = BaseDocument::new(Default::default());
        doc.set_custom_element_reactions(true);
        doc.observe_custom_element_enqueues();
        let id = doc.root_node().id;
        let attribute = R::AttributeChanged {
            element: id,
            name: "a".into(),
            namespace: None,
            old_value: None,
            new_value: Some("payload".into()),
        };
        doc.enqueue_custom_element_reactions(vec![attribute.clone()]);
        let bytes = doc.custom_element_reaction_bytes;
        let mark = doc.custom_element_reaction_mark();
        doc.enqueue_custom_element_reactions(vec![R::Connected(id), attribute]);
        let suffix = doc.take_custom_element_reactions_since(mark);
        assert!(matches!(
            suffix.as_slice(),
            [R::Connected(_), R::AttributeChanged { .. }]
        ));
        assert_eq!(doc.custom_element_reaction_bytes, bytes);
        assert_eq!(doc.take_enqueued_custom_element_targets(), vec![id]);
        assert_eq!(doc.pending_custom_element_reaction_count(), 1);
    }

    #[test]
    fn moving_a_form_updates_pending_arguments_without_reordering_the_target() {
        use CustomElementReaction as R;
        let mut doc = BaseDocument::new(Default::default());
        doc.set_custom_element_reactions(true);
        doc.observe_custom_element_enqueues();
        let name = || markup5ever::QualName::new(None, markup5ever::ns!(html), "x-a".into());
        let element = doc.mutate().create_element(name(), vec![]);
        let form = doc.mutate().create_element(name(), vec![]);
        let new = doc.root_node().id;
        doc.enqueue_custom_element_reactions(vec![
            R::Connected(element),
            R::FormAssociated {
                element,
                form: Some(form),
            },
            R::FormAssociated {
                element,
                form: Some(form),
            },
            R::FormAssociated {
                element,
                form: None,
            },
        ]);
        let mut tokens = [42].into_iter();
        doc.relocate_queued_form_arguments(
            |id| (id == form).then_some(new),
            |id| {
                assert_eq!(id, new);
                tokens.next()
            },
        );
        assert!(doc.take_custom_element_reaction_overflow());
        assert_eq!(doc.take_enqueued_custom_element_targets(), vec![element; 4]);
        let reactions = doc.take_custom_element_reactions();
        assert!(matches!(
            reactions.as_slice(),
            [
                R::Connected(_),
                R::FormAssociatedFromArena { form: 42, .. },
                R::FormAssociated { form: None, .. }
            ]
        ));
    }

    #[test]
    fn observed_element_queues_preserve_unrelated_pending_reactions() {
        let mut doc = BaseDocument::new(crate::DocumentConfig::default());
        doc.set_custom_element_reactions(true);
        doc.observe_custom_element_enqueues();
        let name = || markup5ever::QualName::new(None, markup5ever::ns!(html), "x-a".into());
        let a = doc.mutate().create_element(name(), vec![]);
        let b = doc.mutate().create_element(name(), vec![]);
        doc.enqueue_custom_element_reactions(vec![CustomElementReaction::Connected(a)]);
        assert_eq!(doc.take_enqueued_custom_element_targets(), vec![a]);
        doc.enqueue_custom_element_reactions(vec![CustomElementReaction::Connected(b)]);
        assert_eq!(doc.take_enqueued_custom_element_targets(), vec![b]);
        assert!(doc.take_custom_element_reaction_for(b).is_some());
        assert!(doc.take_custom_element_reaction_for(b).is_none());
        assert_eq!(doc.extract_custom_element_reactions(|id| id == b).len(), 0);
        assert!(doc.take_enqueued_custom_element_targets().is_empty());
        assert!(
            matches!(doc.take_custom_element_reaction_for(a), Some(CustomElementReaction::Connected(id)) if id == a)
        );
    }

    #[test]
    fn enqueue_observations_are_bounded_even_when_reactions_are_consumed() {
        let mut doc = BaseDocument::new(crate::DocumentConfig::default());
        doc.set_custom_element_reactions(true);
        doc.observe_custom_element_enqueues();
        let element = doc.mutate().create_element(
            markup5ever::QualName::new(None, markup5ever::ns!(html), "x-a".into()),
            vec![],
        );
        for _ in 0..MAX_QUEUED_REACTIONS {
            assert!(
                doc.enqueue_custom_element_reactions(vec![CustomElementReaction::Connected(
                    element
                )])
                .is_empty()
            );
            assert!(doc.take_custom_element_reaction_for(element).is_some());
        }
        assert_eq!(
            doc.enqueue_custom_element_reactions(vec![CustomElementReaction::Connected(element)])
                .len(),
            1
        );
        assert!(doc.take_custom_element_reaction_overflow());
        assert_eq!(
            doc.take_enqueued_custom_element_targets().len(),
            MAX_QUEUED_REACTIONS
        );
        assert!(
            doc.enqueue_custom_element_reactions(vec![CustomElementReaction::Connected(element)])
                .is_empty()
        );
    }

    #[test]
    fn an_elements_reactions_are_taken_before_the_next_elements() {
        let mut doc = BaseDocument::new(crate::DocumentConfig::default());
        doc.set_custom_element_reactions(true);
        let name = || markup5ever::QualName::new(None, markup5ever::ns!(html), "x-a".into());
        let a = doc.mutate().create_element(name(), vec![]);
        let b = doc.mutate().create_element(name(), vec![]);
        doc.enqueue_custom_element_reactions(vec![
            CustomElementReaction::Disconnected(a),
            CustomElementReaction::Disconnected(b),
            CustomElementReaction::Connected(a),
            CustomElementReaction::Connected(b),
        ]);
        let mut order = Vec::new();
        let mut last = None;
        while let Some(reaction) = doc.take_next_custom_element_reaction(last) {
            last = Some(reaction.target());
            order.push(reaction);
        }
        assert!(matches!(
            order[..],
            [
                CustomElementReaction::Disconnected(first),
                CustomElementReaction::Connected(second),
                CustomElementReaction::Disconnected(third),
                CustomElementReaction::Connected(fourth),
            ] if first == a && second == a && third == b && fourth == b
        ));
    }

    #[test]
    fn remapped_reactions_name_rebuilt_nodes_and_drop_unmapped_ones() {
        let mut doc = BaseDocument::new(crate::DocumentConfig::default());
        let mut m = doc.mutate();
        let name = || markup5ever::QualName::new(None, markup5ever::ns!(html), "x-a".into());
        let (a, b) = (
            m.create_element(name(), vec![]),
            m.create_element(name(), vec![]),
        );
        let map = |id| (id == a).then_some(b);
        assert!(matches!(
            CustomElementReaction::Disconnected(a).remap(map),
            Some(CustomElementReaction::Disconnected(id)) if id == b
        ));
        let foreign_form = CustomElementReaction::FormAssociated {
            element: a,
            form: Some(b),
        };
        assert!(foreign_form.remap(map).is_none());
        let carried_form = CustomElementReaction::FormAssociatedFromArena {
            element: a,
            form: 42,
        }
        .remap(map)
        .expect("the element has a destination");
        assert_eq!(carried_form.referenced_nodes().collect::<Vec<_>>(), [b]);
        assert!(matches!(
            carried_form,
            CustomElementReaction::FormAssociatedFromArena { element, form: 42 } if element == b
        ));
        let adopted = CustomElementReaction::Adopted {
            element: a,
            old_document: a,
            new_document: a,
        };
        assert!(adopted.remap(map).is_none());
        let carried = CustomElementReaction::AdoptedFromArena {
            element: a,
            old_document: 1,
            new_document: 2,
        };
        assert!(matches!(
            carried.remap(map),
            Some(CustomElementReaction::AdoptedFromArena { element, old_document: 1, new_document: 2 })
                if element == b
        ));
    }
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

    #[test]
    fn unknown_embedder_ids_change_nothing() {
        let mut doc = BaseDocument::new(crate::DocumentConfig::default());
        let unknown = NodeId::from_u64(u64::MAX - 1);
        assert!(doc.get_node(unknown).is_none());
        doc.set_custom_element_state(unknown, CustomElementState::Custom);
        assert!(doc.upgrade_reactions(unknown).is_empty());
    }
}
