//! The handles html5ever's tree builder holds for the nodes it builds.

use std::cell::Cell;
use std::rc::Rc;

use blitz_dom::NodeId;
use html5ever::QualName;

/// An HTML tree-builder handle remains valid after node admission fails. Its
/// name is retained without an arena allocation; it can never alias a live node.
#[derive(Clone)]
pub struct ParserHandle(pub(crate) Rc<ParserNode>);

pub(crate) struct ParserNode {
    lease: Option<blitz_dom::NodeLease>,
    pub(crate) parser_form: Cell<Option<NodeId>>,
    pub(crate) declarative_root: Cell<Option<NodeId>>,
    pub(crate) name: QualName,
}

#[cfg(test)]
thread_local! {
    static LIVE_HANDLES: Cell<usize> = const { Cell::new(0) };
}

#[cfg(test)]
impl Drop for ParserNode {
    fn drop(&mut self) {
        LIVE_HANDLES.with(|count| count.set(count.get() - 1));
    }
}

#[cfg(test)]
pub(crate) fn live_parser_handles() -> usize {
    LIVE_HANDLES.with(Cell::get)
}

impl ParserHandle {
    pub(crate) fn new(lease: Option<blitz_dom::NodeLease>, name: QualName) -> Self {
        #[cfg(test)]
        LIVE_HANDLES.with(|count| count.set(count.get() + 1));
        Self(Rc::new(ParserNode {
            lease,
            name,
            parser_form: Cell::new(None),
            declarative_root: Cell::new(None),
        }))
    }

    pub(crate) fn node_id(&self) -> Option<NodeId> {
        self.0.lease.as_ref().map(blitz_dom::NodeLease::id)
    }

    /// A `<template>` whose contents are the declarative shadow root it
    /// attached.
    pub(crate) fn declares_shadow_root(&self) -> bool {
        self.0.declarative_root.get().is_some()
    }

    pub(crate) fn node_name(&self) -> &QualName {
        &self.0.name
    }

    pub(crate) fn non_element(lease: Option<blitz_dom::NodeLease>) -> Self {
        Self::new(lease, QualName::new(None, html5ever::ns!(html), "".into()))
    }
}
