//! Lossless descendant filters shared by live script collections.
//! https://dom.spec.whatwg.org/#concept-getelementsbytagname
//! https://dom.spec.whatwg.org/#concept-getelementsbyclassname
mod classes;

use crate::{BaseDocument, DomString, NodeBudgetExceeded, NodeId, ns};
use classes::Classes;

/// A compiled query; it contains owned strings and no document or script edges.
pub struct ElementFilter(Kind);

enum Kind {
    QualifiedName(DomString),
    Classes(Classes),
    Name(DomString),
    DocumentCollection(DocumentCollection),
}

/// The six live HTML element collections exposed by a Document.
#[derive(Clone, Copy)]
pub enum DocumentCollection {
    Scripts,
    Images,
    Forms,
    Embeds,
    Links,
    Anchors,
}

impl ElementFilter {
    pub fn qualified_name(name: DomString) -> Self {
        Self(Kind::QualifiedName(name))
    }

    pub fn class_names(value: &DomString) -> Self {
        Self(Kind::Classes(Classes::new(value)))
    }

    /// HTML elements with this exact null-namespace name attribute.
    pub fn name(value: DomString) -> Self {
        Self(Kind::Name(value))
    }

    pub fn document_collection(collection: DocumentCollection) -> Self {
        Self(Kind::DocumentCollection(collection))
    }

    fn accepts(&self, document: &BaseDocument, id: NodeId, html: bool, quirks: bool) -> bool {
        let Some(element) = document.get_node(id).and_then(|node| node.element_data()) else {
            return false;
        };
        match &self.0 {
            Kind::QualifiedName(name) => {
                if name == "*" {
                    return true;
                }
                let lowercase = html && element.name.ns == ns!(html);
                let expected = name.utf16_units().map(|unit| {
                    if lowercase {
                        classes::lowercase(unit)
                    } else {
                        unit
                    }
                });
                let local = element.name.local.as_ref().encode_utf16();
                if let Some(prefix) = &element.name.prefix {
                    prefix
                        .as_ref()
                        .encode_utf16()
                        .chain([u16::from(b':')])
                        .chain(local)
                        .eq(expected)
                } else {
                    local.eq(expected)
                }
            }
            Kind::Classes(classes) => element
                .attr_dom(crate::local_name!("class"))
                .is_some_and(|value| classes.matches(value, quirks)),
            Kind::DocumentCollection(collection) => {
                if element.name.ns != ns!(html) {
                    return false;
                }
                match collection {
                    DocumentCollection::Scripts => {
                        element.name.local == crate::local_name!("script")
                    }
                    DocumentCollection::Images => element.name.local == crate::local_name!("img"),
                    DocumentCollection::Forms => element.name.local == crate::local_name!("form"),
                    DocumentCollection::Embeds => element.name.local == crate::local_name!("embed"),
                    DocumentCollection::Links => {
                        matches!(element.name.local.as_ref(), "a" | "area")
                            && element.has_attr(crate::local_name!("href"))
                    }
                    DocumentCollection::Anchors => {
                        element.name.local == crate::local_name!("a")
                            && element.has_attr(crate::local_name!("name"))
                    }
                }
            }
            Kind::Name(name) => {
                element.name.ns == ns!(html)
                    && element
                        .attr_dom(crate::local_name!("name"))
                        .is_some_and(|value| value.utf16_units().eq(name.utf16_units()))
            }
        }
    }
}

impl BaseDocument {
    /// Ordinary descendants in tree order, excluding the scope itself, shadow
    /// trees and template contents. Admit each match before growing the output.
    pub fn filtered_descendant_elements(
        &self,
        scope: NodeId,
        filter: &ElementFilter,
        limit: usize,
    ) -> Result<Vec<NodeId>, NodeBudgetExceeded> {
        let Some(root) = self.get_node(scope) else {
            return Ok(Vec::new());
        };
        let metadata = self.document_metadata(scope);
        let html = metadata.document_type.is_html();
        let quirks = metadata.quirks;
        let mut work = root.children.iter().rev().copied().collect::<Vec<_>>();
        let mut result = Vec::new();
        while let Some(id) = work.pop() {
            let Some(node) = self.get_node(id) else {
                continue;
            };
            if filter.accepts(self, id, html, quirks) {
                if result.len() >= limit {
                    return Err(NodeBudgetExceeded);
                }
                result.push(id);
            }
            work.extend(node.children.iter().rev().copied());
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests;
