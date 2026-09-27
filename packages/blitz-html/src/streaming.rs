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

use crate::html_sink::{HtmlSink, SharedDocument};

type Sink = HtmlSink<SharedDocument>;

pub enum ParseStep {
    /// A parser-inserted `<script>` just ended: run it, then call `run` again.
    Script(NodeId),
    /// All input has been parsed.
    Done,
}

pub struct StreamingParser {
    tokenizer: Tokenizer<TreeBuilder<NodeId, Sink>>,
    input: BufferQueue,
    /// Characters from `document.write`, parsed before the rest of the input
    /// (the "insertion point").
    written: BufferQueue,
    end_of_input: bool,
    finished: bool,
}

impl StreamingParser {
    pub fn new(doc: Rc<RefCell<BaseDocument>>) -> Self {
        let sink = HtmlSink::with_access(SharedDocument(doc));
        let tree_builder = TreeBuilder::new(
            sink,
            TreeBuilderOpts {
                exact_errors: false,
                // Scripts run, so `<noscript>` content is raw text.
                scripting_enabled: true,
                iframe_srcdoc: false,
                drop_doctype: false,
                quirks_mode: QuirksMode::NoQuirks,
            },
        );
        Self {
            tokenizer: Tokenizer::new(tree_builder, TokenizerOpts::default()),
            input: BufferQueue::default(),
            written: BufferQueue::default(),
            end_of_input: false,
            finished: false,
        }
    }

    /// Append network input.
    pub fn feed(&mut self, chunk: &str) {
        self.input.push_back(StrTendril::from_slice(chunk));
    }

    /// No more network input will arrive.
    pub fn end_of_input(&mut self) {
        self.end_of_input = true;
    }

    /// `document.write(text)`: insert at the insertion point, ahead of the
    /// remaining input.
    pub fn write(&mut self, text: &str) {
        self.written.push_back(StrTendril::from_slice(text));
    }

    /// Parse only the characters written by `document.write` since the last
    /// call, stopping early at a script end tag. Used while a script is
    /// running, so its writes land in the document before it continues.
    pub fn run_written(&mut self) -> ParseStep {
        Self::feed_until_script(&self.tokenizer, &self.written)
    }

    /// Feed `queue` until it is exhausted or a script ends. Encoding hints
    /// (`<meta charset>`) are ignored: input is already decoded.
    fn feed_until_script(
        tokenizer: &Tokenizer<TreeBuilder<NodeId, Sink>>,
        queue: &BufferQueue,
    ) -> ParseStep {
        loop {
            match tokenizer.feed(queue) {
                TokenizerResult::Script(node) => return ParseStep::Script(node),
                TokenizerResult::Done => return ParseStep::Done,
                TokenizerResult::EncodingIndicator(_) => continue,
            }
        }
    }

    /// Parse until the next script end tag, or until input runs out.
    pub fn run(&mut self) -> ParseStep {
        if self.finished {
            return ParseStep::Done;
        }
        if let ParseStep::Script(node) = self.run_written() {
            return ParseStep::Script(node);
        }
        if let ParseStep::Script(node) = Self::feed_until_script(&self.tokenizer, &self.input) {
            return ParseStep::Script(node);
        }
        if self.end_of_input && self.input.is_empty() && self.written.is_empty() {
            self.tokenizer.end();
            self.finished = true;
        }
        ParseStep::Done
    }

    /// Whether the whole input has been parsed.
    pub fn is_finished(&self) -> bool {
        self.finished
    }
}
