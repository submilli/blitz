//! Per-build admission, shared by name references and every emitted node.
use crate::{BaseDocument, ElementData, Node};
use std::{cell::Cell, ops::Deref};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AccessibilityLimit;

/// Counts inspected nodes/attributes and an upper bound on UTF-8 bytes copied
/// or scanned. A failed operation stays failed, and no partial tree is returned.
pub struct AccessibilityBudget {
    work: Cell<usize>,
    text: Cell<usize>,
    failed: Cell<bool>,
}
impl AccessibilityBudget {
    pub fn new(work: usize, text: usize) -> Self {
        Self {
            work: Cell::new(work),
            text: Cell::new(text),
            failed: Cell::new(false),
        }
    }
    pub fn remaining(&self) -> (usize, usize) {
        (self.work.get(), self.text.get())
    }
    pub(super) fn failed(&self) -> bool {
        self.failed.get()
    }
    pub(super) fn refuse(&self) {
        self.failed.set(true);
    }
    pub(super) fn work(&self, count: usize) -> bool {
        self.charge(&self.work, count)
    }
    pub(super) fn text(&self, count: usize) -> bool {
        self.charge(&self.text, count)
    }
    fn charge(&self, remaining: &Cell<usize>, count: usize) -> bool {
        if self.failed.get() {
            return false;
        }
        match remaining.get().checked_sub(count) {
            Some(left) => {
                remaining.set(left);
                true
            }
            None => {
                self.refuse();
                false
            }
        }
    }
}

pub(super) struct Builder<'a> {
    pub document: &'a BaseDocument,
    pub budget: &'a AccessibilityBudget,
}
impl Deref for Builder<'_> {
    type Target = BaseDocument;
    fn deref(&self) -> &BaseDocument {
        self.document
    }
}
impl Builder<'_> {
    pub(super) fn admit_node(&self, node: &Node) -> bool {
        if !self.budget.work(
            1 + node
                .attrs()
                .map_or(0, |attrs| attrs.len().saturating_mul(32)),
        ) {
            return false;
        }
        if let Some(element) = node.element_data() {
            if !self.budget.text(element.name.local.len()) {
                return false;
            }
            for attr in element.attrs() {
                if !self
                    .budget
                    .text(attr.value.utf16_len_bound().saturating_mul(3))
                {
                    return false;
                }
            }
        }
        true
    }
    pub(super) fn attribute(&self, element: &ElementData, name: &str) -> Option<String> {
        for attr in element.attrs() {
            if !self.budget.work(1) {
                return None;
            }
            if attr.name.local.as_ref() == name {
                if !self
                    .budget
                    .text(attr.value.utf16_len_bound().saturating_mul(3))
                {
                    return None;
                }
                return Some(attr.value.to_string());
            }
        }
        None
    }
    pub(super) fn raw_text(&self, node: &Node) -> String {
        let mut out = String::new();
        let mut pending = vec![node.id];
        while let Some(id) = pending.pop() {
            let node = &self.nodes[id];
            if !self.admit_node(node) {
                break;
            }
            if let Some(text) = node.text_data() {
                if !self
                    .budget
                    .text(text.content.utf16_len_bound().saturating_mul(3))
                {
                    break;
                }
                out.push_str(text.content.as_str_lossy());
            } else {
                if !self.budget.work(node.children.len()) {
                    break;
                }
                pending.extend(node.children.iter().rev().copied());
            }
        }
        out
    }
    pub(super) fn form_value(&self, node: &Node) -> Option<String> {
        let element = node.element_data()?;
        // Current IDL values are separate from content attributes. Admit the
        // engine's retained value before form_value clones or normalizes it.
        if let Some(value) = &element.form_state.value {
            if !self.budget.text(value.utf16_len_bound().saturating_mul(3)) {
                return None;
            }
        } else if element.name.local.as_ref() == "textarea" {
            self.raw_text(node);
        } else {
            self.attribute(element, "value");
        }
        if let Some(name) = element.selected_file_name()
            && !self.budget.text(name.len().saturating_add(12))
        {
            return None;
        }
        if self.budget.failed() {
            return None;
        }
        self.document.form_value(node.id)
    }
}
