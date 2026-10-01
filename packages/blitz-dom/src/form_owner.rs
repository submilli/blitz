//! Script-free HTML form association shared by native input and DOM bindings.
use crate::{
    BaseDocument, NodeId, ns,
    traversal::{AncestorTraverser, TreeTraverser},
};

impl BaseDocument {
    /// Resolve a form-associated element's owner from its current tree.
    /// Explicit IDs participate only while connected and within the same tree.
    /// <https://html.spec.whatwg.org/multipage/form-control-infrastructure.html#reset-the-form-owner>
    pub fn form_owner(&self, id: NodeId) -> Option<NodeId> {
        let element = self.get_node(id)?.element_data()?;
        if element.name.ns != ns!(html)
            || !matches!(
                &*element.name.local,
                "button"
                    | "fieldset"
                    | "input"
                    | "object"
                    | "output"
                    | "select"
                    | "textarea"
                    | "img"
            )
        {
            return None;
        }
        if self.is_connected(id)
            && &*element.name.local != "img"
            && let Some(attribute) = self.attribute_by_name(id, "form")
        {
            let first = TreeTraverser::new_with_root(self, self.tree_root(id)).find(|&node| {
                self.attribute_by_name(node, "id")
                    .is_some_and(|a| a.value == attribute.value)
            });
            return first.filter(|&node| self.is_html_form(node));
        }
        AncestorTraverser::new(self, id).find(|&node| self.is_html_form(node))
    }

    fn is_html_form(&self, id: NodeId) -> bool {
        self.get_node(id)
            .and_then(|n| n.element_data())
            .is_some_and(|e| e.name.ns == ns!(html) && &*e.name.local == "form")
    }
}

#[cfg(test)]
mod tests {
    use crate::{BaseDocument, DocumentConfig, QualName, ns};

    #[test]
    fn form_ownership_tracks_attachment_explicit_ids_and_invalid_references() {
        let mut doc = BaseDocument::new(DocumentConfig::default());
        let root = doc.root_node().id;
        let mut m = doc.mutate();
        let container = m
            .try_create_element(QualName::new(None, ns!(html), "div".into()), vec![])
            .unwrap();
        m.pre_insert(container, root, None).unwrap();
        let form = m
            .try_create_element(QualName::new(None, ns!(html), "form".into()), vec![])
            .unwrap();
        let button = m
            .try_create_element(QualName::new(None, ns!(html), "button".into()), vec![])
            .unwrap();
        m.set_attribute(form, QualName::new(None, ns!(), "id".into()), "owner");
        m.set_attribute(button, QualName::new(None, ns!(), "form".into()), "owner");
        m.pre_insert(form, container, None).unwrap();
        assert_eq!(m.doc.form_owner(button), None);
        m.pre_insert(button, container, None).unwrap();
        assert_eq!(m.doc.form_owner(button), Some(form));
        m.pre_insert(button, form, None).unwrap();
        m.set_attribute(button, QualName::new(None, ns!(), "form".into()), "missing");
        assert_eq!(m.doc.form_owner(button), None);
        m.remove_node(form);
        assert_eq!(m.doc.form_owner(button), Some(form));
        m.remove_node(button);
        assert_eq!(m.doc.form_owner(button), None);
    }
}
