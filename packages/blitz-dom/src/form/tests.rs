//! Submitter overrides remain current when form and button occupy different arenas.
use crate::{BaseDocument, DocumentConfig, NodeId, QualName, ns};
use blitz_traits::navigation::{NavigationOptions, NavigationProvider};
use std::sync::{Arc, Mutex};
#[derive(Default)]
struct Navigation(Mutex<Vec<NavigationOptions>>);
impl NavigationProvider for Navigation {
    fn navigate_to(&self, options: NavigationOptions) {
        self.0.lock().unwrap().push(options);
    }
}
fn element(
    doc: &mut BaseDocument,
    parent: NodeId,
    name: &str,
    attributes: &[(&str, &str)],
) -> NodeId {
    let mut m = doc.mutate();
    let node = m.create_element(QualName::new(None, ns!(html), name.into()), vec![]);
    for (name, value) in attributes {
        m.set_attribute(node, QualName::new(None, ns!(), (*name).into()), *value);
    }
    m.pre_insert(node, parent, None).unwrap();
    node
}
#[test]
fn a_foreign_submitter_controls_action_method_and_encoding_but_not_entry_membership() {
    let navigation = Arc::new(Navigation::default());
    let mut form_doc = BaseDocument::new(DocumentConfig {
        base_url: Some("https://form.test/page".into()),
        navigation_provider: Some(navigation.clone()),
        ..Default::default()
    });
    let root = form_doc.root_node().id;
    let form = element(&mut form_doc, root, "form", &[("action", "/own")]);
    element(
        &mut form_doc,
        form,
        "input",
        &[("name", "field"), ("value", "ok")],
    );
    let mut button_doc = BaseDocument::new(DocumentConfig {
        base_url: Some("https://button.test/page".into()),
        ..Default::default()
    });
    let root = button_doc.root_node().id;
    let button = element(
        &mut button_doc,
        root,
        "button",
        &[
            ("name", "button"),
            ("value", "yes"),
            ("formaction", "/current"),
            ("formmethod", "post"),
            ("formenctype", "text/plain"),
        ],
    );
    let overrides = button_doc.submitter_overrides(button);
    form_doc.submit_form_with_overrides(form, None, &overrides);
    let seen = navigation.0.lock().unwrap();
    let request = &seen[0];
    assert_eq!(request.url.as_str(), "https://button.test/current");
    assert_eq!(request.method.as_str(), "POST");
    assert_eq!(request.content_type.as_deref(), Some("text/plain"));
    let blitz_traits::net::Body::Form(entries) = &request.document_resource else {
        panic!("POST carries form entries")
    };
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name.as_str(), "field");
    assert_eq!(entries[0].value.as_ref(), "ok");
}
#[test]
fn empty_submitter_action_uses_its_document_url_instead_of_the_base() {
    let navigation = Arc::new(Navigation::default());
    let mut doc = BaseDocument::new(DocumentConfig {
        base_url: Some("https://page.test/original".into()),
        navigation_provider: Some(navigation.clone()),
        ..Default::default()
    });
    let root = doc.root_node().id;
    let html = element(&mut doc, root, "html", &[]);
    let head = element(&mut doc, html, "head", &[]);
    element(
        &mut doc,
        head,
        "base",
        &[("href", "https://assets.test/base/")],
    );
    let form = element(&mut doc, html, "form", &[("action", "wrong")]);
    let button = element(&mut doc, form, "button", &[("formaction", "")]);
    doc.submit_form(form, button);
    assert_eq!(
        navigation.0.lock().unwrap()[0].url.as_str(),
        "https://page.test/original?"
    );
}
