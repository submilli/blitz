//! The arena-independent data of one node, and planting it as a new node.
//!
//! Same-arena cloning and cross-arena import both capture a [`Seed`] (what
//! the clone steps copy, or everything adoption keeps) and plant it.

use markup5ever::{QualName, ns};

use crate::custom_elements::CustomElementState;
use crate::dom_api::is_text_control;
use crate::node::{Attribute, FormControlState, NodeData, ScriptState};
use crate::{BaseDocument, DocumentMutator, DomString, NodeId};

impl DocumentMutator<'_> {
    /// Create one detached node in `document` and apply its seeded state.
    pub(crate) fn plant(&mut self, seed: Seed, document: NodeId) -> NodeId {
        let id = match seed.data {
            SeedData::Element { name, attrs } => self.create_element(name, attrs),
            SeedData::Text(text) => self.create_text_node(text),
            SeedData::Comment(contents) => self.create_comment_node(contents),
            SeedData::ProcessingInstruction { target, contents } => {
                self.create_processing_instruction(&target, contents)
            }
            SeedData::Doctype {
                name,
                public_id,
                system_id,
            } => self.create_doctype(&name, &public_id, &system_id),
            SeedData::Fragment => self.create_document_fragment(),
        };
        // Set the document before custom state, so this is no adoption.
        self.set_node_document(id, document);
        if let Some(element) = seed.element {
            self.apply_element_seed(id, element);
        }
        id
    }

    fn apply_element_seed(&mut self, id: NodeId, seed: ElementSeed) {
        let checked = seed.form.checked;
        let selected = seed.form.selected;
        let associated = seed.internals.is_some();
        {
            let element = self.doc.nodes[id]
                .element_data_mut()
                .expect("element seeds plant elements");
            element.custom_element_is = seed.is;
            element.form_state = seed.form;
            element.custom_internals = seed.internals;
            element.custom_states = seed.custom_states;
            element.script_state = seed.script;
            element.canvas_dimension_revision = seed.canvas_revision;
        }
        self.doc.set_custom_element_state(id, seed.custom);
        if associated {
            self.doc.form_associated_custom.insert(id);
        }
        // Mirror the carried control state into the element's style flags.
        if let Some(checked) = checked {
            let dirty = self.doc.nodes[id]
                .element_data()
                .is_some_and(|element| element.form_state.checked_dirty);
            self.write_checkedness(id, checked);
            if let Some(element) = self.doc.nodes[id].element_data_mut() {
                element.form_state.checked_dirty = dirty;
            }
        }
        if let Some(selected) = selected {
            self.write_option_selectedness(id, selected, None);
        }
    }
}

/// The arena-independent data of one node.
pub(crate) struct Seed {
    data: SeedData,
    element: Option<ElementSeed>,
}

enum SeedData {
    Element {
        name: QualName,
        attrs: Vec<Attribute>,
    },
    Text(DomString),
    Comment(DomString),
    ProcessingInstruction {
        target: String,
        contents: DomString,
    },
    Doctype {
        name: String,
        public_id: String,
        system_id: String,
    },
    Fragment,
}

struct ElementSeed {
    is: Option<String>,
    custom: CustomElementState,
    form: FormControlState,
    internals: Option<Box<crate::custom_internals::CustomInternals>>,
    custom_states: Vec<style::values::AtomIdent>,
    script: ScriptState,
    /// Embedders compare this to keep a moved canvas's bitmap.
    canvas_revision: u64,
}

impl Seed {
    /// Capture `id`. A transfer keeps all element state; a copy keeps what
    /// the HTML clone steps copy for input and textarea controls.
    pub(crate) fn capture(doc: &BaseDocument, id: NodeId, transfer: bool) -> Self {
        let data = match &doc.nodes[id].data {
            NodeData::Element(element) | NodeData::AnonymousBlock(element) => {
                return Self {
                    data: SeedData::Element {
                        name: element.name.clone(),
                        attrs: element.attrs().to_vec(),
                    },
                    element: Some(if transfer {
                        ElementSeed::moved(element)
                    } else {
                        ElementSeed::cloned(doc, id, element)
                    }),
                };
            }
            NodeData::Text(text) => SeedData::Text(text.content.clone()),
            NodeData::Comment { contents } => SeedData::Comment(contents.clone()),
            NodeData::ProcessingInstruction { target, contents } => {
                SeedData::ProcessingInstruction {
                    target: target.clone(),
                    contents: contents.clone(),
                }
            }
            NodeData::Doctype {
                name,
                public_id,
                system_id,
            } => SeedData::Doctype {
                name: name.clone(),
                public_id: public_id.clone(),
                system_id: system_id.clone(),
            },
            // Callers reject documents; a document is never a child.
            NodeData::DocumentFragment | NodeData::Document(_) => SeedData::Fragment,
        };
        Self {
            data,
            element: None,
        }
    }
}

impl ElementSeed {
    fn moved(element: &crate::node::ElementData) -> Self {
        Self {
            is: element.custom_element_is.clone(),
            custom: element.custom_element_state,
            form: element.form_state.clone(),
            internals: element
                .custom_internals
                .as_deref()
                .map(|internals| Box::new(internals.for_transfer())),
            custom_states: element.custom_states.clone(),
            script: element.script_state,
            canvas_revision: element.canvas_dimension_revision,
        }
    }

    fn cloned(doc: &BaseDocument, id: NodeId, element: &crate::node::ElementData) -> Self {
        let custom = if element.custom_element_is.is_some() {
            CustomElementState::Undefined
        } else {
            CustomElementState::initial(&element.name)
        };
        let mut form = FormControlState::default();
        if element.name.ns == ns!(html) && matches!(&*element.name.local, "input" | "textarea") {
            let state = &element.form_state;
            let value = state.value.clone().or_else(|| doc.form_value_dom(id));
            form.value = is_text_control(element).then_some(value).flatten();
            form.value_dirty = state.value_dirty;
            form.last_change_by_user = state.last_change_by_user;
            if &*element.name.local == "input" {
                form.files = state.files.clone();
                form.checked = state.checked;
                form.checked_dirty = state.checked_dirty;
            }
        }
        Self {
            is: element.custom_element_is.clone(),
            custom,
            form,
            internals: None,
            custom_states: Vec::new(),
            script: ScriptState::default(),
            canvas_revision: 0,
        }
    }
}
