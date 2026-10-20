//! Observation-only redaction, independent of script-visible DOM getters.
use crate::{BaseDocument, DomString, Node, NodeData, NodeId};
mod bounded;
pub use bounded::{ObservationBudget, ObservationLimit, ObservationQueryError};

impl Node {
    /// Password controls and host-marked credential controls are opaque to observations.
    pub fn masks_observation(&self) -> bool {
        self.element_data().is_some_and(|el| {
            el.credential_field
                || (el.name.local.as_ref() == "input"
                    && el
                        .attr_dom(markup5ever::local_name!("type"))
                        .is_some_and(password_type))
        })
    }
    /// Text extraction for automation. DOM textContent keeps its normal semantics.
    pub fn observation_text(&self) -> String {
        let mut out = String::new();
        let mut pending = vec![self];
        while let Some(node) = pending.pop() {
            if node.masks_observation() {
                out.push_str("[REDACTED]");
            } else if let NodeData::Text(text) = &node.data {
                out.push_str(text.content.as_str_lossy());
            } else {
                pending.extend(node.children.iter().rev().map(|id| &node.tree()[*id]));
            }
        }
        out
    }
}
// Inspect at most nine original code units. Redaction must not flatten a
// potentially large UTF-16 attribute just to reject its type as non-password.
fn password_type(value: &DomString) -> bool {
    let mut units = value.utf16_units();
    b"password".iter().all(|expected| {
        units.next().is_some_and(|unit| {
            u8::try_from(unit).is_ok_and(|unit| unit.eq_ignore_ascii_case(expected))
        })
    }) && units.next().is_none()
}

impl BaseDocument {
    /// Mark before injection. The mark follows adoption and native cloning.
    pub fn mark_credential_field(&mut self, id: NodeId) -> bool {
        let Some(el) = self.get_node_mut(id).and_then(Node::element_data_mut) else {
            return false;
        };
        el.credential_field = true;
        true
    }
    /// HTML for automation only; protected controls are replaced with a fixed marker.
    pub fn observation_html(&self) -> String {
        crate::serialize::observation_html(self.root_node())
            .as_str_lossy()
            .to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DocumentConfig, QualName, ns};
    fn control(doc: &mut BaseDocument, tag: &str) -> NodeId {
        let root = doc.root_node().id;
        let mut mutation = doc.mutate();
        let id = mutation.create_element(QualName::new(None, ns!(html), tag.into()), vec![]);
        mutation.append_children(root, &[id]);
        id
    }
    #[test]
    fn masking_is_observation_only_and_survives_types_and_clones() {
        let mut doc = BaseDocument::new(DocumentConfig::default());
        let id = control(&mut doc, "input");
        doc.mutate().set_attribute(
            id,
            QualName::new(None, ns!(), "value".into()),
            "private-value",
        );
        assert!(doc.mark_credential_field(id));
        for kind in ["text", "button", "submit", "reset", "password"] {
            doc.mutate()
                .set_attribute(id, QualName::new(None, ns!(), "type".into()), kind);
            assert!(doc.get_node(id).unwrap().masks_observation());
            assert!(!doc.observation_html().contains("private-value"));
            assert!(doc.root_node().inner_html().contains("private-value"));
            let tree = doc.build_accessibility_tree();
            for (_, node) in tree.nodes {
                assert_ne!(node.value(), Some("private-value"));
                assert_ne!(node.label(), Some("private-value"));
            }
        }
        let copy = doc.mutate().clone_node(id, true);
        assert!(doc.get_node(copy).unwrap().masks_observation());
    }
    #[test]
    fn textarea_descendants_and_password_defaults_are_opaque() {
        let mut doc = BaseDocument::new(DocumentConfig::default());
        let id = control(&mut doc, "textarea");
        let text = doc.mutate().create_text_node("private-value");
        doc.mutate().append_children(id, &[text]);
        assert!(doc.mark_credential_field(id));
        assert_eq!(doc.get_node(id).unwrap().observation_text(), "[REDACTED]");
        assert!(!doc.observation_html().contains("private-value"));
        assert_eq!(doc.get_node(id).unwrap().text_content(), "private-value");
        let password = control(&mut doc, "input");
        doc.mutate().set_attribute(
            password,
            QualName::new(None, ns!(), "type".into()),
            "PASSWORD",
        );
        assert!(doc.get_node(password).unwrap().masks_observation());
    }
}
