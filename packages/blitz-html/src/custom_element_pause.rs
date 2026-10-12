//! Construct custom elements as the streaming parser creates them.
//!
//! HTML's "create an element for a token" constructs an element whose
//! custom element definition exists synchronously, appends the token's
//! attributes and runs their callbacks, all before the element is inserted
//! (steps 7–10). The tree builder runs no script, so the creation is split:
//!
//! 1. With a [`DefinitionLookup`], the tree builder stops right before it
//!    would create such an element (`TreeBuilder::set_pause_before`), after
//!    the steps that precede the creation; the streaming parser reports
//!    `ParseStep::CustomElement`.
//! 2. The embedder constructs the element and hands it over
//!    ([`BoundedHtml::construct`]): the sink appends the tag's attributes to
//!    it and keeps it in [`HtmlSink`]'s constructed slot, and the embedder
//!    runs the attributes' callbacks.
//! 3. The tree builder creates the paused element
//!    ([`BoundedHtml::create_paused`]), taking the constructed one from the
//!    slot (`HtmlSink::take_constructed`), and inserts it.
//!
//! <https://html.spec.whatwg.org/multipage/parsing.html#create-an-element-for-the-token>

use blitz_dom::NodeId;
use blitz_dom::custom_elements::is_valid_custom_element_name;
use html5ever::{LocalName, QualName, ns};

use crate::bounded_parser::BoundedHtml;
use crate::html_sink::{DocAccess, HtmlSink, ParserHandle};

/// Whether a valid custom element name has a definition in the registry of
/// the document being parsed. Consulted for each such start tag.
pub type DefinitionLookup = Box<dyn Fn(&str) -> bool>;

impl<A: DocAccess> BoundedHtml<A> {
    /// Stop before creating elements whose name `lookup` reports defined,
    /// except in inert template contents: their document has no browsing
    /// context, so no definitions. A declarative shadow root's contents
    /// belong to the document.
    pub(crate) fn set_definition_lookup(&mut self, lookup: DefinitionLookup) {
        self.0.set_pause_before(
            Box::new(move |name| is_valid_custom_element_name(name) && lookup(name)),
            Box::new(ParserHandle::declares_shadow_root),
        );
    }

    /// The name of the custom element the tree builder stopped before.
    pub(crate) fn paused_custom_element(&self) -> Option<LocalName> {
        self.0.paused_tag().map(|tag| tag.name)
    }

    /// Use `element`, constructed for the paused start tag, as its element,
    /// and append the tag's attributes to it. Refused (`false`) without a
    /// paused tag, or unless `element` is a live HTML element of its name.
    pub(crate) fn construct(&self, element: NodeId) -> bool {
        let Some(tag) = self.0.paused_tag() else {
            return false;
        };
        let name = QualName::new(None, ns!(html), tag.name.clone());
        self.0.sink.construct(element, &name, &tag.attrs)
    }

    /// Create and insert the paused element (the constructed one, if any).
    /// `false` without a paused tag.
    pub(crate) fn create_paused(&self) -> bool {
        let created = self.0.create_paused_element();
        self.0.sink.constructed.borrow_mut().take();
        created
    }
}

/// The constructed-element slot.
impl<A: DocAccess> HtmlSink<A> {
    /// Step 9: append the token's attributes to `element`, the custom element
    /// constructed for it, and keep it for the tree builder's creation of
    /// that element. Refused (`false`) unless `element` is a live HTML
    /// element named `name`.
    fn construct(&self, element: NodeId, name: &QualName, attrs: &[html5ever::Attribute]) -> bool {
        let valid = self.with(|m| {
            m.doc
                .get_node(element)
                .and_then(|node| node.element_data())
                .is_some_and(|data| data.name == *name)
        });
        if !valid || self.node_limit_exceeded() {
            return false;
        }
        self.with(|m| {
            for attr in attrs {
                m.set_attribute(element, attr.name.clone(), attr.value.as_ref());
            }
        });
        self.record_handler_attributes(element, attrs);
        *self.constructed.borrow_mut() = Some(self.handle(Some(element), name.clone()));
        true
    }

    /// The constructed element, when the tree builder creates the element it
    /// was constructed for: the paused element is created right after the
    /// slot is filled, and is the only element that creation makes.
    pub(crate) fn take_constructed(&self, name: &QualName) -> Option<ParserHandle> {
        let mut constructed = self.constructed.borrow_mut();
        if constructed
            .as_ref()
            .is_none_or(|handle| handle.node_name() != name)
        {
            return None;
        }
        constructed.take()
    }
}
