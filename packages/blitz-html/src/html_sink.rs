//! An implementation for Html5ever's sink trait, allowing us to parse HTML into a DOM.

use html5ever::ParseOpts;
use html5ever::tokenizer::TokenizerOpts;
use html5ever::tree_builder::TreeBuilderOpts;
use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use blitz_dom::node::Attribute;
use blitz_dom::{BaseDocument, DocumentMutator, HtmlParserProvider, NodeId};
use html5ever::{
    QualName,
    tendril::StrTendril,
    tree_builder::{ElementFlags, NodeOrText, QuirksMode, TreeSink},
};

/// Convert an html5ever Attribute which uses tendril for its value to a blitz Attribute
/// which uses String.
fn html5ever_to_blitz_attr(attr: html5ever::Attribute) -> Attribute {
    Attribute {
        name: attr.name,
        value: attr.value.to_string().into(),
    }
}

#[derive(Copy, Clone, Default, Debug)]
pub struct HtmlProvider;

impl HtmlParserProvider for HtmlProvider {
    fn try_parse_inner_html(
        &self,
        mutr: &mut DocumentMutator<'_>,
        element_id: NodeId,
        html: &str,
    ) -> Result<(), blitz_dom::NodeBudgetExceeded> {
        DocumentHtmlParser::try_parse_inner_html_into_mutator(mutr, element_id, html)
    }

    fn try_parse_into_document_node(
        &self,
        mutr: &mut DocumentMutator<'_>,
        document: NodeId,
        markup: &str,
        xml: bool,
    ) -> Result<(), blitz_dom::NodeBudgetExceeded> {
        DocumentHtmlParser::try_parse_into_document_node(mutr, document, markup, xml)
    }

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
}

/// A mutator borrowed for the whole parse (one-shot parsing).
pub struct BorrowedMutator<'m, 'doc>(RefCell<&'m mut DocumentMutator<'doc>>);

impl DocAccess for BorrowedMutator<'_, '_> {
    fn with_mutator<R>(&self, f: impl FnOnce(&mut DocumentMutator<'_>) -> R) -> R {
        let mut guard = self.0.borrow_mut();
        f(&mut guard)
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
    /// Once admission fails, this parse stops changing the document.
    exhausted: Cell<bool>,
    /// Nodes created by this parse, including handles not yet attached when
    /// admission failed. A transactional caller can release all of them.
    allocations: Option<Rc<RefCell<Vec<NodeId>>>>,
}

/// An HTML tree-builder handle remains valid after node admission fails. Its
/// name is retained without an arena allocation; it can never alias a live node.
#[derive(Clone)]
pub struct ParserHandle(Rc<ParserNode>);

struct ParserNode {
    lease: Option<blitz_dom::NodeLease>,
    parser_form: Cell<Option<NodeId>>,
    name: QualName,
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
    fn new(lease: Option<blitz_dom::NodeLease>, name: QualName) -> Self {
        #[cfg(test)]
        LIVE_HANDLES.with(|count| count.set(count.get() + 1));
        Self(Rc::new(ParserNode {
            lease,
            name,
            parser_form: Cell::new(None),
        }))
    }

    pub(crate) fn node_id(&self) -> Option<NodeId> {
        self.0.lease.as_ref().map(blitz_dom::NodeLease::id)
    }

    fn non_element(lease: Option<blitz_dom::NodeLease>) -> Self {
        Self::new(lease, QualName::new(None, html5ever::ns!(html), "".into()))
    }
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
            exhausted: Cell::new(false),
            allocations: None,
        }
    }

    fn with<R>(&self, f: impl FnOnce(&mut DocumentMutator<'_>) -> R) -> R {
        self.access.with_mutator(f)
    }

    fn handle(&self, id: Option<NodeId>, name: QualName) -> ParserHandle {
        ParserHandle::new(id.map(|id| self.with(|m| m.doc.lease_node(id))), name)
    }

    fn non_element(&self, id: Option<NodeId>) -> ParserHandle {
        ParserHandle::non_element(id.map(|id| self.with(|m| m.doc.lease_node(id))))
    }

    /// Whether this parse has stopped admitting nodes.
    pub fn node_limit_exceeded(&self) -> bool {
        self.exhausted.get()
    }

    pub(crate) fn finish_xml(&self) -> Result<(), blitz_dom::NodeBudgetExceeded> {
        if self.errors.borrow().is_empty() {
            return Ok(());
        }
        self.with(|m| {
            let document = self.document_node.unwrap_or_else(|| m.doc.root_node().id);
            m.document_metadata_mut(document).xml_parse_error = true;
            let children = m
                .doc
                .get_node(document)
                .expect("parser document remains live")
                .children
                .to_vec();
            for child in children {
                m.remove_and_drop_node(child);
            }
            m.doc.check_node_allocation(2)?;
            let error = m.try_create_element(
                QualName::new(
                    None,
                    "http://www.mozilla.org/newlayout/xml/parsererror.xml".into(),
                    "parsererror".into(),
                ),
                Vec::new(),
            )?;
            let text = m.try_create_text_node("XML parsing error")?;
            m.append_children(error, &[text]);
            m.append_children(document, &[error]);
            Ok(())
        })
    }

    fn allocate(
        &self,
        create: impl FnOnce(&mut DocumentMutator<'_>) -> Result<NodeId, blitz_dom::NodeBudgetExceeded>,
    ) -> Option<NodeId> {
        if self.exhausted.get() {
            return None;
        }
        match self.with(create) {
            Ok(id) => {
                if let Some(allocations) = &self.allocations {
                    allocations.borrow_mut().push(id);
                }
                Some(id)
            }
            Err(_) => {
                self.exhausted.set(true);
                None
            }
        }
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
            let _ = crate::bounded_parser::parse_html(sink, opts, html, None);
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
        let _ = Self::try_parse_into_document_node(mutr, document, markup, xml);
    }

    /// Report admission exhaustion to callers creating an inert document.
    pub fn try_parse_into_document_node(
        mutr: &mut DocumentMutator<'_>,
        document: NodeId,
        markup: &str,
        xml: bool,
    ) -> Result<(), blitz_dom::NodeBudgetExceeded> {
        let metadata = mutr.document_metadata_mut(document);
        if xml && metadata.document_type.is_html() {
            metadata.document_type = blitz_dom::DocumentType::Xml;
        }
        metadata.quirks = false;
        metadata.xml_parse_error = false;
        let mut sink = DocumentHtmlParser::new(mutr);
        sink.document_node = Some(document);
        sink.fragment = true;
        let allocations = Rc::new(RefCell::new(Vec::new()));
        sink.allocations = Some(allocations.clone());
        if xml {
            sink.is_xml = true;
            let result = crate::bounded_parser::parse_xml(sink, markup);
            if result.is_err() {
                discard_allocations(mutr, &allocations.borrow());
            }
            return result;
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
        let result = crate::bounded_parser::parse_html(sink, opts, markup, None);
        if result.is_err() {
            discard_allocations(mutr, &allocations.borrow());
        }
        result
    }

    /// Parse the input as XML (XHTML), regardless of its content.
    ///
    /// [`parse_into_mutator`](Self::parse_into_mutator) sniffs the content to decide between HTML
    /// and XML parsing, but the sniffing cannot detect all XHTML documents (e.g. ones with an
    /// `<!DOCTYPE html>` doctype). Callers which know the document is XHTML from out-of-band
    /// information (a `Content-Type` header or an `.xht`/`.xhtml` file extension) should use
    /// this method instead.
    pub fn parse_xml_into_mutator<'a, 'd>(mutr: &'a mut DocumentMutator<'d>, xml: &str) {
        let root = mutr.doc.root_node().id;
        mutr.document_metadata_mut(root).document_type = blitz_dom::DocumentType::Xml;
        let mut sink = DocumentHtmlParser::new(mutr);
        sink.is_xml = true;
        let _ = crate::bounded_parser::parse_xml(sink, xml);
    }

    pub fn parse_inner_html_into_mutator<'a, 'd>(
        mutr: &'a mut DocumentMutator<'d>,
        element_id: NodeId,
        html: &str,
    ) {
        let _ = Self::try_parse_inner_html_into_mutator(mutr, element_id, html);
    }

    /// Commit the parsed fragment only if all of it fits the node budget.
    pub fn try_parse_inner_html_into_mutator(
        mutr: &mut DocumentMutator<'_>,
        element_id: NodeId,
        html: &str,
    ) -> Result<(), blitz_dom::NodeBudgetExceeded> {
        // Parse under a detached document: html5ever puts the fragment's root
        // element under the document node, and under the real document the
        // parsed nodes would briefly be connected (visible to mutation
        // observers, which could then hold a node dropped below).
        let scratch = mutr.try_create_document_node()?;
        let context_name = mutr.element_name(element_id).cloned();
        let Some(context_name) = context_name else {
            mutr.remove_and_drop_node(scratch);
            return Ok(());
        };
        let context = ParserHandle::new(Some(mutr.doc.lease_node(element_id)), context_name);
        let mut sink = DocumentHtmlParser::new(mutr);
        sink.fragment = true;
        sink.document_node = Some(scratch);
        let allocations = Rc::new(RefCell::new(Vec::new()));
        sink.allocations = Some(allocations.clone());

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
        let result = crate::bounded_parser::parse_html(sink, opts, html, Some(context));
        if result.is_err() {
            discard_allocations(mutr, &allocations.borrow());
        }

        // html5ever creates a fragment root under the (scratch) document node
        // and parses into it. Move its children to element_id, then drop the
        // scratch document and the root.
        if result.is_ok()
            && let Some(fragment_root_id) = mutr.last_child_id(scratch)
        {
            let child_ids = mutr.child_ids(fragment_root_id);
            mutr.append_children(element_id, &child_ids);
        }
        mutr.remove_and_drop_node(scratch);
        result
    }
}

impl<A: DocAccess> TreeSink for HtmlSink<A> {
    type Output = ();
    type Handle = ParserHandle;
    type ElemName<'a>
        = &'a QualName
    where
        Self: 'a;

    fn finish(self) -> Self::Output {}

    fn parse_error(&self, msg: Cow<'static, str>) {
        // Errors are diagnostics, not an unbounded copy of hostile markup.
        let mut errors = self.errors.borrow_mut();
        if errors.len() < 128 {
            errors.push(msg);
        }
    }

    fn get_document(&self) -> Self::Handle {
        self.non_element(Some(
            self.document_node
                .unwrap_or_else(|| self.with(|m| m.doc.root_node().id)),
        ))
    }

    fn elem_name<'a>(&'a self, target: &'a Self::Handle) -> Self::ElemName<'a> {
        &target.0.name
    }

    fn create_element(
        &self,
        name: QualName,
        attrs: Vec<html5ever::Attribute>,
        _flags: ElementFlags,
    ) -> Self::Handle {
        let is_script =
            name.local == html5ever::local_name!("script") && name.ns == html5ever::ns!(html);
        let id = self.allocate(|m| {
            let attrs = attrs.into_iter().map(html5ever_to_blitz_attr).collect();
            let id = m.try_create_element(name.clone(), attrs)?;
            if is_script {
                m.mark_script(id, true, self.fragment);
            }
            Ok(id)
        });
        self.handle(id, name)
    }

    fn create_comment(&self, text: StrTendril) -> Self::Handle {
        self.non_element(self.allocate(|m| m.try_create_comment_node(text.as_ref())))
    }

    fn create_pi(&self, target: StrTendril, data: StrTendril) -> Self::Handle {
        self.non_element(
            self.allocate(|m| m.try_create_processing_instruction(&target, data.as_ref())),
        )
    }

    fn append(&self, parent: &Self::Handle, child: NodeOrText<Self::Handle>) {
        let Some(parent) = parent.node_id().filter(|_| !self.exhausted.get()) else {
            return;
        };
        match child {
            NodeOrText::AppendNode(child) => {
                if let Some(id) = child.node_id() {
                    self.with(|m| {
                        m.append_children(parent, &[id]);
                        if let Some(form) = child.0.parser_form.take() {
                            m.associate_parser_form(id, form);
                        }
                    });
                }
            }
            NodeOrText::AppendText(text) => {
                let appended = self.with(|m| {
                    m.last_child_id(parent)
                        .is_some_and(|last| m.append_text_to_node(last, &text).is_ok())
                });
                if !appended {
                    if let Some(child) = self.allocate(|m| m.try_create_text_node(text.as_ref())) {
                        self.with(|m| m.append_children(parent, &[child]));
                    }
                }
            }
        }
    }

    fn append_before_sibling(&self, sibling: &Self::Handle, child: NodeOrText<Self::Handle>) {
        let Some(sibling) = sibling.node_id().filter(|_| !self.exhausted.get()) else {
            return;
        };
        match child {
            NodeOrText::AppendNode(child) => {
                if let Some(id) = child.node_id() {
                    self.with(|m| {
                        m.insert_nodes_before(sibling, &[id]);
                        if let Some(form) = child.0.parser_form.take() {
                            m.associate_parser_form(id, form);
                        }
                    });
                }
            }
            NodeOrText::AppendText(text) => {
                let appended = self.with(|m| {
                    m.previous_sibling_id(sibling)
                        .is_some_and(|previous| m.append_text_to_node(previous, &text).is_ok())
                });
                if !appended {
                    if let Some(child) = self.allocate(|m| m.try_create_text_node(text.as_ref())) {
                        self.with(|m| m.insert_nodes_before(sibling, &[child]));
                    }
                }
            }
        }
    }

    fn append_based_on_parent_node(
        &self,
        element: &Self::Handle,
        previous: &Self::Handle,
        child: NodeOrText<Self::Handle>,
    ) {
        if self.exhausted.get() {
            return;
        }
        if element
            .node_id()
            .is_some_and(|id| self.with(|m| m.node_has_parent(id)))
        {
            self.append_before_sibling(element, child);
        } else {
            self.append(previous, child);
        }
    }

    fn append_doctype_to_document(
        &self,
        name: StrTendril,
        public_id: StrTendril,
        system_id: StrTendril,
    ) {
        let child = self.allocate(|m| m.try_create_doctype(&name, &public_id, &system_id));
        if let Some(child) = child {
            self.append(
                &self.get_document(),
                NodeOrText::AppendNode(self.non_element(Some(child))),
            );
        }
    }

    fn get_template_contents(&self, target: &Self::Handle) -> Self::Handle {
        if let Some(existing) = target
            .node_id()
            .and_then(|id| self.with(|m| m.try_template_contents(id)))
        {
            return self.non_element(Some(existing));
        }
        let id = target
            .node_id()
            .and_then(|id| self.allocate(|m| m.try_ensure_template_contents(id)));
        self.non_element(id)
    }

    fn same_node(&self, x: &Self::Handle, y: &Self::Handle) -> bool {
        match (x.node_id(), y.node_id()) {
            (Some(x), Some(y)) => x == y,
            _ => Rc::ptr_eq(&x.0, &y.0),
        }
    }

    fn set_quirks_mode(&self, mode: QuirksMode) {
        self.quirks_mode.set(mode);
        self.with(|m| {
            let document = self.document_node.unwrap_or_else(|| m.doc.root_node().id);
            m.document_metadata_mut(document).quirks = mode == QuirksMode::Quirks;
        });
    }

    fn add_attrs_if_missing(&self, target: &Self::Handle, attrs: Vec<html5ever::Attribute>) {
        if let Some(id) = target.node_id().filter(|_| !self.exhausted.get()) {
            let attrs = attrs.into_iter().map(html5ever_to_blitz_attr).collect();
            self.with(|m| m.add_attrs_if_missing(id, attrs));
        }
    }

    fn associate_with_form(
        &self,
        target: &Self::Handle,
        form: &Self::Handle,
        _nodes: (&Self::Handle, Option<&Self::Handle>),
    ) {
        // Fragment insertion resets parser owners. Inert documents have no
        // browsing context, matching Chrome's frame check for this association.
        if !self.fragment {
            target.0.parser_form.set(form.node_id());
        }
    }

    fn remove_from_parent(&self, target: &Self::Handle) {
        if let Some(id) = target.node_id().filter(|_| !self.exhausted.get()) {
            self.with(|m| m.remove_node(id));
        }
    }

    fn mark_script_already_started(&self, node: &Self::Handle) {
        if let Some(id) = node.node_id().filter(|_| !self.exhausted.get()) {
            self.with(|m| m.mark_script(id, false, true));
        }
    }

    fn reparent_children(&self, old: &Self::Handle, new: &Self::Handle) {
        if self.exhausted.get() {
            return;
        }
        if let (Some(old), Some(new)) = (old.node_id(), new.node_id()) {
            self.with(|m| m.reparent_children(old, new));
        }
    }
}

fn discard_allocations(m: &mut DocumentMutator<'_>, allocated: &[NodeId]) {
    for &id in allocated {
        if m.doc.get_node(id).is_some() {
            m.remove_and_drop_node(id);
        }
    }
}

#[test]
fn parses_some_html() {
    use blitz_dom::{BaseDocument, DocumentConfig};

    let html = "<!DOCTYPE html><html><body><h1>hello world</h1></body></html>";
    let mut doc = BaseDocument::new(DocumentConfig::default());
    let mut mutr = doc.mutate();
    let sink = DocumentHtmlParser::new(&mut mutr);

    let _ = crate::bounded_parser::parse_html(sink, Default::default(), html, None);

    drop(mutr);
    doc.print_tree()

    // Now our tree should have some nodes in it
}
