use super::*;
use crate::{BaseDocument, DocumentConfig};

fn document(limit: usize) -> BaseDocument {
    BaseDocument::new(DocumentConfig {
        node_limit: Some(limit),
        ..Default::default()
    })
}

#[test]
fn html_document_without_title_has_the_fixed_inert_structure_and_owners() {
    let mut doc = document(16);
    let active = doc.root_node().id;
    let before = doc.node_count();
    let created = doc.mutate().try_create_html_document(None).unwrap();
    assert_eq!(doc.node_count(), before + 6);
    assert_ne!(created, active);
    let root = doc.get_node(created).unwrap();
    assert_eq!(root.children.len(), 2);
    let doctype = root.children[0];
    let html = root.children[1];
    assert_eq!(doc.node_type(doctype), 10);
    assert_eq!(doc.node_name(doctype), "html");
    assert_eq!(doc.node_name(html), "HTML");
    let head = doc.document_head(created).unwrap();
    let body = doc.document_body(created).unwrap();
    assert_eq!(
        doc.get_node(html).unwrap().children.as_slice(),
        &[head, body]
    );
    assert!(doc.get_node(head).unwrap().children.is_empty());
    assert!(doc.get_node(body).unwrap().children.is_empty());
    for id in [created, doctype, html, head, body] {
        assert_eq!(doc.node_document(id), created);
        assert!(doc.is_connected(id));
        assert!(!doc.get_node(id).unwrap().flags.is_in_document());
    }
    assert!(doc.document_metadata(created).document_type.is_html());
    assert!(!doc.document_metadata(created).quirks);
    assert_eq!(doc.document_url(created).as_str(), "about:blank");
    assert_eq!(doc.document_base_url(created).as_str(), "about:blank");
    assert!(doc.root_node().children.is_empty());
    assert_eq!(
        doc.template_document(doc.template_document(created)),
        doc.template_document(created)
    );
}

#[test]
fn html_document_title_preserves_text_and_every_utf16_unit_without_parsing() {
    let mut doc = document(16);
    let units = vec![b'<' as u16, b'&' as u16, 0, 13, 10, 0xd800, 0xdc00, 0xdfff];
    let created = doc
        .mutate()
        .try_create_html_document(Some(DomString::from_utf16(units.clone())))
        .unwrap();
    let head = doc.document_head(created).unwrap();
    let title = doc.get_node(head).unwrap().children[0];
    let text = doc.get_node(title).unwrap().children[0];
    assert_eq!(doc.node_name(title), "TITLE");
    assert_eq!(doc.node_type(text), 3);
    assert!(
        doc.character_data_dom(text)
            .unwrap()
            .utf16_units()
            .eq(units)
    );
    assert_eq!(doc.node_document(title), created);
    assert_eq!(doc.node_document(text), created);
    assert_eq!(doc.node_count(), 10);
}

#[test]
fn a_present_empty_title_still_creates_its_text_node() {
    let mut doc = document(10);
    let created = doc
        .mutate()
        .try_create_html_document(Some(DomString::default()))
        .unwrap();
    let head = doc.document_head(created).unwrap();
    let title = doc.get_node(head).unwrap().children[0];
    assert_eq!(doc.get_node(title).unwrap().children.len(), 1);
    let text = doc.get_node(title).unwrap().children[0];
    assert!(doc.character_data_dom(text).unwrap().is_empty());
    assert_eq!(doc.node_count(), 10);
}

#[test]
fn html_document_quota_failure_precedes_all_allocation_and_mutation() {
    for (limit, title) in [(7, None), (9, Some(DomString::from("title")))] {
        let mut doc = document(limit);
        doc.set_mutation_recording(true);
        doc.take_logged_mutations();
        let before = doc.node_count();
        assert_eq!(
            doc.mutate().try_create_html_document(title),
            Err(NodeBudgetExceeded)
        );
        assert_eq!(doc.node_count(), before);
        assert!(doc.root_node().children.is_empty());
        assert!(doc.take_logged_mutations().is_empty());
    }
}

#[test]
fn html_document_admission_includes_the_private_template_document_at_exact_limit() {
    for (limit, title) in [(8, None), (10, Some(DomString::from("title")))] {
        let mut doc = document(limit);
        let created = doc.mutate().try_create_html_document(title).unwrap();
        assert_eq!(doc.node_count(), limit);
        assert_ne!(doc.template_document(created), created);
        assert!(doc.mutate().try_create_text_node("overflow").is_err());
    }
}

#[derive(Default)]
struct TitleSpy(std::sync::Mutex<Vec<String>>);
impl blitz_traits::shell::ShellProvider for TitleSpy {
    fn set_window_title(&self, title: String) {
        self.0.lock().unwrap().push(title);
    }
}
fn spied_document() -> (BaseDocument, std::sync::Arc<TitleSpy>) {
    let spy = std::sync::Arc::new(TitleSpy::default());
    let doc = BaseDocument::new(DocumentConfig {
        shell_provider: Some(spy.clone()),
        ..Default::default()
    });
    (doc, spy)
}
#[test]
fn inert_factory_titles_do_not_notify_the_active_window() {
    let (mut doc, spy) = spied_document();
    let created = doc
        .mutate()
        .try_create_html_document(Some("inert".into()))
        .unwrap();
    assert!(spy.0.lock().unwrap().is_empty());
    let title = doc
        .get_node(doc.document_head(created).unwrap())
        .unwrap()
        .children[0];
    doc.mutate()
        .try_set_text_content(title, "still inert")
        .unwrap();
    assert!(spy.0.lock().unwrap().is_empty());
    doc.mutate().activate_document_node(created);
    assert!(spy.0.lock().unwrap().is_empty());
    doc.mutate()
        .try_set_text_content(title, "embedded active")
        .unwrap();
    assert!(spy.0.lock().unwrap().is_empty());
}
#[test]
fn an_inert_title_does_not_displace_a_pending_active_title_notification() {
    let (mut doc, spy) = spied_document();
    let root = doc.root_node().id;
    let title;
    {
        let mut m = doc.mutate();
        let html = m.create_element(html_name("html"), Vec::new());
        let head = m.create_element(html_name("head"), Vec::new());
        title = m.create_element(html_name("title"), Vec::new());
        let text = m.create_text_node("initial");
        m.append_children(title, &[text]);
        m.append_children(head, &[title]);
        m.append_children(html, &[head]);
        m.append_children(root, &[html]);
    }
    assert_eq!(*spy.0.lock().unwrap(), ["initial"]);
    spy.0.lock().unwrap().clear();
    {
        let mut m = doc.mutate();
        m.try_set_text_content(title, "active").unwrap();
        m.try_create_html_document(Some("inert".into())).unwrap();
    }
    assert_eq!(*spy.0.lock().unwrap(), ["active"]);
}
#[test]
fn detached_and_non_html_titles_do_not_notify_the_window() {
    let (mut doc, spy) = spied_document();
    let root = doc.root_node().id;
    {
        let mut m = doc.mutate();
        let detached = m.create_element(html_name("title"), Vec::new());
        let text = m.create_text_node("detached");
        m.append_children(detached, &[text]);
        let svg = m.create_element(QualName::new(None, ns!(svg), "svg".into()), Vec::new());
        let title = m.create_element(QualName::new(None, ns!(svg), "title".into()), Vec::new());
        let text = m.create_text_node("SVG");
        m.append_children(title, &[text]);
        m.append_children(svg, &[title]);
        m.append_children(root, &[svg]);
    }
    assert!(spy.0.lock().unwrap().is_empty());
}
