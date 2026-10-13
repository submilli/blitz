//! Incremental HTML parsing that pauses at scripts.
//!
//! The HTML parser must stop at each `</script>` so the script can run
//! against the document parsed so far (and may `document.write` into the
//! input stream) before parsing continues. [`StreamingParser`] exposes that
//! as a loop: call [`StreamingParser::run`] until it returns
//! [`ParseStep::Done`], running each [`ParseStep::Script`] in between.
//!
//! It also stops before creating an element whose custom element
//! definition exists, when given a [`DefinitionLookup`], so the embedder can
//! construct it: see [`ParseStep::CustomElement`].
//!
//! The document is shared (`Rc<RefCell<_>>`) and only borrowed per tree
//! operation, so it is free for scripts while the parser is paused.

use std::cell::RefCell;
use std::rc::Rc;

use blitz_dom::{BaseDocument, NodeId};
use html5ever::tendril::StrTendril;
use html5ever::tokenizer::{BufferQueue, Tokenizer, TokenizerOpts};
use html5ever::tree_builder::{QuirksMode, TreeBuilder, TreeBuilderOpts};
use html5ever::{LocalName, TokenizerResult};

use crate::bounded_parser::BoundedHtml;
use crate::custom_element_pause::DefinitionLookup;
use crate::html_sink::{HtmlSink, SharedDocument};

pub enum ParseStep {
    /// A parser-inserted `<script>` just ended: run it, then call `run` again.
    Script(NodeId),
    /// The parser is about to create an HTML element of this name, which has
    /// a custom element definition. Construct it (HTML "create an element
    /// for a token" steps 7–8), hand it to
    /// [`StreamingParser::construct_custom_element`], which appends the
    /// token's attributes (step 9), run their reactions (step 10), then call
    /// `run` again.
    CustomElement(LocalName),
    /// The parser created and inserted that element: run the reactions of
    /// its insertion, then call `run` again.
    CustomElementCreated,
    /// All input has been parsed.
    Done,
}

pub struct StreamingParser {
    tokenizer: Option<Tokenizer<BoundedHtml<SharedDocument>>>,
    input: BufferQueue,
    /// Characters from `document.write` before the insertion point, parsed
    /// before the rest of the input.
    written: BufferQueue,
    /// Written characters after each saved insertion point, innermost last.
    /// They follow `written` again once the script that saved it returns.
    held: Vec<BufferQueue>,
    end_of_input: bool,
}

impl StreamingParser {
    pub fn new(doc: Rc<RefCell<BaseDocument>>) -> Self {
        Self::with_target(doc, None)
    }

    /// Incrementally parse a detached HTML document without activating scripts.
    /// The caller owns replacement/EOF; existing nodes survive subsequent chunks.
    pub fn for_document_node(doc: Rc<RefCell<BaseDocument>>, document: NodeId) -> Self {
        Self::with_target(doc, Some(document))
    }

    fn with_target(doc: Rc<RefCell<BaseDocument>>, document: Option<NodeId>) -> Self {
        let mut sink = HtmlSink::with_access(SharedDocument(doc));
        sink.document_node = document;
        sink.fragment = document.is_some();
        let tree_builder = TreeBuilder::new(
            sink,
            TreeBuilderOpts {
                exact_errors: false,
                // Only the active document runs scripts and treats noscript as raw text.
                scripting_enabled: document.is_none(),
                iframe_srcdoc: false,
                drop_doctype: false,
                quirks_mode: QuirksMode::NoQuirks,
            },
        );
        Self {
            tokenizer: Some(Tokenizer::new(
                BoundedHtml::new(tree_builder),
                TokenizerOpts::default(),
            )),
            input: BufferQueue::default(),
            written: BufferQueue::default(),
            held: Vec::new(),
            end_of_input: false,
        }
    }

    /// Stop before creating elements whose name `lookup` reports defined.
    pub fn set_definition_lookup(&mut self, lookup: DefinitionLookup) {
        if let Some(tokenizer) = &mut self.tokenizer {
            tokenizer.sink.set_definition_lookup(lookup);
        }
    }

    /// After [`ParseStep::CustomElement`]: create that element as `element`,
    /// the custom element the embedder constructed, and append the start
    /// tag's attributes to it now. Refused (`false`) once the parser has
    /// processed that tag (a nested run did), or unless `element` is a live
    /// HTML element of its name; the parser then creates the element itself.
    pub fn construct_custom_element(&mut self, element: NodeId) -> bool {
        self.tokenizer
            .as_ref()
            .is_some_and(|tokenizer| tokenizer.sink.construct(element))
    }

    /// The custom element the parser stopped before
    /// ([`ParseStep::CustomElement`]), until it creates it.
    pub fn paused_custom_element(&self) -> Option<LocalName> {
        self.tokenizer
            .as_ref()
            .and_then(|tokenizer| tokenizer.sink.paused_custom_element())
    }

    /// Elements the parser gave an `on*` attribute since the last call, which
    /// may need their event handlers activated.
    pub fn take_handler_elements(&mut self) -> Vec<NodeId> {
        self.tokenizer
            .as_ref()
            .map(|tokenizer| tokenizer.sink.sink().take_handler_elements())
            .unwrap_or_default()
    }

    /// Append network input.
    pub fn feed(&mut self, chunk: &str) {
        if self
            .tokenizer
            .as_ref()
            .is_none_or(|tokenizer| tokenizer.sink.exhausted())
        {
            return;
        }
        self.input.push_back(StrTendril::from_slice(chunk));
    }

    /// No more network input will arrive.
    pub fn end_of_input(&mut self) {
        self.end_of_input = true;
    }

    /// `document.write(text)`: insert at the insertion point, ahead of the
    /// remaining input.
    pub fn write(&mut self, text: &str) {
        if self
            .tokenizer
            .as_ref()
            .is_none_or(|tokenizer| tokenizer.sink.exhausted())
        {
            return;
        }
        self.written.push_back(StrTendril::from_slice(text));
    }

    /// A parser script is about to run: move the insertion point to just
    /// before the next unparsed character, so its writes precede characters
    /// earlier writes left unparsed (HTML "</script>" in the "text" insertion
    /// mode, and executing a pending parsing-blocking script). The caller
    /// bounds script nesting and pairs each call with
    /// [`StreamingParser::restore_insertion_point`].
    pub fn save_insertion_point(&mut self) {
        self.held.push(std::mem::take(&mut self.written));
    }

    /// The script that saved the insertion point returned: the characters
    /// after it follow those its writes left unparsed. Does nothing without a
    /// saved insertion point (or once an exhausted parser dropped its input);
    /// an unpaired call is a caller bug.
    pub fn restore_insertion_point(&mut self) {
        let Some(after) = self.held.pop() else {
            return;
        };
        while let Some(chunk) = after.pop_front() {
            self.written.push_back(chunk);
        }
    }

    /// Parse only the characters written by `document.write` up to the
    /// insertion point, stopping early at a script end tag or custom element
    /// creation. Used while a script is running, so its writes land in the
    /// document before it continues.
    pub fn run_written(&mut self) -> ParseStep {
        let Some(tokenizer) = &self.tokenizer else {
            return ParseStep::Done;
        };
        if tokenizer.sink.create_paused() {
            self.stop_if_exhausted();
            return ParseStep::CustomElementCreated;
        }
        let step = Self::feed_until_pause(tokenizer, &self.written);
        self.stop_if_exhausted();
        step
    }

    fn stop_if_exhausted(&mut self) {
        if self
            .tokenizer
            .as_ref()
            .is_some_and(|tokenizer| tokenizer.sink.exhausted())
        {
            self.tokenizer.take();
            while self.input.pop_front().is_some() {}
            while self.written.pop_front().is_some() {}
            self.held.clear();
        }
    }

    /// Feed `queue` until it is exhausted, a script ends or a custom element
    /// is about to be created. Encoding hints (`<meta charset>`) are ignored:
    /// input is already decoded.
    fn feed_until_pause(
        tokenizer: &Tokenizer<BoundedHtml<SharedDocument>>,
        queue: &BufferQueue,
    ) -> ParseStep {
        loop {
            if tokenizer.sink.exhausted() {
                return ParseStep::Done;
            }
            match tokenizer.feed(queue) {
                TokenizerResult::Script(node) => {
                    // A paused start tag stops the tokenizer as a script
                    // does; check it first.
                    if let Some(name) = tokenizer.sink.paused_custom_element() {
                        return ParseStep::CustomElement(name);
                    }
                    if let Some(id) = node.node_id().filter(|_| !tokenizer.sink.exhausted()) {
                        return ParseStep::Script(id);
                    }
                }
                TokenizerResult::Done => return ParseStep::Done,
                TokenizerResult::EncodingIndicator(_) => continue,
            }
        }
    }

    /// Parse until the next script end tag or custom element creation (see
    /// [`ParseStep`]), or until input runs out. While an insertion point is
    /// saved, the input after it waits for its script, so only written
    /// characters before it are parsed.
    pub fn run(&mut self) -> ParseStep {
        if self.is_finished() {
            return ParseStep::Done;
        }
        let step = self.run_written();
        if !matches!(step, ParseStep::Done) {
            return step;
        }
        if !self.held.is_empty() {
            return ParseStep::Done;
        }
        let Some(tokenizer) = &self.tokenizer else {
            return ParseStep::Done;
        };
        let step = Self::feed_until_pause(tokenizer, &self.input);
        if !matches!(step, ParseStep::Done) {
            return step;
        }
        self.stop_if_exhausted();
        if self.end_of_input
            && self.input.is_empty()
            && self.written.is_empty()
            && self.held.is_empty()
            && let Some(tokenizer) = self.tokenizer.take()
        {
            // EOF can leave formatting/form handles in the tree builder. Release
            // all of them now, even when the finished parser itself is retained.
            tokenizer.end();
        }
        ParseStep::Done
    }

    /// Whether the whole input has been parsed.
    pub fn is_finished(&self) -> bool {
        self.tokenizer.is_none()
    }
}
