//! Scoped DOMString ID lookup, sharing the connected-document index when exact.
use crate::{BaseDocument, DomString, NodeId};

impl BaseDocument {
    /// First descendant with this exact nonempty ID, in ordinary tree order.
    /// Template contents and shadow trees are separate scopes. UTF-16 IDs never
    /// become CSS selectors, and scalar index collisions fall back to exact reads.
    pub fn get_element_by_id_in(&self, scope: NodeId, value: &DomString) -> Option<NodeId> {
        if value.is_empty() {
            return None;
        }
        if scope == self.root_node().id {
            let candidate = self.get_element_by_id(value.as_str_lossy())?;
            if self.has_dom_id(candidate, value) {
                return Some(candidate);
            }
        }
        let mut work = self
            .get_node(scope)?
            .children
            .iter()
            .rev()
            .copied()
            .collect::<Vec<_>>();
        while let Some(id) = work.pop() {
            if self.has_dom_id(id, value) {
                return Some(id);
            }
            work.extend(self.get_node(id)?.children.iter().rev().copied());
        }
        None
    }

    /// First HTML head child of the document element. Preserve the existing
    /// binding/Chrome behavior even for a root other than HTML html.
    pub fn document_head(&self, document: NodeId) -> Option<NodeId> {
        self.html_child_named(self.document_element_node(document)?, &["head"])
    }

    /// First HTML body or frameset child of an HTML html document element.
    pub fn document_body(&self, document: NodeId) -> Option<NodeId> {
        let root = self.document_element_node(document)?;
        let element = root.element_data()?;
        if element.name.ns != crate::ns!(html) || element.name.local != crate::local_name!("html") {
            return None;
        }
        self.html_child_named(root, &["body", "frameset"])
    }

    fn document_element_node(&self, document: NodeId) -> Option<&crate::Node> {
        let document = self.get_node(document)?;
        if !matches!(document.data, crate::NodeData::Document(_)) {
            return None;
        }
        document
            .children
            .iter()
            .filter_map(|&id| self.get_node(id))
            .find(|node| node.element_data().is_some())
    }

    fn html_child_named(&self, parent: &crate::Node, names: &[&str]) -> Option<NodeId> {
        parent.children.iter().copied().find(|&id| {
            self.get_node(id)
                .and_then(|node| node.element_data())
                .is_some_and(|element| {
                    element.name.ns == crate::ns!(html)
                        && names.contains(&element.name.local.as_ref())
                })
        })
    }

    fn has_dom_id(&self, id: NodeId, value: &DomString) -> bool {
        self.get_node(id)
            .and_then(|node| node.element_data())
            .and_then(|element| element.attr_dom(crate::local_name!("id")))
            == Some(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DocumentConfig, QualName, ns};

    fn element(doc: &mut BaseDocument, id: DomString) -> NodeId {
        let node = doc
            .mutate()
            .try_create_element(QualName::new(None, ns!(html), "div".into()), Vec::new())
            .unwrap();
        doc.mutate()
            .set_attribute(node, QualName::new(None, ns!(), "id".into()), id);
        node
    }

    #[test]
    fn id_lookup_uses_exact_code_units_despite_scalar_index_collisions() {
        let mut doc = BaseDocument::new(DocumentConfig::default());
        let root = doc.root_node().id;
        let parent = element(&mut doc, "parent".into());
        doc.mutate().append_children(root, &[parent]);
        let surrogate = DomString::from_utf16(vec![0xd800]);
        let lone = element(&mut doc, surrogate.clone());
        let replacement = element(&mut doc, "\u{fffd}".into());
        doc.mutate().append_children(parent, &[lone, replacement]);
        assert_eq!(doc.get_element_by_id_in(root, &surrogate), Some(lone));
        assert_eq!(
            doc.get_element_by_id_in(root, &"\u{fffd}".into()),
            Some(replacement)
        );
        doc.mutate()
            .pre_insert(replacement, parent, Some(lone))
            .unwrap();
        assert_eq!(doc.get_element_by_id_in(root, &surrogate), Some(lone));
        assert_eq!(
            doc.get_element_by_id_in(root, &"\u{fffd}".into()),
            Some(replacement)
        );
    }

    #[test]
    fn scoped_lookup_excludes_scope_and_unrelated_or_inert_trees() {
        let mut doc = BaseDocument::new(DocumentConfig::default());
        let root = doc.root_node().id;
        let a = element(&mut doc, "same".into());
        let b = element(&mut doc, "same".into());
        doc.mutate().append_children(root, &[a]);
        let fragment = doc.mutate().try_create_document_fragment().unwrap();
        doc.mutate().append_children(fragment, &[b]);
        assert_eq!(doc.get_element_by_id_in(root, &"same".into()), Some(a));
        assert_eq!(doc.get_element_by_id_in(fragment, &"same".into()), Some(b));
        assert_eq!(doc.get_element_by_id_in(a, &"same".into()), None);
        let template = doc
            .mutate()
            .try_create_element(
                QualName::new(None, ns!(html), "template".into()),
                Vec::new(),
            )
            .unwrap();
        let contents = doc.mutate().try_ensure_template_contents(template).unwrap();
        let inert = element(&mut doc, "inert".into());
        doc.mutate().append_children(contents, &[inert]);
        doc.mutate().append_children(a, &[template]);
        assert_eq!(doc.get_element_by_id_in(root, &"inert".into()), None);
        assert_eq!(
            doc.get_element_by_id_in(contents, &"inert".into()),
            Some(inert)
        );
    }

    #[test]
    fn duplicate_order_and_empty_or_selector_like_ids_follow_dom_rules() {
        let mut doc = BaseDocument::new(DocumentConfig::default());
        let root = doc.root_node().id;
        let parent = element(&mut doc, "parent".into());
        doc.mutate().append_children(root, &[parent]);
        let a = element(&mut doc, "[a].# b".into());
        let b = element(&mut doc, "[a].# b".into());
        let empty = element(&mut doc, "".into());
        doc.mutate().append_children(parent, &[a, b, empty]);
        assert_eq!(doc.get_element_by_id_in(root, &"[a].# b".into()), Some(a));
        doc.mutate().pre_insert(b, parent, Some(a)).unwrap();
        assert_eq!(doc.get_element_by_id_in(root, &"[a].# b".into()), Some(b));
        doc.mutate().remove_child(parent, b).unwrap();
        assert_eq!(doc.get_element_by_id_in(root, &"[a].# b".into()), Some(a));
        assert_eq!(doc.get_element_by_id_in(root, &"".into()), None);
        doc.mutate()
            .set_attribute(a, QualName::new(None, ns!(), "id".into()), "changed");
        assert_eq!(doc.get_element_by_id_in(root, &"[a].# b".into()), None);
        assert_eq!(doc.get_element_by_id_in(root, &"changed".into()), Some(a));
    }
    #[test]
    fn shadow_attribute_changes_never_enter_the_document_id_index() {
        let mut doc = BaseDocument::new(DocumentConfig::default());
        let document = doc.root_node().id;
        let host = element(&mut doc, "host".into());
        doc.mutate().append_children(document, &[host]);
        let shadow = doc
            .mutate()
            .attach_shadow(host, crate::shadow::ShadowRootInit::default())
            .unwrap();
        let child = element(&mut doc, "before".into());
        doc.mutate().append_children(shadow, &[child]);
        doc.mutate()
            .set_attribute(child, QualName::new(None, ns!(), "id".into()), "after");
        assert_eq!(doc.get_element_by_id("after"), None);
        assert_eq!(doc.get_element_by_id_in(document, &"after".into()), None);
        assert_eq!(
            doc.get_element_by_id_in(shadow, &"after".into()),
            Some(child)
        );
    }
    fn tag(doc: &mut BaseDocument, name: &str) -> NodeId {
        doc.mutate()
            .try_create_element(QualName::new(None, ns!(html), name.into()), Vec::new())
            .unwrap()
    }

    #[test]
    fn body_requires_html_root_while_head_preserves_direct_child_projection() {
        let mut doc = BaseDocument::new(DocumentConfig::default());
        let document = doc.root_node().id;
        let div = tag(&mut doc, "div");
        let head = tag(&mut doc, "head");
        let body = tag(&mut doc, "body");
        doc.mutate().append_children(document, &[div]);
        doc.mutate().append_children(div, &[head, body]);
        assert_eq!(doc.document_head(document), Some(head));
        assert_eq!(doc.document_body(document), None);
        let svg = doc
            .mutate()
            .try_create_element(QualName::new(None, ns!(svg), "html".into()), Vec::new())
            .unwrap();
        doc.mutate().append_children(svg, &[head, body]);
        doc.mutate().replace_child(document, svg, div).unwrap();
        assert_eq!(doc.document_head(document), Some(head));
        assert_eq!(doc.document_body(document), None);
    }

    #[test]
    fn head_and_body_read_direct_children_and_framesets_in_tree_order() {
        let mut doc = BaseDocument::new(DocumentConfig::default());
        let document = doc.root_node().id;
        let html = tag(&mut doc, "html");
        let head = tag(&mut doc, "head");
        let frameset = tag(&mut doc, "frameset");
        let body = tag(&mut doc, "body");
        doc.mutate().append_children(document, &[html]);
        doc.mutate().append_children(html, &[head, frameset, body]);
        assert_eq!(doc.document_head(document), Some(head));
        assert_eq!(doc.document_body(document), Some(frameset));
        doc.mutate().remove_child(html, frameset).unwrap();
        assert_eq!(doc.document_body(document), Some(body));
        doc.mutate().append_children(body, &[head]);
        assert_eq!(doc.document_head(document), None);
        assert_eq!(doc.document_body(body), None);
    }
}
