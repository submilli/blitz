//! Style MIME changes disassociate old native sheets in every document scope.
use super::*;
use crate::shadow::ShadowRootInit;
use crate::{DocumentConfig, QualName, ns};

fn element(doc: &mut BaseDocument, parent: NodeId, name: &str) -> NodeId {
    let mut mutation = doc.mutate();
    let id = mutation.create_element(QualName::new(None, ns!(html), name.into()), vec![]);
    mutation.append_children(parent, &[id]);
    id
}
fn attribute(name: &str) -> QualName {
    QualName::new(None, ns!(), name.into())
}

#[test]
fn style_mime_changes_disassociate_sheets_in_light_shadow_and_inert_trees() {
    for scope in ["light", "shadow", "inert"] {
        let mut doc = BaseDocument::new(DocumentConfig::default());
        let root = doc.root_node().id;
        let html = element(&mut doc, root, "html");
        let body = element(&mut doc, html, "body");
        let parent = match scope {
            "shadow" => {
                let host = element(&mut doc, body, "section");
                doc.mutate()
                    .attach_shadow(
                        host,
                        ShadowRootInit {
                            open: true,
                            ..Default::default()
                        },
                    )
                    .unwrap()
            }
            "inert" => {
                let other = doc.mutate().try_create_html_document(None).unwrap();
                doc.document_head(other).unwrap()
            }
            _ => body,
        };
        let target = element(&mut doc, parent, "div");
        doc.set_style_property(target, "width", "10px");
        let owner = element(&mut doc, parent, "style");
        doc.mutate()
            .set_attribute(owner, attribute("type"), "text/plain");
        doc.mutate()
            .set_text_content(owner, "div{width:20px!important}");
        assert!(
            doc.retain_stylesheet(owner).is_none(),
            "{scope}: non-CSS initial source"
        );
        doc.mutate()
            .set_attribute(owner, attribute("type"), "TEXT/CSS");
        let old = doc
            .retain_stylesheet(owner)
            .expect("CSS MIME activates a sheet");
        assert!(doc.stylesheet_is_current(&old));
        if scope != "inert" {
            doc.resolve(0.0);
            assert_eq!(doc.resolved_style_value(target, "width"), "20px");
        }
        doc.mutate()
            .set_attribute(owner, attribute("type"), "text/plain");
        assert!(doc.retain_stylesheet(owner).is_none());
        assert!(!doc.stylesheet_is_current(&old));
        if scope != "inert" {
            doc.resolve(0.0);
            assert_eq!(doc.resolved_style_value(target, "width"), "10px");
        }
        doc.mutate().clear_attribute(owner, attribute("type"));
        assert!(doc.retain_stylesheet(owner).is_some());
        assert!(!doc.stylesheet_is_current(&old));
        for invalid in [" text/css", "text/css ", "text/css;charset=utf-8"] {
            doc.mutate()
                .set_attribute(owner, attribute("type"), invalid);
            doc.mutate()
                .set_text_content(owner, "div{width:30px!important}");
            assert!(
                doc.retain_stylesheet(owner).is_none(),
                "{scope}: {invalid:?}"
            );
        }
        assert_eq!(doc.retained_stylesheet_rule_count(&old, &[]), Some(1));
    }
}
