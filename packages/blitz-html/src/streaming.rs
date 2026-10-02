//! Incremental HTML parsing that pauses at scripts.
//!
//! The HTML parser must stop at each `</script>` so the script can run
//! against the document parsed so far (and may `document.write` into the
//! input stream) before parsing continues. [`StreamingParser`] exposes that
//! as a loop: call [`StreamingParser::run`] until it returns
//! [`ParseStep::Done`], running each [`ParseStep::Script`] in between.
//!
//! The document is shared (`Rc<RefCell<_>>`) and only borrowed per tree
//! operation, so it is free for scripts while the parser is paused.

use std::cell::RefCell;
use std::rc::Rc;

use blitz_dom::{BaseDocument, NodeId};
use html5ever::TokenizerResult;
use html5ever::tendril::StrTendril;
use html5ever::tokenizer::{BufferQueue, Tokenizer, TokenizerOpts};
use html5ever::tree_builder::{QuirksMode, TreeBuilder, TreeBuilderOpts};

use crate::bounded_parser::BoundedHtml;
use crate::html_sink::{HtmlSink, SharedDocument};

type Sink = BoundedHtml<SharedDocument>;

pub enum ParseStep {
    /// A parser-inserted `<script>` just ended: run it, then call `run` again.
    Script(NodeId),
    /// All input has been parsed.
    Done,
}

pub struct StreamingParser {
    tokenizer: Option<Tokenizer<Sink>>,
    input: BufferQueue,
    /// Characters from `document.write`, parsed before the rest of the input
    /// (the "insertion point").
    written: BufferQueue,
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
                BoundedHtml(tree_builder),
                TokenizerOpts::default(),
            )),
            input: BufferQueue::default(),
            written: BufferQueue::default(),
            end_of_input: false,
        }
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

    /// Parse only the characters written by `document.write` since the last
    /// call, stopping early at a script end tag. Used while a script is
    /// running, so its writes land in the document before it continues.
    pub fn run_written(&mut self) -> ParseStep {
        let Some(tokenizer) = &self.tokenizer else {
            return ParseStep::Done;
        };
        let step = Self::feed_until_script(tokenizer, &self.written);
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
        }
    }

    /// Feed `queue` until it is exhausted or a script ends. Encoding hints
    /// (`<meta charset>`) are ignored: input is already decoded.
    fn feed_until_script(tokenizer: &Tokenizer<Sink>, queue: &BufferQueue) -> ParseStep {
        loop {
            if tokenizer.sink.exhausted() {
                return ParseStep::Done;
            }
            match tokenizer.feed(queue) {
                TokenizerResult::Script(node) => {
                    if let Some(id) = node.node_id().filter(|_| !tokenizer.sink.exhausted()) {
                        return ParseStep::Script(id);
                    }
                }
                TokenizerResult::Done => return ParseStep::Done,
                TokenizerResult::EncodingIndicator(_) => continue,
            }
        }
    }

    /// Parse until the next script end tag, or until input runs out.
    pub fn run(&mut self) -> ParseStep {
        if self.is_finished() {
            return ParseStep::Done;
        }
        if let ParseStep::Script(node) = self.run_written() {
            return ParseStep::Script(node);
        }
        let Some(tokenizer) = &self.tokenizer else {
            return ParseStep::Done;
        };
        if let ParseStep::Script(node) = Self::feed_until_script(tokenizer, &self.input) {
            return ParseStep::Script(node);
        }
        self.stop_if_exhausted();
        if self.end_of_input
            && self.input.is_empty()
            && self.written.is_empty()
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
