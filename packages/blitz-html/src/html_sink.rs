//! An implementation for Html5ever's sink trait, allowing us to parse HTML into a DOM.

use html5ever::ParseOpts;
use html5ever::tokenizer::TokenizerOpts;
use html5ever::tree_builder::TreeBuilderOpts;
use std::borrow::Cow;
use std::cell::{Cell, Ref, RefCell};
use std::rc::Rc;

use blitz_dom::node::Attribute;
use blitz_dom::{BaseDocument, DocumentMutator, HtmlParserProvider, NodeId};
use html5ever::{
    QualName,
    tendril::{StrTendril, TendrilSink},
    tree_builder::{ElementFlags, NodeOrText, QuirksMode, TreeSink},
};

/// Convert an html5ever Attribute which uses tendril for its value to a blitz Attribute
/// which uses String.
fn html5ever_to_blitz_attr(attr: html5ever::Attribute) -> Attribute {
    Attribute {
        name: attr.name,
        value: attr.value.to_string(),
    }
}

#[derive(Copy, Clone, Default, Debug)]
pub struct HtmlProvider;

impl HtmlParserProvider for HtmlProvider {
    fn parse_inner_html<'m2, 'doc2>(
        &self,
        mutr: &'m2 mut DocumentMutator<'doc2>,
        element_id: NodeId,
        html: &str,
    ) {
        DocumentHtmlParser::parse_inner_html_into_mutator(mutr, element_id, html);
    }

    fn parse_into_document_node<'m, 'doc>(
        &self,
        mutr: &'m mut DocumentMutator<'doc>,
        document: NodeId,
        markup: &str,
        xml: bool,
    ) {
        DocumentHtmlParser::parse_into_document_node(mutr, document, markup, xml);
    }

    fn parse_document(
        &self,
        html: &str,
        config: blitz_dom::DocumentConfig,
    ) -> Box<dyn blitz_dom::Document> {
        Box::new(crate::HtmlDocument::from_html(html, config))
    }
}

/// How a tree sink reaches the document it builds.
pub trait DocAccess {
    /// Run `f` with a mutator. Implementations may create a fresh mutator
    /// per call, so no borrow of the document outlives `f`.
    fn with_mutator<R>(&self, f: impl FnOnce(&mut DocumentMutator<'_>) -> R) -> R;
    /// The qualified name of element `id`.
    fn element_name(&self, id: NodeId) -> Ref<'_, QualName>;
}

/// A mutator borrowed for the whole parse (one-shot parsing).
pub struct BorrowedMutator<'m, 'doc>(RefCell<&'m mut DocumentMutator<'doc>>);

impl DocAccess for BorrowedMutator<'_, '_> {
    fn with_mutator<R>(&self, f: impl FnOnce(&mut DocumentMutator<'_>) -> R) -> R {
        let mut guard = self.0.borrow_mut();
        f(&mut guard)
    }

    fn element_name(&self, id: NodeId) -> Ref<'_, QualName> {
        Ref::map(self.0.borrow(), |docm| {
            docm.element_name(id)
                .expect("TreeSink::elem_name called on a node which is not an element!")
        })
    }
}

/// A shared document, borrowed only per tree operation, so scripts can run
/// (and mutate the document) while the parser is paused.
pub struct SharedDocument(pub Rc<RefCell<BaseDocument>>);

impl DocAccess for SharedDocument {
    fn with_mutator<R>(&self, f: impl FnOnce(&mut DocumentMutator<'_>) -> R) -> R {
        let mut doc = self.0.borrow_mut();
        let mut mutator = doc.mutate();
        f(&mut mutator)
    }

    fn element_name(&self, id: NodeId) -> Ref<'_, QualName> {
        Ref::map(self.0.borrow(), |doc| {
            &doc.get_node(id)
                .and_then(|n| n.element_data())
                .expect("TreeSink::elem_name called on a node which is not an element!")
                .name
        })
    }
}

/// An html5ever tree sink building a Blitz document.
pub struct HtmlSink<A: DocAccess> {
    access: A,

    /// Errors that occurred during parsing.
    pub errors: RefCell<Vec<Cow<'static, str>>>,

    /// The document's quirks mode.
    pub quirks_mode: Cell<QuirksMode>,
    pub is_xml: bool,
    /// Fragment parsing (`innerHTML`): scripts are marked already started.
    pub fragment: bool,
    /// Build into this detached document node instead of the tree's root.
    pub document_node: Option<NodeId>,
}

/// The one-shot parser over a borrowed mutator.
pub type DocumentHtmlParser<'m, 'doc> = HtmlSink<BorrowedMutator<'m, 'doc>>;

impl<A: DocAccess> HtmlSink<A> {
    pub fn with_access(access: A) -> Self {
        HtmlSink {
            access,
            errors: RefCell::new(Vec::new()),
            quirks_mode: Cell::new(QuirksMode::NoQuirks),
            is_xml: false,
            fragment: false,
            document_node: None,
        }
    }

    fn with<R>(&self, f: impl FnOnce(&mut DocumentMutator<'_>) -> R) -> R {
        self.access.with_mutator(f)
    }
}

impl<'m, 'doc> HtmlSink<BorrowedMutator<'m, 'doc>> {
    pub fn new(mutr: &'m mut DocumentMutator<'doc>) -> Self {
        Self::with_access(BorrowedMutator(RefCell::new(mutr)))
    }

    /// Detects documents without an XML or DOCTYPE declaration whose root `<html>` element
    /// declares the XHTML namespace (e.g. `<html xmlns="http://www.w3.org/1999/xhtml">`)
    fn root_element_has_xhtml_namespace(html: &str) -> bool {
        let rest = html.trim_start_matches('\u{feff}').trim_start();
        let Some(rest) = rest.strip_prefix("<html") else {
            return false;
        };
        let Some(tag_end) = rest.find('>') else {
            return false;
        };
        rest[..tag_end].contains("xmlns=\"http://www.w3.org/1999/xhtml\"")
            || rest[..tag_end].contains("xmlns='http://www.w3.org/1999/xhtml'")
    }

    pub fn parse_into_mutator<'a, 'd>(mutr: &'a mut DocumentMutator<'d>, html: &str) {
        let is_xhtml_doc = html.starts_with("<?xml")
            || html.starts_with("<!DOCTYPE") && {
                let first_line = html.lines().next().unwrap();
                first_line.contains("XHTML") || first_line.contains("xhtml")
            }
            || Self::root_element_has_xhtml_namespace(html);

        if is_xhtml_doc {
            Self::parse_xml_into_mutator(mutr, html);
        } else {
            // Parse as HTML
            let mut sink = DocumentHtmlParser::new(mutr);
            sink.is_xml = false;
            let opts = ParseOpts {
                tokenizer: TokenizerOpts::default(),
                tree_builder: TreeBuilderOpts {
                    exact_errors: false,
                    scripting_enabled: false, // Enables parsing of <noscript> tags
                    iframe_srcdoc: false,
                    drop_doctype: false,
                    quirks_mode: QuirksMode::NoQuirks,
                },
            };
            html5ever::parse_document(sink, opts)
                .from_utf8()
                .read_from(&mut html.as_bytes())
                .unwrap();
        }
    }

    /// Parse `markup` as a complete document into the detached document node
    /// `document`. Scripting is disabled and scripts are marked already
    /// started, as for `DOMParser`.
    pub fn parse_into_document_node<'a, 'd>(
        mutr: &'a mut DocumentMutator<'d>,
        document: NodeId,
        markup: &str,
        xml: bool,
    ) {
        let mut sink = DocumentHtmlParser::new(mutr);
        sink.document_node = Some(document);
        sink.fragment = true;
        if xml {
            sink.is_xml = true;
            xml5ever::driver::parse_document(sink, Default::default())
                .from_utf8()
                .read_from(&mut markup.as_bytes())
                .unwrap();
            return;
        }
        let opts = ParseOpts {
            tokenizer: TokenizerOpts::default(),
            tree_builder: TreeBuilderOpts {
                exact_errors: false,
                scripting_enabled: false,
                iframe_srcdoc: false,
                drop_doctype: false,
                quirks_mode: QuirksMode::NoQuirks,
            },
        };
        html5ever::parse_document(sink, opts)
            .from_utf8()
            .read_from(&mut markup.as_bytes())
            .unwrap();
    }

    /// Parse the input as XML (XHTML), regardless of its content.
    ///
    /// [`parse_into_mutator`](Self::parse_into_mutator) sniffs the content to decide between HTML
    /// and XML parsing, but the sniffing cannot detect all XHTML documents (e.g. ones with an
    /// `<!DOCTYPE html>` doctype). Callers which know the document is XHTML from out-of-band
    /// information (a `Content-Type` header or an `.xht`/`.xhtml` file extension) should use
    /// this method instead.
    pub fn parse_xml_into_mutator<'a, 'd>(mutr: &'a mut DocumentMutator<'d>, xml: &str) {
        let mut sink = DocumentHtmlParser::new(mutr);
        sink.is_xml = true;
        xml5ever::driver::parse_document(sink, Default::default())
            .from_utf8()
            .read_from(&mut xml.as_bytes())
            .unwrap();
    }

    pub fn parse_inner_html_into_mutator<'a, 'd>(
        mutr: &'a mut DocumentMutator<'d>,
        element_id: NodeId,
        html: &str,
    ) {
        // Parse under a detached document: html5ever puts the fragment's root
        // element under the document node, and under the real document the
        // parsed nodes would briefly be connected (visible to mutation
        // observers, which could then hold a node dropped below).
        let scratch = mutr.create_document_node();
        let mut sink = DocumentHtmlParser::new(mutr);
        sink.fragment = true;
        sink.document_node = Some(scratch);

        let opts = ParseOpts {
            tokenizer: TokenizerOpts::default(),
            tree_builder: TreeBuilderOpts {
                exact_errors: false,
                scripting_enabled: false, // Enables parsing of <noscript> tags
                iframe_srcdoc: false,
                drop_doctype: true,
                quirks_mode: QuirksMode::NoQuirks,
            },
        };
        html5ever::driver::parse_fragment_for_element(sink, opts, element_id, false, None)
            .from_utf8()
            .read_from(&mut html.as_bytes())
            .unwrap();

        // html5ever creates a fragment root under the (scratch) document node
        // and parses into it. Move its children to element_id, then drop the
        // scratch document and the root.
        let fragment_root_id = mutr.last_child_id(scratch).unwrap();
        let child_ids = mutr.child_ids(fragment_root_id);
        mutr.append_children(element_id, &child_ids);
        mutr.remove_and_drop_node(scratch);
    }
}

impl<A: DocAccess> TreeSink for HtmlSink<A> {
    type Output = ();

    // we use the ID of the nodes in the tree as the handle
    type Handle = NodeId;

    type ElemName<'a>
        = Ref<'a, QualName>
    where
        Self: 'a;

    fn finish(self) -> Self::Output {
        #[cfg(feature = "tracing")]
        for error in self.errors.borrow().iter() {
            tracing::error!("{error}");
        }
    }

    fn parse_error(&self, msg: Cow<'static, str>) {
        self.errors.borrow_mut().push(msg);
    }

    fn get_document(&self) -> Self::Handle {
        match self.document_node {
            Some(node) => node,
            None => self.with(|m| m.doc.root_node().id),
        }
    }

    fn elem_name<'a>(&'a self, target: &'a Self::Handle) -> Self::ElemName<'a> {
        self.access.element_name(*target)
    }

    fn create_element(
        &self,
        name: QualName,
        attrs: Vec<html5ever::Attribute>,
        _flags: ElementFlags,
    ) -> Self::Handle {
        let attrs = attrs.into_iter().map(html5ever_to_blitz_attr).collect();
        let is_script =
            name.local == html5ever::local_name!("script") && name.ns == html5ever::ns!(html);
        let fragment = self.fragment;
        self.with(|m| {
            let id = m.create_element(name, attrs);
            if is_script {
                // The parser runs the scripts it creates; fragment parsing
                // (innerHTML) never runs them.
                m.mark_script(id, true, fragment);
            }
            id
        })
    }

    fn create_comment(&self, text: StrTendril) -> Self::Handle {
        self.with(|m| m.create_comment_node(text.as_ref()))
    }

    fn create_pi(&self, target: StrTendril, data: StrTendril) -> Self::Handle {
        self.with(|m| m.create_processing_instruction(&target, data.as_ref()))
    }

    fn append(&self, parent_id: &Self::Handle, child: NodeOrText<Self::Handle>) {
        match child {
            NodeOrText::AppendNode(id) => self.with(|m| m.append_children(*parent_id, &[id])),
            // If content to append is text, first attempt to append it to the last child of parent.
            // Else create a new text node and append it to the parent
            NodeOrText::AppendText(text) => self.with(|m| {
                let last_child_id = m.last_child_id(*parent_id);
                let has_appended = if let Some(id) = last_child_id {
                    m.append_text_to_node(id, &text).is_ok()
                } else {
                    false
                };
                if !has_appended {
                    let new_child_id = m.create_text_node(text.as_ref());
                    m.append_children(*parent_id, &[new_child_id]);
                }
            }),
        }
    }

    // Note: The tree builder promises we won't have a text node after the insertion point.
    // https://github.com/servo/html5ever/blob/main/rcdom/lib.rs#L338
    fn append_before_sibling(&self, sibling_id: &Self::Handle, new_node: NodeOrText<Self::Handle>) {
        match new_node {
            NodeOrText::AppendNode(id) => self.with(|m| m.insert_nodes_before(*sibling_id, &[id])),
            // If content to append is text, first attempt to append it to the node before sibling_node
            // Else create a new text node and insert it before sibling_node
            NodeOrText::AppendText(text) => self.with(|m| {
                let previous_sibling_id = m.previous_sibling_id(*sibling_id);
                let has_appended = if let Some(id) = previous_sibling_id {
                    m.append_text_to_node(id, &text).is_ok()
                } else {
                    false
                };
                if !has_appended {
                    let new_child_id = m.create_text_node(text.as_ref());
                    m.insert_nodes_before(*sibling_id, &[new_child_id]);
                }
            }),
        };
    }

    fn append_based_on_parent_node(
        &self,
        element: &Self::Handle,
        prev_element: &Self::Handle,
        child: NodeOrText<Self::Handle>,
    ) {
        if self.with(|m| m.node_has_parent(*element)) {
            self.append_before_sibling(element, child);
        } else {
            self.append(prev_element, child);
        }
    }

    fn append_doctype_to_document(
        &self,
        name: StrTendril,
        public_id: StrTendril,
        system_id: StrTendril,
    ) {
        let document = self.get_document();
        self.with(|m| {
            let doctype = m.create_doctype(&name, &public_id, &system_id);
            m.append_children(document, &[doctype]);
        });
    }

    fn get_template_contents(&self, target: &Self::Handle) -> Self::Handle {
        // Parse a template element's children into its (detached, inert)
        // "template contents" fragment node rather than into the element itself
        self.with(|m| m.template_contents(*target))
    }

    fn same_node(&self, x: &Self::Handle, y: &Self::Handle) -> bool {
        x == y
    }

    fn set_quirks_mode(&self, mode: QuirksMode) {
        self.quirks_mode.set(mode);
    }

    fn add_attrs_if_missing(&self, target: &Self::Handle, attrs: Vec<html5ever::Attribute>) {
        let attrs = attrs.into_iter().map(html5ever_to_blitz_attr).collect();
        self.with(|m| m.add_attrs_if_missing(*target, attrs));
    }

    fn remove_from_parent(&self, target: &Self::Handle) {
        self.with(|m| m.remove_node(*target));
    }

    fn mark_script_already_started(&self, node: &Self::Handle) {
        self.with(|m| m.mark_script(*node, false, true));
    }

    fn reparent_children(&self, old_parent_id: &Self::Handle, new_parent_id: &Self::Handle) {
        self.with(|m| m.reparent_children(*old_parent_id, *new_parent_id));
    }
}

#[test]
fn parses_some_html() {
    use blitz_dom::{BaseDocument, DocumentConfig};

    let html = "<!DOCTYPE html><html><body><h1>hello world</h1></body></html>";
    let mut doc = BaseDocument::new(DocumentConfig::default());
    let mut mutr = doc.mutate();
    let sink = DocumentHtmlParser::new(&mut mutr);

    html5ever::parse_document(sink, Default::default())
        .from_utf8()
        .read_from(&mut html.as_bytes())
        .unwrap();

    drop(mutr);
    doc.print_tree()

    // Now our tree should have some nodes in it
}
