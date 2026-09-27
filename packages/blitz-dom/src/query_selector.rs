use std::collections::HashSet;

use blitz_traits::node_id::NodeId;
use cssparser::{ParseErrorKind, SourceLocation};
use selectors::SelectorList;
use smallvec::SmallVec;
use style::dom::{TDocument, TNode};
use style::dom_apis::{
    MayUseInvalidation, QueryAll, QueryFirst, element_closest, element_matches, query_selector,
};
use style::selector_parser::{SelectorImpl, SelectorParser};
use style_traits::{ParseError, StyleParseErrorKind};

use crate::{BaseDocument, Node};

impl BaseDocument {
    /// Find the node with the specified id attribute (if one exists).
    /// If multiple nodes have the same id, the first in tree order is returned.
    pub fn get_element_by_id(&self, id: &str) -> Option<NodeId> {
        let candidates = self.nodes_to_id.get(id)?;
        match candidates.len() {
            0 => None,
            1 => candidates.iter().next().copied(),
            _ => self.first_in_tree_order(candidates),
        }
    }

    /// Find the first of `candidates` in tree order
    fn first_in_tree_order(&self, candidates: &HashSet<NodeId>) -> Option<NodeId> {
        let mut stack = vec![self.root_node_id];
        while let Some(node_id) = stack.pop() {
            if candidates.contains(&node_id) {
                return Some(node_id);
            }
            stack.extend(self.nodes[node_id].children.iter().rev().copied());
        }
        None
    }

    /// Add a node to the id-to-node map
    pub(crate) fn add_to_id_map(&mut self, id: &str, node_id: NodeId) {
        if id.is_empty() {
            return;
        }
        self.nodes_to_id
            .entry(id.to_string())
            .or_default()
            .insert(node_id);
    }

    /// Remove a node from the id-to-node map
    pub(crate) fn remove_from_id_map(&mut self, id: &str, node_id: NodeId) {
        if let Some(node_ids) = self.nodes_to_id.get_mut(id) {
            node_ids.remove(&node_id);
            if node_ids.is_empty() {
                self.nodes_to_id.remove(id);
            }
        }
    }

    /// Find the first node that matches the selector specified as a string
    /// Returns:
    ///   - Err(_) if parsing the selector fails
    ///   - Ok(None) if nothing matches
    ///   - Ok(Some(node_id)) with the first node ID that matches if one is found
    pub fn query_selector<'input>(
        &self,
        selector: &'input str,
    ) -> Result<Option<NodeId>, ParseError<'input>> {
        self.query_selector_in(self.root_node_id, selector)
    }

    /// Find the first descendant of `scope` that matches the selector specified
    /// as a string.
    ///
    /// The scope node itself is never matched. Selector parts may match nodes
    /// outside the scope while evaluating relationships between descendants.
    ///
    /// Returns:
    ///   - `Err(_)` if parsing the selector fails
    ///   - `Ok(None)` if nothing matches
    ///   - `Ok(Some(node_id))` with the first matching descendant ID otherwise
    pub fn query_selector_in<'input>(
        &self,
        scope: NodeId,
        selector: &'input str,
    ) -> Result<Option<NodeId>, ParseError<'input>> {
        let selector_list = self.try_parse_selector_list(selector)?;
        Ok(self.query_selector_in_raw(scope, &selector_list))
    }

    /// Find the first descendant of the document root that matches the
    /// selector(s) specified in `selector_list`.
    ///
    /// The document root itself is never matched. Selector parts may match
    /// nodes outside the scope while evaluating relationships between
    /// descendants.
    pub fn query_selector_raw(&self, selector_list: &SelectorList<SelectorImpl>) -> Option<NodeId> {
        self.query_selector_in_raw(self.root_node_id, selector_list)
    }

    /// Find the first descendant of `scope` that matches the selector(s)
    /// specified in `selector_list`.
    ///
    /// The scope node itself is never matched. Selector parts may match nodes
    /// outside the scope while evaluating relationships between descendants.
    pub fn query_selector_in_raw(
        &self,
        scope: NodeId,
        selector_list: &SelectorList<SelectorImpl>,
    ) -> Option<NodeId> {
        let root_node = &self.nodes[scope];
        let mut result = None;
        query_selector::<&Node, QueryFirst>(
            root_node,
            selector_list,
            &mut result,
            self.may_use_invalidation_for(scope),
        );

        result.map(|node| node.id)
    }

    /// Find all nodes that match the selector specified as a string
    /// Returns:
    ///   - `Err(_)` if parsing the selector fails
    ///   - `Ok(SmallVec<usize>)` with all matching nodes otherwise
    pub fn query_selector_all<'input>(
        &self,
        selector: &'input str,
    ) -> Result<SmallVec<[NodeId; 32]>, ParseError<'input>> {
        self.query_selector_all_in(self.root_node_id, selector)
    }

    /// Find all descendants of `scope` that match the selector specified as a
    /// string, in tree order.
    ///
    /// The scope node itself is never matched. Selector parts may match nodes
    /// outside the scope while evaluating relationships between descendants.
    ///
    /// Returns:
    ///   - `Err(_)` if parsing the selector fails
    ///   - `Ok(_)` with all matching descendant IDs otherwise
    pub fn query_selector_all_in<'input>(
        &self,
        scope: NodeId,
        selector: &'input str,
    ) -> Result<SmallVec<[NodeId; 32]>, ParseError<'input>> {
        let selector_list = self.try_parse_selector_list(selector)?;
        Ok(self.query_selector_all_in_raw(scope, &selector_list))
    }

    /// Find all descendants of the document root that match the selector(s)
    /// specified in `selector_list`, in tree order.
    ///
    /// The document root itself is never matched. Selector parts may match
    /// nodes outside the scope while evaluating relationships between
    /// descendants.
    pub fn query_selector_all_raw(
        &self,
        selector_list: &SelectorList<SelectorImpl>,
    ) -> SmallVec<[NodeId; 32]> {
        self.query_selector_all_in_raw(self.root_node_id, selector_list)
    }

    /// Find all descendants of `scope` that match the selector(s) specified in
    /// `selector_list`, in tree order.
    ///
    /// The scope node itself is never matched. Selector parts may match nodes
    /// outside the scope while evaluating relationships between descendants.
    pub fn query_selector_all_in_raw(
        &self,
        scope: NodeId,
        selector_list: &SelectorList<SelectorImpl>,
    ) -> SmallVec<[NodeId; 32]> {
        let root_node = &self.nodes[scope];
        let mut results = SmallVec::new();
        query_selector::<&Node, QueryAll>(
            root_node,
            selector_list,
            &mut results,
            self.may_use_invalidation_for(scope),
        );

        results.iter().map(|node| node.id).collect()
    }

    fn may_use_invalidation_for(&self, scope: NodeId) -> MayUseInvalidation {
        if scope == self.root_node_id {
            MayUseInvalidation::Yes
        } else {
            MayUseInvalidation::No
        }
    }

    /// Test whether the node identified by `node_id` matches the selector
    /// specified as a string.
    ///
    /// Non-element nodes never match.
    pub fn matches_selector<'input>(
        &self,
        node_id: NodeId,
        selector: &'input str,
    ) -> Result<bool, ParseError<'input>> {
        let selector_list = self.try_parse_selector_list(selector)?;
        Ok(self.nodes[node_id].matches_selector_raw(&selector_list))
    }

    /// Find the closest matching element at or above the node identified by
    /// `node_id`.
    ///
    /// Non-element nodes never match and return `None`.
    pub fn closest<'input>(
        &self,
        node_id: NodeId,
        selector: &'input str,
    ) -> Result<Option<NodeId>, ParseError<'input>> {
        let selector_list = self.try_parse_selector_list(selector)?;
        Ok(self.nodes[node_id].closest_raw(&selector_list))
    }

    /// Parse `input` as a selector list. A selector nested deeper than
    /// [`MAX_SELECTOR_NESTING`] is a parse error: Stylo's parser and
    /// matcher recurse once per level.
    pub fn try_parse_selector_list<'input>(
        &self,
        input: &'input str,
    ) -> Result<SelectorList<SelectorImpl>, ParseError<'input>> {
        if nesting_exceeds(input, MAX_SELECTOR_NESTING) {
            return Err(ParseError {
                kind: ParseErrorKind::Custom(StyleParseErrorKind::UnspecifiedError),
                location: SourceLocation { line: 0, column: 1 },
            });
        }
        let url_extra_data = self.url.url_extra_data();
        SelectorParser::parse_author_origin_no_namespace(input, &url_extra_data)
    }
}

/// The deepest nesting of parentheses (`:is(`, `:not(`, `:has(`, …) a
/// selector may have. Stylo's selector parser and matcher recurse once per
/// level, and a few thousand levels overflow an 8 MiB stack (between
/// 2,000 and 5,000 in an optimized native build), so a page's
/// `querySelector` could abort the process. Real selectors nest a handful
/// of levels.
pub const MAX_SELECTOR_NESTING: usize = 128;

/// Whether `css` nests parenthesized blocks (functions included) more
/// than `limit` deep, not counting parentheses in strings, comments or
/// escapes. It over-counts only for unbalanced input, which the parser
/// rejects anyway.
fn nesting_exceeds(css: &str, limit: usize) -> bool {
    let mut depth = 0usize;
    let mut bytes = css.bytes();
    while let Some(byte) = bytes.next() {
        match byte {
            b'\\' => {
                bytes.next();
            }
            b'"' | b'\'' => skip_string(&mut bytes, byte),
            b'/' if bytes.clone().next() == Some(b'*') => {
                bytes.next();
                skip_comment(&mut bytes);
            }
            b'(' => {
                depth += 1;
                if depth > limit {
                    return true;
                }
            }
            b')' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    false
}

/// Past the end of a string opened by `quote`: its closing quote, or the
/// newline that ends a bad string.
fn skip_string(bytes: &mut std::str::Bytes<'_>, quote: u8) {
    while let Some(byte) = bytes.next() {
        match byte {
            b'\\' => {
                bytes.next();
            }
            b'\n' => return,
            _ if byte == quote => return,
            _ => {}
        }
    }
}

/// Past the `*/` that closes a comment.
fn skip_comment(bytes: &mut std::str::Bytes<'_>) {
    let mut star = false;
    for byte in bytes.by_ref() {
        if star && byte == b'/' {
            return;
        }
        star = byte == b'*';
    }
}

impl Node {
    /// Find the first descendant of this node that matches the selector(s)
    /// specified in `selector_list`.
    ///
    /// The scope node itself is never matched. Selector parts may match nodes
    /// outside the scope while evaluating relationships between descendants.
    ///
    /// Text and comment scope nodes return no matches.
    pub fn query_selector_raw(&self, selector_list: &SelectorList<SelectorImpl>) -> Option<NodeId> {
        let mut result = None;
        query_selector::<&Node, QueryFirst>(
            self,
            selector_list,
            &mut result,
            MayUseInvalidation::No,
        );
        result.map(|node| node.id)
    }

    /// Find all descendants of this node that match the selector(s) specified
    /// in `selector_list`, in tree order.
    ///
    /// The scope node itself is never matched. Selector parts may match nodes
    /// outside the scope while evaluating relationships between descendants.
    ///
    /// Text and comment scope nodes return no matches.
    pub fn query_selector_all_raw(
        &self,
        selector_list: &SelectorList<SelectorImpl>,
    ) -> SmallVec<[NodeId; 32]> {
        let mut results = SmallVec::new();
        query_selector::<&Node, QueryAll>(
            self,
            selector_list,
            &mut results,
            MayUseInvalidation::No,
        );
        results.iter().map(|node| node.id).collect()
    }

    /// Test whether this element matches the selector(s) specified in
    /// `selector_list`.
    ///
    /// Non-element nodes never match.
    pub fn matches_selector_raw(&self, selector_list: &SelectorList<SelectorImpl>) -> bool {
        if !self.is_element() {
            return false;
        }

        element_matches(&self, selector_list, self.owner_doc().quirks_mode())
    }

    /// Find the closest matching element at or above this element.
    ///
    /// Non-element nodes never match and return `None`.
    pub fn closest_raw(&self, selector_list: &SelectorList<SelectorImpl>) -> Option<NodeId> {
        if !self.is_element() {
            return None;
        }

        element_closest(self, selector_list, self.owner_doc().quirks_mode()).map(|node| node.id)
    }
}
