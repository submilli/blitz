//! Stop tree building at the token that exhausts DOM admission.
//!
//! TreeSink cannot return allocation errors. A denied handle completes the
//! current tree-builder token safely. Later tokens are discarded rather than
//! added to the tree builder's open-element/formatting/namespace stacks. The
//! tokenizer may finish its finite input chunk, but allocates no more handles.
//! Streaming drivers refuse later input after admission stops.

use html5ever::tendril::StrTendril;
use html5ever::tokenizer::{BufferQueue, Token, TokenSink, TokenSinkResult, Tokenizer};
use html5ever::tree_builder::TreeBuilder;
use html5ever::{ParseOpts, TokenizerResult};
use xml5ever::tokenizer::{ProcessResult, XmlTokenizer};
use xml5ever::tree_builder::XmlTreeBuilder;

use crate::html_sink::{DocAccess, HtmlSink, ParserHandle};

pub(crate) struct BoundedHtml<A: DocAccess>(pub TreeBuilder<ParserHandle, HtmlSink<A>>);

impl<A: DocAccess> BoundedHtml<A> {
    pub fn exhausted(&self) -> bool {
        self.0.sink.node_limit_exceeded()
    }
}

impl<A: DocAccess> TokenSink for BoundedHtml<A> {
    type Handle = ParserHandle;

    fn process_token(&self, token: Token, line: u64) -> TokenSinkResult<Self::Handle> {
        if self.exhausted() {
            return TokenSinkResult::Continue;
        }
        self.0.process_token(token, line)
    }

    fn end(&self) {
        if !self.exhausted() {
            self.0.end();
        }
    }

    fn adjusted_current_node_present_but_not_in_html_namespace(&self) -> bool {
        self.0
            .adjusted_current_node_present_but_not_in_html_namespace()
    }
}

pub(crate) fn parse_html<A: DocAccess>(
    sink: HtmlSink<A>,
    opts: ParseOpts,
    markup: &str,
    context: Option<ParserHandle>,
) -> Result<(), blitz_dom::NodeBudgetExceeded> {
    let (tree, tokenizer_opts) = match context {
        Some(context) => {
            let tree = TreeBuilder::new_for_fragment(sink, context, None, opts.tree_builder);
            let mut tokenizer_opts = opts.tokenizer;
            tokenizer_opts.initial_state = Some(tree.tokenizer_state_for_context_elem(false));
            (tree, tokenizer_opts)
        }
        None => (TreeBuilder::new(sink, opts.tree_builder), opts.tokenizer),
    };
    let tokenizer = Tokenizer::new(BoundedHtml(tree), tokenizer_opts);
    let input = BufferQueue::default();
    input.push_back(StrTendril::from_slice(markup));
    while !tokenizer.sink.exhausted() {
        if matches!(tokenizer.feed(&input), TokenizerResult::Done) {
            tokenizer.end();
            break;
        }
    }
    if tokenizer.sink.exhausted() {
        Err(blitz_dom::NodeBudgetExceeded)
    } else {
        Ok(())
    }
}

struct BoundedXml<A: DocAccess>(XmlTreeBuilder<ParserHandle, HtmlSink<A>>);

impl<A: DocAccess> xml5ever::tokenizer::TokenSink for BoundedXml<A> {
    type Handle = ParserHandle;

    fn process_token(&self, token: xml5ever::tokenizer::Token) -> ProcessResult<Self::Handle> {
        if self.0.sink.node_limit_exceeded() {
            return ProcessResult::Continue;
        }
        self.0.process_token(token)
    }

    fn end(&self) {
        if !self.0.sink.node_limit_exceeded() {
            self.0.end();
        }
    }
}

pub(crate) fn parse_xml<A: DocAccess>(
    sink: HtmlSink<A>,
    markup: &str,
) -> Result<(), blitz_dom::NodeBudgetExceeded> {
    let tree = XmlTreeBuilder::new(sink, Default::default());
    let tokenizer = XmlTokenizer::new(BoundedXml(tree), Default::default());
    let input = BufferQueue::default();
    input.push_back(StrTendril::from_slice(markup));
    while !tokenizer.sink.0.sink.node_limit_exceeded() {
        if matches!(tokenizer.feed(&input), TokenizerResult::Done) {
            tokenizer.end();
            break;
        }
    }
    if tokenizer.sink.0.sink.node_limit_exceeded() {
        Err(blitz_dom::NodeBudgetExceeded)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html_sink::{DocumentHtmlParser, live_parser_handles};
    use blitz_dom::{BaseDocument, DocumentConfig};

    #[test]
    fn denied_html_tokens_do_not_grow_open_element_handles() {
        let mut doc = BaseDocument::new(DocumentConfig {
            node_limit: Some(1),
            ..Default::default()
        });
        let mut mutator = doc.mutate();
        let sink = DocumentHtmlParser::new(&mut mutator);
        let tokenizer = Tokenizer::new(
            BoundedHtml(TreeBuilder::new(sink, Default::default())),
            Default::default(),
        );
        let input = BufferQueue::default();
        input.push_back(StrTendril::from_slice("<div>"));
        let _ = tokenizer.feed(&input);
        assert!(tokenizer.sink.exhausted());
        let retained = live_parser_handles();
        input.push_back(StrTendril::from_slice(
            &"<div><b><template>".repeat(100_000),
        ));
        let _ = tokenizer.feed(&input);
        assert_eq!(live_parser_handles(), retained);
        drop(tokenizer);
        assert_eq!(live_parser_handles(), 0);
    }

    #[test]
    fn denied_xml_tokens_do_not_grow_element_or_namespace_handles() {
        let mut doc = BaseDocument::new(DocumentConfig {
            node_limit: Some(1),
            ..Default::default()
        });
        let mut mutator = doc.mutate();
        let sink = DocumentHtmlParser::new(&mut mutator);
        let tokenizer = XmlTokenizer::new(
            BoundedXml(XmlTreeBuilder::new(sink, Default::default())),
            Default::default(),
        );
        let input = BufferQueue::default();
        input.push_back(StrTendril::from_slice("<a>"));
        let _ = tokenizer.feed(&input);
        assert!(tokenizer.sink.0.sink.node_limit_exceeded());
        let retained = live_parser_handles();
        input.push_back(StrTendril::from_slice(
            &"<a xmlns:x='urn:x'><x:b>".repeat(100_000),
        ));
        let _ = tokenizer.feed(&input);
        assert_eq!(live_parser_handles(), retained);
        drop(tokenizer);
        assert_eq!(live_parser_handles(), 0);
    }
}
