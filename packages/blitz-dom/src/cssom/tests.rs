use super::*;
use crate::shadow::ShadowRootInit;
use crate::{DocumentConfig, QualName, ns};

fn element(doc: &mut BaseDocument, parent: NodeId, name: &str) -> NodeId {
    let mut mutation = doc.mutate();
    let id = mutation.create_element(QualName::new(None, ns!(html), name.into()), vec![]);
    mutation.append_children(parent, &[id]);
    id
}
fn document() -> (BaseDocument, NodeId) {
    let mut doc = BaseDocument::new(DocumentConfig::default());
    let root = doc.root_node().id;
    let html = element(&mut doc, root, "html");
    let body = element(&mut doc, html, "body");
    (doc, body)
}
fn owner_sheet(doc: &mut BaseDocument, parent: NodeId, css: &str) -> CssomSheet {
    let owner = element(doc, parent, "style");
    doc.mutate().set_text_content(owner, css);
    doc.retain_stylesheet(owner).unwrap()
}
fn width(doc: &mut BaseDocument, node: NodeId) -> String {
    doc.resolve(0.0);
    doc.resolved_style_value(node, "width")
}

#[test]
fn retained_rule_identity_survives_shifts_deletion_and_sheet_replacement() {
    let (mut doc, body) = document();
    let sheet = owner_sheet(&mut doc, body, "a{color:red} b{color:blue}");
    let rule = doc.retain_stylesheet_rule(&sheet, &[1]).unwrap();
    doc.retained_stylesheet_insert_rule(&sheet, &[], "c{}", 0)
        .unwrap();
    assert_eq!(
        rule.identity(),
        doc.retain_stylesheet_rule(&sheet, &[2]).unwrap().identity()
    );
    doc.retained_stylesheet_delete_rule(&sheet, &[], 2).unwrap();
    assert!(!doc.retained_rule_is_attached(&rule));
    doc.retained_rule_style_set_property(&rule, "color", "green", true)
        .unwrap();
    assert_eq!(
        doc.retained_rule_style_get_property(&rule, "color"),
        Some(("green".into(), true))
    );
    doc.mutate().set_text_content(sheet.owner, "d{}");
    assert_eq!(doc.retained_rule_info(&rule).unwrap().attributes[0].1, "b");
    let mut foreign = BaseDocument::new(DocumentConfig::default());
    assert!(foreign.retained_rule_info(&rule).is_none());
    assert_eq!(
        foreign.retained_rule_style_set_css_text(&rule, "color:red"),
        Err(CssomError::NotFound)
    );
}

#[test]
fn retained_nested_rules_and_lists_survive_detachment() {
    let (mut doc, body) = document();
    let sheet = owner_sheet(&mut doc, body, "@media all {a{color:red}}");
    let group = doc.retain_stylesheet_rule(&sheet, &[0]).unwrap();
    let child = doc.retain_child_rule(&group, 0).unwrap();
    doc.retained_stylesheet_delete_rule(&sheet, &[], 0).unwrap();
    assert!(!doc.retained_rule_is_attached(&child));
    assert!(doc.retained_rule_has_parent(&child));
    doc.retained_rule_insert_rule(&group, "b{}", 0).unwrap();
    assert_eq!(doc.retained_rule_count(&group), Some(2));
    assert_eq!(
        doc.retain_child_rule(&group, 1).unwrap().identity(),
        child.identity()
    );
    doc.retained_rule_set_selector_text(&child, "div").unwrap();
    assert_eq!(
        doc.retained_rule_info(&child).unwrap().attributes[0].1,
        "div"
    );
}

#[test]
fn adopted_sheet_mutations_media_disabled_and_replacement_restyle_document() {
    let (mut doc, body) = document();
    let target = element(&mut doc, body, "div");
    doc.set_style_property(target, "width", "10px");
    let sheet = doc.create_constructed_stylesheet();
    doc.retained_stylesheet_replace(&sheet, "div{width:20px!important}")
        .unwrap();
    doc.adopt_stylesheets(doc.root_node().id, std::slice::from_ref(&sheet))
        .unwrap();
    assert_eq!(width(&mut doc, target), "20px");
    let rule = doc.retain_stylesheet_rule(&sheet, &[0]).unwrap();
    doc.retained_rule_style_set_property(&rule, "width", "30px", true)
        .unwrap();
    assert_eq!(width(&mut doc, target), "30px");
    doc.set_stylesheet_disabled(&sheet, true).unwrap();
    assert_eq!(width(&mut doc, target), "10px");
    doc.set_stylesheet_disabled(&sheet, false).unwrap();
    assert_eq!(width(&mut doc, target), "30px");
    doc.set_stylesheet_media(&sheet, "not all").unwrap();
    assert_eq!(width(&mut doc, target), "10px");
    doc.set_stylesheet_media(&sheet, "all").unwrap();
    assert_eq!(width(&mut doc, target), "30px");
    doc.retained_stylesheet_replace(&sheet, "div{width:40px!important}")
        .unwrap();
    assert!(!doc.retained_rule_is_attached(&rule));
    assert_eq!(width(&mut doc, target), "40px");
    doc.retained_rule_style_set_property(&rule, "width", "50px", true)
        .unwrap();
    assert_eq!(width(&mut doc, target), "40px");
    doc.adopt_stylesheets(doc.root_node().id, &[]).unwrap();
    assert_eq!(width(&mut doc, target), "10px");
}

#[test]
fn adopted_shadow_sheet_is_scoped_and_restyles_after_rule_edits() {
    let (mut doc, body) = document();
    let host = element(&mut doc, body, "div");
    let root = doc
        .mutate()
        .attach_shadow(
            host,
            ShadowRootInit {
                open: true,
                ..Default::default()
            },
        )
        .unwrap();
    let target = element(&mut doc, root, "span");
    let outside = element(&mut doc, body, "span");
    doc.set_style_property(target, "width", "10px");
    doc.set_style_property(outside, "width", "10px");
    let sheet = doc.create_constructed_stylesheet();
    doc.retained_stylesheet_replace(&sheet, "span{display:block;width:20px!important}")
        .unwrap();
    doc.adopt_stylesheets(root, std::slice::from_ref(&sheet))
        .unwrap();
    assert_eq!(width(&mut doc, target), "20px");
    assert_eq!(width(&mut doc, outside), "10px");
    let rule = doc.retain_stylesheet_rule(&sheet, &[0]).unwrap();
    doc.retained_rule_style_set_css_text(&rule, "display:block;width:30px!important")
        .unwrap();
    assert_eq!(width(&mut doc, target), "30px");
    doc.set_stylesheet_disabled(&sheet, true).unwrap();
    assert_eq!(width(&mut doc, target), "10px");
    doc.set_stylesheet_disabled(&sheet, false).unwrap();
    doc.set_stylesheet_media(&sheet, "not all").unwrap();
    assert_eq!(width(&mut doc, target), "10px");
    doc.set_stylesheet_media(&sheet, "all").unwrap();
    assert_eq!(width(&mut doc, target), "30px");
    doc.adopt_stylesheets(root, &[]).unwrap();
    assert_eq!(width(&mut doc, target), "10px");
}

#[test]
fn adoption_rejects_foreign_and_nonconstructed_sheets_atomically() {
    let (mut doc, body) = document();
    let root = doc.root_node().id;
    let owner = owner_sheet(&mut doc, body, "a{}");
    assert_eq!(
        doc.adopt_stylesheets(root, &[owner]),
        Err(CssomError::NotSupported)
    );
    let foreign = BaseDocument::new(DocumentConfig::default()).create_constructed_stylesheet();
    assert_eq!(
        doc.adopt_stylesheets(root, &[foreign]),
        Err(CssomError::NotSupported)
    );
    assert!(doc.adopted_stylesheets.is_empty());
    let constructed = doc.create_constructed_stylesheet();
    assert!(constructed.origin_clean());
    assert_eq!(
        doc.retained_stylesheet_insert_rule(&constructed, &[], "@import url('x.css');", 0),
        Err(CssomError::Syntax)
    );
}

#[test]
fn declaration_named_properties_include_enabled_shorthands_and_aliases() {
    let names = crate::css_style_property_names();
    assert!(names.contains(&"margin"));
    assert!(names.contains(&"color"));
    assert!(names.contains(&"-webkit-transform"));
    assert!(names.windows(2).all(|pair| pair[0] < pair[1]));
}

#[test]
fn media_items_are_grammar_parsed_and_mutate_without_duplicate_reordering() {
    let mut doc = BaseDocument::new(DocumentConfig::default());
    let sheet = doc.create_constructed_stylesheet();
    doc.set_stylesheet_media(&sheet, "screen, print").unwrap();
    doc.append_stylesheet_medium(&sheet, "screen").unwrap();
    assert_eq!(
        doc.stylesheet_media_items(&sheet).unwrap(),
        ["screen", "print"]
    );
    doc.append_stylesheet_medium(&sheet, "screen, print")
        .unwrap();
    assert_eq!(doc.stylesheet_media_items(&sheet).unwrap().len(), 2);
    assert_eq!(
        doc.delete_stylesheet_medium(&sheet, "speech"),
        Err(CssomError::NotFound)
    );
    doc.delete_stylesheet_medium(&sheet, "screen").unwrap();
    assert_eq!(doc.stylesheet_media_items(&sheet).unwrap(), ["print"]);
}

#[test]
fn retained_declaration_urls_use_stylesheet_base_after_redirect() {
    let (mut doc, body) = document();
    let target = element(&mut doc, body, "div");
    let owner = element(&mut doc, body, "style");
    let parsed = style::stylesheets::Stylesheet::from_str(
        "div{}",
        style::stylesheets::UrlExtraData::from(
            url::Url::parse("https://cdn.example/css/final.css").unwrap(),
        ),
        Origin::Author,
        ServoArc::new(doc.guard.wrap(style::media_queries::MediaList::empty())),
        doc.guard.clone(),
        None,
        None,
        QuirksMode::NoQuirks,
        AllowImportRules::No,
    );
    doc.add_stylesheet_for_node(DocumentStyleSheet(ServoArc::new(parsed)), owner);
    let sheet = doc.retain_stylesheet(owner).unwrap();
    let rule = doc.retain_stylesheet_rule(&sheet, &[0]).unwrap();
    doc.retained_rule_style_set_property(&rule, "background-image", "url(image.png)", false)
        .unwrap();
    doc.resolve(0.0);
    assert!(
        doc.resolved_style_value(target, "background-image")
            .contains("https://cdn.example/css/image.png")
    );
    doc.retained_rule_style_set_css_text(&rule, "background-image:url(other.png)")
        .unwrap();
    doc.resolve(0.0);
    assert!(
        doc.resolved_style_value(target, "background-image")
            .contains("https://cdn.example/css/other.png")
    );
}

#[test]
fn font_face_css_text_replaces_descriptors_and_preserves_rule_identity() {
    let (mut doc, body) = document();
    let sheet = owner_sheet(
        &mut doc,
        body,
        "@font-face{font-family:Old;src:local(Old);font-weight:bold}",
    );
    let rule = doc.retain_stylesheet_rule(&sheet, &[0]).unwrap();
    doc.retained_rule_style_set_css_text(&rule, "font-family:New;src:local(New)")
        .unwrap();
    assert_eq!(
        doc.retained_rule_style_get_property(&rule, "font-family")
            .unwrap()
            .0,
        "New"
    );
    assert_eq!(
        doc.retained_rule_style_get_property(&rule, "font-weight")
            .unwrap()
            .0,
        ""
    );
    assert_eq!(
        rule.identity(),
        doc.retain_stylesheet_rule(&sheet, &[0]).unwrap().identity()
    );
    doc.retained_rule_style_set_css_text(&rule, "invalid: value")
        .unwrap();
    assert_eq!(doc.retained_rule_style_css_text(&rule).unwrap(), "");
}

fn media_attribute(doc: &mut BaseDocument, owner: NodeId, value: &str) {
    doc.mutate()
        .set_attribute(owner, QualName::new(None, ns!(), "media".into()), value);
}

#[test]
fn owner_media_initial_dynamic_and_removed_attributes_apply_in_each_scope() {
    use crate::net::{ResourceHandler, StylesheetHandler};
    for shadow in [false, true] {
        for linked in [false, true] {
            let (mut doc, body) = document();
            let parent = if shadow {
                let host = element(&mut doc, body, "div");
                doc.mutate()
                    .attach_shadow(
                        host,
                        ShadowRootInit {
                            open: true,
                            ..Default::default()
                        },
                    )
                    .unwrap()
            } else {
                body
            };
            let target = element(&mut doc, parent, "div");
            doc.set_style_property(target, "width", "10px");
            let owner = element(&mut doc, parent, if linked { "link" } else { "style" });
            media_attribute(&mut doc, owner, "not all");
            if linked {
                let handler = ResourceHandler::boxed(
                    doc.tx.clone(),
                    doc.id(),
                    Some(doc.resource_pin(owner)),
                    doc.shell_provider.clone(),
                    StylesheetHandler {
                        source_url: url::Url::parse("https://example.test/style.css").unwrap(),
                        guard: doc.guard.clone(),
                        net_provider: doc.net_provider.clone(),
                        abort_signal: None,
                    },
                );
                handler.bytes(
                    "https://example.test/style.css".into(),
                    blitz_traits::net::Bytes::from_static(b"div{width:20px!important}"),
                );
                doc.handle_messages();
            } else {
                doc.mutate()
                    .set_text_content(owner, "div{width:20px!important}");
            }
            let sheet = doc.retain_stylesheet(owner).unwrap();
            assert_eq!(doc.stylesheet_media(&sheet).unwrap(), "not all");
            assert_eq!(
                width(&mut doc, target),
                "10px",
                "shadow={shadow} linked={linked}"
            );
            media_attribute(&mut doc, owner, "all");
            assert_eq!(
                doc.retain_stylesheet(owner).unwrap().identity(),
                sheet.identity()
            );
            assert_eq!(width(&mut doc, target), "20px");
            media_attribute(&mut doc, owner, "not all");
            assert_eq!(width(&mut doc, target), "10px");
            doc.mutate()
                .clear_attribute(owner, QualName::new(None, ns!(), "media".into()));
            assert_eq!(doc.stylesheet_media(&sheet).unwrap(), "");
            assert_eq!(width(&mut doc, target), "20px");
            doc.mutate().remove_node(owner);
            media_attribute(&mut doc, owner, "not all");
            assert_eq!(doc.stylesheet_media(&sheet).unwrap(), "");
        }
    }
}

#[test]
fn stylesheet_tree_order_survives_owner_slot_reuse_and_reordering() {
    for shadow in [false, true] {
        let (mut doc, body) = document();
        let parent = if shadow {
            let host = element(&mut doc, body, "div");
            doc.mutate()
                .attach_shadow(
                    host,
                    ShadowRootInit {
                        open: true,
                        ..Default::default()
                    },
                )
                .unwrap()
        } else {
            body
        };
        let spare = element(&mut doc, parent, "aside");
        let first = owner_sheet(&mut doc, parent, "p{width:20px}");
        let target = element(&mut doc, parent, "p");
        doc.mutate().remove_and_drop_node(spare);
        let second = owner_sheet(&mut doc, parent, "p{width:30px}");
        assert!(
            second.owner < first.owner,
            "fixture must reuse an earlier slot"
        );
        media_attribute(&mut doc, second.owner, "print");
        assert_eq!(width(&mut doc, target), "20px");
        media_attribute(&mut doc, second.owner, "screen");
        assert_eq!(width(&mut doc, target), "30px", "shadow={shadow}");
        if !shadow {
            assert_eq!(
                doc.stylesheet_owner_nodes().collect::<Vec<_>>(),
                [first.owner, second.owner]
            );
        }
        doc.mutate()
            .insert_nodes_before(first.owner, &[second.owner]);
        assert_eq!(width(&mut doc, target), "20px", "reordered shadow={shadow}");
    }
}

#[test]
fn moving_a_container_of_multiple_sheets_reorders_the_cascade_atomically() {
    for shadow in [false, true] {
        let (mut doc, body) = document();
        let parent = if shadow {
            let host = element(&mut doc, body, "div");
            doc.mutate()
                .attach_shadow(
                    host,
                    ShadowRootInit {
                        open: true,
                        ..Default::default()
                    },
                )
                .unwrap()
        } else {
            body
        };
        let first = owner_sheet(&mut doc, parent, "p{width:10px}");
        let container = element(&mut doc, parent, "div");
        let a = owner_sheet(&mut doc, container, "p{width:20px}");
        let b = owner_sheet(&mut doc, container, "p{width:30px}");
        let target = element(&mut doc, parent, "p");
        assert_eq!(width(&mut doc, target), "30px");
        doc.mutate().insert_nodes_before(first.owner, &[container]);
        assert_eq!(width(&mut doc, target), "10px", "shadow={shadow}");
        if !shadow {
            assert_eq!(
                doc.stylesheet_owner_nodes().collect::<Vec<_>>(),
                [a.owner, b.owner, first.owner]
            );
        }
        doc.mutate().append_children(parent, &[a.owner, b.owner]);
        assert_eq!(width(&mut doc, target), "30px");
        let adopted = doc.create_constructed_stylesheet();
        doc.retained_stylesheet_replace(&adopted, "p{width:40px}")
            .unwrap();
        let scope = if shadow { parent } else { doc.root_node().id };
        doc.adopt_stylesheets(scope, &[adopted]).unwrap();
        doc.mutate()
            .insert_nodes_before(first.owner, &[a.owner, b.owner]);
        assert_eq!(width(&mut doc, target), "40px");
        doc.adopt_stylesheets(scope, &[]).unwrap();
        assert_eq!(width(&mut doc, target), "10px");
    }
}
