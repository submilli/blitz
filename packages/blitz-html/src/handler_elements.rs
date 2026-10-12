//! Elements the parser gives event handler content attributes, so the
//! embedder can activate their handlers after each parse step without walking
//! the document.

use blitz_dom::NodeId;

use crate::html_sink::{DocAccess, HtmlSink};

impl<A: DocAccess> HtmlSink<A> {
    /// Remember `element` if one of `attrs` may be an event handler, so the
    /// embedder can activate it without walking the document.
    pub(crate) fn record_handler_attributes(
        &self,
        element: NodeId,
        attrs: &[html5ever::Attribute],
    ) {
        if has_handler_attribute(attrs) {
            self.record_handler_element(element);
        }
    }

    /// Remember `element`, which has an `on*` attribute. Repeated `<html>`
    /// and `<body>` tags name the same element again.
    pub(crate) fn record_handler_element(&self, element: NodeId) {
        let mut elements = self.handler_elements.borrow_mut();
        if elements.last() != Some(&element) {
            elements.push(element);
        }
    }

    /// The elements recorded by [`Self::record_handler_attributes`].
    pub(crate) fn take_handler_elements(&self) -> Vec<NodeId> {
        let mut elements = std::mem::take(&mut *self.handler_elements.borrow_mut());
        // Repeated `<html>` and `<body>` tags may name an element again.
        elements.sort_unstable();
        elements.dedup();
        elements
    }
}

/// Whether one of `attrs` may be an event handler content attribute.
pub(crate) fn has_handler_attribute(attrs: &[html5ever::Attribute]) -> bool {
    attrs
        .iter()
        .any(|attr| attr.name.ns == html5ever::ns!() && attr.name.local.starts_with("on"))
}
