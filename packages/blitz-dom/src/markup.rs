//! Staged fragment parsing and transactional markup replacement.
use crate::dom_api::{DomError, node_type};
use crate::{BaseDocument, DocumentMutator, NodeBudgetExceeded, NodeId, QualName, ns};

impl DocumentMutator<'_> {
    /// Replace the children of `id` with the result of parsing `html` as a
    /// fragment in its context (the `innerHTML` setter). Old children are
    /// detached, not dropped. For `<template>`, replaces the template contents.
    pub fn set_inner_html_detaching(&mut self, id: NodeId, html: &str) {
        let _ = self.try_set_inner_html_detaching(id, html);
    }

    /// Parse before replacing children. On quota failure the old subtree and
    /// its identities remain intact, and the temporary parse tree is released.
    pub fn try_set_inner_html_detaching(
        &mut self,
        id: NodeId,
        html: &str,
    ) -> Result<(), crate::NodeBudgetExceeded> {
        self.try_replace_html(id, html, false, |_, _| Ok(()))
    }

    /// HTML unsafe parsing enables parser-owned declarative roots. Admission
    /// remains transactional and old children remain usable after replacement.
    pub fn try_set_html_unsafe(
        &mut self,
        id: NodeId,
        html: &str,
    ) -> Result<(), crate::NodeBudgetExceeded> {
        self.try_replace_html(id, html, true, |_, _| Ok(()))
    }

    /// Admit the staged ordinary children and their owned subtrees before
    /// replacing anything. None denotes an empty result. The callback must only
    /// inspect native state; it must not execute script or reenter the engine.
    /// Callback rejection releases all scratch nodes and leaves old children intact.
    pub fn try_set_inner_html_admitted<T, E: From<NodeBudgetExceeded>>(
        &mut self,
        id: NodeId,
        html: &str,
        admit: impl FnOnce(&BaseDocument, Option<NodeId>) -> Result<T, E>,
    ) -> Result<T, E> {
        self.try_replace_html(id, html, false, admit)
    }

    fn try_replace_html<T, E: From<NodeBudgetExceeded>>(
        &mut self,
        id: NodeId,
        html: &str,
        declarative: bool,
        admit: impl FnOnce(&BaseDocument, Option<NodeId>) -> Result<T, E>,
    ) -> Result<T, E> {
        let context = self.doc.shadow_host_of(id).unwrap_or(id);
        let Some(element) = self.doc.nodes[context].element_data() else {
            return admit(self.doc, None);
        };
        let name = element.name.clone();
        let is_template =
            name.ns == ns!(html) && name.local == markup5ever::local_name!("template");
        if html.is_empty() {
            let admission = admit(self.doc, None)?;
            let target = if is_template {
                element.template_contents
            } else {
                Some(id)
            };
            if let Some(target) = target {
                self.replace_all(target, None);
            }
            return Ok(admission);
        }
        let scratch = self.try_create_element(name, Vec::new())?;
        self.set_node_document(scratch, self.doc.node_document(id));
        let result = self.parse_temporary_fragment_mode(scratch, html, declarative);
        if let Err(error) = result {
            self.remove_and_drop_node(scratch);
            return Err(error.into());
        }
        let admission = match admit(self.doc, Some(scratch)) {
            Ok(value) => value,
            Err(error) => {
                self.remove_and_drop_node(scratch);
                return Err(error);
            }
        };
        let target = if is_template {
            match self.try_ensure_template_contents(id) {
                Ok(contents) => contents,
                Err(error) => {
                    self.remove_and_drop_node(scratch);
                    return Err(error.into());
                }
            }
        } else {
            id
        };
        let parsed = self.doc.nodes[scratch].children.to_vec();
        self.with_one_child_list_record(target, |m| {
            m.replace_all(target, None);
            m.append_children(target, &parsed);
        });
        self.remove_and_drop_node(scratch);
        Ok(admission)
    }

    /// `Element.insertAdjacentHTML(position, html)`: parse `html` as a
    /// fragment in the right context element and insert the result.
    pub fn insert_adjacent_html(
        &mut self,
        id: NodeId,
        position: &str,
        html: &str,
    ) -> Result<(), DomError> {
        let parent = self.doc.nodes[id].parent;
        let (context, target_parent, anchor) = match position.to_ascii_lowercase().as_str() {
            "beforebegin" | "afterend" => {
                let parent = parent.ok_or(DomError::NoModificationAllowed)?;
                if self.doc.node_type(parent) == node_type::DOCUMENT {
                    return Err(DomError::NoModificationAllowed);
                }
                let anchor = if position.eq_ignore_ascii_case("beforebegin") {
                    Some(id)
                } else {
                    self.next_sibling_id(id)
                };
                (parent, parent, anchor)
            }
            "afterbegin" => (id, id, self.doc.nodes[id].children.first().copied()),
            "beforeend" => (id, id, None),
            _ => return Err(DomError::Syntax),
        };
        // Parse into a detached element named like the context element.
        // A context that is not an element, or is the `html` element, parses
        // as a `body` would (HTML § 8.5, insertAdjacentHTML step 2).
        let context_name = self.doc.nodes[context]
            .element_data()
            .map(|el| el.name.clone())
            .filter(|name| {
                !(name.ns == ns!(html) && name.local == markup5ever::local_name!("html"))
            })
            .unwrap_or_else(|| QualName::new(None, ns!(html), markup5ever::local_name!("body")));
        let scratch = self
            .try_create_element(context_name, Vec::new())
            .map_err(|_| DomError::QuotaExceeded)?;
        if self.parse_temporary_fragment(scratch, html).is_err() {
            self.remove_and_drop_node(scratch);
            return Err(DomError::QuotaExceeded);
        }
        let parsed = self.doc.nodes[scratch].children.to_vec();
        // Moving out of an unobservable parser context must not expose its
        // soon-to-be-freed ID through a removal record.
        self.detach_children(scratch);
        if !parsed.is_empty() {
            match anchor {
                Some(anchor) => self.insert_nodes_before(anchor, &parsed),
                None => self.append_children(target_parent, &parsed),
            }
        }
        self.remove_and_drop_node(scratch);
        Ok(())
    }

    /// Scratch construction is unobservable. Keep only the eventual mutation
    /// of the real target, including when fragment parsing rolls back.
    fn parse_temporary_fragment(
        &mut self,
        scratch: NodeId,
        html: &str,
    ) -> Result<(), crate::NodeBudgetExceeded> {
        self.parse_temporary_fragment_mode(scratch, html, false)
    }

    fn parse_temporary_fragment_mode(
        &mut self,
        scratch: NodeId,
        html: &str,
        declarative: bool,
    ) -> Result<(), crate::NodeBudgetExceeded> {
        let log = self.doc.mutation_log.take();
        let selection_scratch = self.selection_scratch.replace(scratch);
        let provider = self.doc.html_parser_provider.clone();
        let result = if declarative {
            provider.try_parse_html_unsafe(self, scratch, html)
        } else {
            provider.try_parse_inner_html(self, scratch, html)
        };
        self.doc.mutation_log = log;
        self.selection_scratch = selection_scratch;
        result
    }
}
