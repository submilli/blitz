//! HTML checkable-input activation shared by native and script event dispatch.
//! <https://html.spec.whatwg.org/multipage/input.html#checkbox-state-(type=checkbox)>

use crate::{BaseDocument, NodeId, NodeLease, ns};

/// Saved state and leases survive callbacks, tree mutations and native collection.
pub struct CheckableActivation {
    target: NodeId,
    checked: bool,
    indeterminate: bool,
    radio: bool,
    previous: Previous,
    _target_lease: NodeLease,
}

/// The radio of the target's group that was checked before activation.
enum Previous {
    None,
    Node {
        id: NodeId,
        _lease: NodeLease,
    },
    /// It was checked, but listeners left it in another document than the
    /// target, so it is in the target's group no longer.
    Elsewhere,
}

impl BaseDocument {
    /// Script MouseEvent dispatch activates disabled checkable controls too.
    pub fn is_checkable_input(&self, id: NodeId) -> bool {
        self.get_node(id)
            .and_then(|n| n.element_data())
            .is_some_and(|e| {
                e.name.ns == ns!(html)
                    && &*e.name.local == "input"
                    && e.attr(markup5ever::local_name!("type")).is_some_and(|t| {
                        t.eq_ignore_ascii_case("checkbox") || t.eq_ignore_ascii_case("radio")
                    })
            })
    }

    /// A nonempty equal name, current form owner and tree define a radio group.
    pub fn same_radio_group(&self, a: NodeId, b: NodeId) -> bool {
        let Some(name) = self.radio_name(a) else {
            return false;
        };
        self.radio_name(b) == Some(name)
            && self.tree_root(a) == self.tree_root(b)
            && self.form_owner(a) == self.form_owner(b)
    }

    /// Resolve a radio group in two linear tree passes. Explicit form references
    /// use the first matching ID; ancestor ownership follows current DOM parents.
    /// This avoids a whole-tree form lookup for every radio candidate.
    pub(crate) fn radio_group_members(&self, target: NodeId) -> Vec<NodeId> {
        use crate::traversal::TreeTraverser;
        let Some(name) = self.radio_name(target) else {
            return Vec::new();
        };
        let root = self.tree_root(target);
        let owners = self.form_owners_in_tree(root);
        let owner = owners.get(&target);
        TreeTraverser::new_with_root(self, root)
            .filter(|&id| self.radio_name(id) == Some(name) && owners.get(&id) == owner)
            .collect()
    }

    pub(crate) fn radio_name(&self, id: NodeId) -> Option<&crate::DomString> {
        let e = self.get_node(id)?.element_data()?;
        if e.name.ns != ns!(html)
            || &*e.name.local != "input"
            || !e
                .attr(markup5ever::local_name!("type"))
                .is_some_and(|t| t.eq_ignore_ascii_case("radio"))
        {
            return None;
        }
        e.attr_dom(markup5ever::local_name!("name"))
            .filter(|name| !name.is_empty())
    }

    /// Save rollback state and update checkedness before invoking any listener.
    pub fn begin_checkable_activation(&mut self, target: NodeId) -> Option<CheckableActivation> {
        if !self.is_checkable_input(target) {
            return None;
        }
        let radio = self
            .get_node(target)?
            .element_data()?
            .attr(markup5ever::local_name!("type"))?
            .eq_ignore_ascii_case("radio");
        let checked = self.checkedness(target);
        let indeterminate = self
            .get_node(target)?
            .element_data()?
            .form_state
            .indeterminate;
        let previous = if radio {
            if checked {
                Some(target)
            } else {
                self.radio_group_members(target)
                    .into_iter()
                    .find(|&id| self.checkedness(id))
            }
        } else {
            None
        };
        let activation = CheckableActivation {
            target,
            checked,
            indeterminate,
            radio,
            previous: previous.map_or(Previous::None, |id| Previous::Node {
                id,
                _lease: self.lease_node(id),
            }),
            _target_lease: self.lease_node(target),
        };
        self.mutate().set_checkedness(target, radio || !checked);
        if !radio {
            self.nodes[target]
                .element_data_mut()?
                .form_state
                .indeterminate = false;
        }
        Some(activation)
    }
}

impl CheckableActivation {
    /// The checkable input the click activates.
    pub fn target(&self) -> NodeId {
        self.target
    }

    /// The other radio of the target's group that was checked before the
    /// click: a canceled click checks it again.
    pub fn previous(&self) -> Option<NodeId> {
        match self.previous {
            Previous::Node { id, .. } if id != self.target => Some(id),
            _ => None,
        }
    }

    /// This activation in `doc`, the document a listener moved its target
    /// into. `map` gives a node's ID in `doc`, or `None` when the node is not
    /// there; `None` when the target is not.
    pub fn relocate(
        self,
        doc: &BaseDocument,
        mut map: impl FnMut(NodeId) -> Option<NodeId>,
    ) -> Option<Self> {
        let target = map(self.target)?;
        let previous = match self.previous {
            Previous::Node { id, .. } => match map(id) {
                Some(id) => Previous::Node {
                    id,
                    _lease: doc.lease_node(id),
                },
                None => Previous::Elsewhere,
            },
            previous => previous,
        };
        Some(Self {
            target,
            previous,
            _target_lease: doc.lease_node(target),
            ..self
        })
    }

    /// Roll back a canceled click, or return the target eligible for input/change.
    /// Listener changes remain authoritative on a successful activation.
    pub fn finish(self, doc: &mut BaseDocument, canceled: bool) -> Option<NodeId> {
        if canceled {
            if !doc.is_checkable_input(self.target) {
                return None;
            }
            let current_radio = doc
                .get_node(self.target)?
                .element_data()?
                .attr(markup5ever::local_name!("type"))
                .is_some_and(|t| t.eq_ignore_ascii_case("radio"));
            if current_radio {
                match self.previous {
                    Previous::Node { id: previous, .. } => {
                        if previous == self.target || doc.same_radio_group(self.target, previous) {
                            doc.mutate().set_checkedness(previous, true);
                        }
                    }
                    Previous::None => doc.mutate().set_checkedness(self.target, false),
                    Previous::Elsewhere => {}
                }
            } else {
                doc.mutate().set_checkedness(self.target, self.checked);
                if let Some(el) = doc.nodes[self.target].element_data_mut() {
                    el.form_state.indeterminate = self.indeterminate;
                }
            }
            return None;
        }
        if !doc.is_connected(self.target)
            || !doc.is_checkable_input(self.target)
            || (doc
                .get_node(self.target)?
                .element_data()?
                .attr(markup5ever::local_name!("type"))
                .is_some_and(|t| t.eq_ignore_ascii_case("radio"))
                && ((self.radio && self.checked) || !doc.checkedness(self.target)))
        {
            return None;
        }
        Some(self.target)
    }
}
