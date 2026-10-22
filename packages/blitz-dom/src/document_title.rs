//! Document titles use ordinary-tree selection and direct child Text content.
//! https://html.spec.whatwg.org/multipage/dom.html#document.title
use crate::{BaseDocument, DocumentMutator, DomString, NodeBudgetExceeded, NodeId, QualName, ns};

/// The native mutation closure, exposed so embedders can admit retained payload.
#[derive(Clone, Debug)]
pub enum DocumentTitleEdit {
    Existing(NodeId),
    Create {
        parent: NodeId,
        name: QualName,
        before: Option<NodeId>,
    },
}

impl BaseDocument {
    /// The appropriate title element, including SVG's direct-child rule.
    pub fn document_title_element(&self, document: NodeId) -> Option<NodeId> {
        let root = self.document_title_root(document)?;
        if self.title_name_is(root, ns!(svg), "svg") {
            return self.nodes[root]
                .children
                .iter()
                .copied()
                .find(|&id| self.title_name_is(id, ns!(svg), "title"));
        }
        let mut work = self.nodes[document]
            .children
            .iter()
            .rev()
            .copied()
            .collect::<Vec<_>>();
        while let Some(id) = work.pop() {
            if self.title_name_is(id, ns!(html), "title") {
                return Some(id);
            }
            work.extend(self.nodes[id].children.iter().rev().copied());
        }
        None
    }

    /// Admit the normalized output, including a pending collapsed space, before
    /// appending any code units. Ignored descendants never allocate text output.
    pub fn document_title_with_limit(
        &self,
        document: NodeId,
        max_units: usize,
    ) -> Option<DomString> {
        let Some(title) = self.document_title_element(document) else {
            return Some(DomString::default());
        };
        let mut output = Vec::new();
        let mut pending_space = false;
        for &child in &self.nodes[title].children {
            let Some(text) = self.nodes[child].text_data() else {
                continue;
            };
            for unit in text.content.utf16_units() {
                if matches!(unit, 9 | 10 | 12 | 13 | 32) {
                    pending_space = !output.is_empty();
                    continue;
                }
                let additional = 1 + usize::from(pending_space);
                if additional > max_units.saturating_sub(output.len()) {
                    return None;
                }
                if pending_space {
                    output.push(32);
                }
                output.push(unit);
                pending_space = false;
            }
        }
        Some(DomString::from_utf16(output))
    }

    /// No mutation occurs while inspecting this plan. Callers must use it without
    /// yielding or running script before the admitted native mutation.
    pub fn document_title_edit(&self, document: NodeId) -> Option<DocumentTitleEdit> {
        let root = self.document_title_root(document)?;
        let svg = self.title_name_is(root, ns!(svg), "svg");
        if !svg && self.nodes[root].element_data()?.name.ns != ns!(html) {
            return None;
        }
        if let Some(title) = self.document_title_element(document) {
            return Some(DocumentTitleEdit::Existing(title));
        }
        // Preserve the shared head lookup, including Chrome's HTML div-root edge.
        let parent = if svg {
            root
        } else {
            self.document_head(document)?
        };
        Some(DocumentTitleEdit::Create {
            parent,
            name: QualName::new(None, if svg { ns!(svg) } else { ns!(html) }, "title".into()),
            before: svg
                .then(|| self.nodes[parent].children.first().copied())
                .flatten(),
        })
    }

    fn document_title_root(&self, document: NodeId) -> Option<NodeId> {
        let node = self.get_node(document)?;
        if !matches!(node.data, crate::NodeData::Document(_)) {
            return None;
        }
        node.children
            .iter()
            .copied()
            .find(|&id| self.nodes[id].element_data().is_some())
    }

    fn title_name_is(&self, id: NodeId, namespace: crate::Namespace, local: &str) -> bool {
        self.nodes[id].element_data().is_some_and(|element| {
            element.name.ns == namespace && element.name.local.as_ref() == local
        })
    }
}

impl DocumentMutator<'_> {
    /// Admit the entire title/text mutation before allocating or detaching nodes.
    /// The embedder separately admits retained payload. No parser or JS runs.
    pub fn try_set_document_title(
        &mut self,
        document: NodeId,
        value: DomString,
    ) -> Result<(), NodeBudgetExceeded> {
        let Some(edit) = self.doc.document_title_edit(document) else {
            return Ok(());
        };
        self.doc.check_node_allocation(
            usize::from(matches!(edit, DocumentTitleEdit::Create { .. }))
                + usize::from(!value.is_empty()),
        )?;
        let title = match edit {
            DocumentTitleEdit::Existing(id) => id,
            DocumentTitleEdit::Create {
                parent,
                name,
                before,
            } => {
                let id = self.create_element(name, Vec::new());
                // A newly created title can always be inserted under the live
                // selected element; the native plan contains no author input.
                if let Some(before) = before {
                    self.insert_nodes_before(before, &[id]);
                } else {
                    self.append_children(parent, &[id]);
                }
                id
            }
        };
        self.set_text_content(title, value);
        Ok(())
    }
}

#[cfg(test)]
mod tests;
