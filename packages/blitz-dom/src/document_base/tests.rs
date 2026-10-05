use crate::{BaseDocument, DocumentConfig, DocumentMutator, NodeId, QualName, ns};
use url::Url;

fn document() -> (BaseDocument, NodeId) {
    let mut doc = BaseDocument::new(DocumentConfig {
        base_url: Some("https://owner.test/old/page".into()),
        ..Default::default()
    });
    let root = doc.root_node().id;
    let body = {
        let mut m = doc.mutate();
        let body = m.create_element(QualName::new(None, ns!(html), "html".into()), vec![]);
        m.pre_insert(body, root, None).unwrap();
        body
    };
    (doc, body)
}

fn base(m: &mut DocumentMutator<'_>, parent: NodeId, href: &str) -> NodeId {
    let base = m.create_element(QualName::new(None, ns!(html), "base".into()), vec![]);
    m.set_attribute(base, QualName::new(None, ns!(), "href".into()), href);
    m.pre_insert(base, parent, None).unwrap();
    base
}

#[test]
fn first_base_freezes_until_href_or_first_element_changes() {
    let (mut doc, body) = document();
    let (first, second) = {
        let mut m = doc.mutate();
        (base(&mut m, body, "assets/"), base(&mut m, body, "second/"))
    };
    assert_eq!(
        doc.document_base_url(body).as_str(),
        "https://owner.test/old/assets/"
    );
    doc.set_base_url("https://owner.test/new/page");
    assert_eq!(
        doc.document_base_url(body).as_str(),
        "https://owner.test/old/assets/"
    );
    doc.mutate().set_attribute(
        second,
        QualName::new(None, ns!(), "href".into()),
        "changed/",
    );
    assert_eq!(
        doc.document_base_url(body).as_str(),
        "https://owner.test/old/assets/"
    );
    doc.mutate()
        .set_attribute(first, QualName::new(None, ns!(), "href".into()), "assets/");
    assert_eq!(
        doc.document_base_url(body).as_str(),
        "https://owner.test/new/assets/"
    );
    doc.mutate().remove_node(first);
    assert_eq!(
        doc.document_base_url(body).as_str(),
        "https://owner.test/new/changed/"
    );
    doc.mutate().pre_insert(first, body, Some(second)).unwrap();
    assert_eq!(
        doc.document_base_url(body).as_str(),
        "https://owner.test/new/assets/"
    );
    assert_eq!(doc.url().as_str(), "https://owner.test/new/page");
}

#[test]
fn invalid_first_base_blocks_later_base_and_freezes_fallback() {
    for href in ["http://[", "data:text/plain,hello", "javascript:void(0)"] {
        let (mut doc, body) = document();
        let first = {
            let mut m = doc.mutate();
            let first = base(&mut m, body, href);
            base(&mut m, body, "https://other.test/assets/");
            first
        };
        doc.set_base_url("https://owner.test/new/page");
        assert_eq!(
            doc.document_base_url(body).as_str(),
            "https://owner.test/old/page"
        );
        doc.mutate()
            .clear_attribute(first, QualName::new(None, ns!(), "href".into()));
        assert_eq!(
            doc.document_base_url(body).as_str(),
            "https://other.test/assets/"
        );
        assert_eq!(
            doc.url().origin().ascii_serialization(),
            "https://owner.test"
        );
    }
}

#[test]
fn inherited_fallback_is_a_snapshot_with_independent_document_url() {
    let (mut doc, body) = document();
    {
        let mut m = doc.mutate();
        base(&mut m, body, "/parent-assets/");
    }
    let captured = doc.document_base_url(body);
    let child = {
        let mut m = doc.mutate();
        let child = m.create_document_node();
        let metadata = m.document_metadata_mut(child);
        metadata.url = Some(Url::parse("about:blank").unwrap());
        metadata.inherited_base_url = Some(captured);
        child
    };
    doc.set_base_url("https://owner.test/moved/page");
    assert_eq!(
        doc.document_base_url(child).as_str(),
        "https://owner.test/parent-assets/"
    );
    let child_base = {
        let mut m = doc.mutate();
        base(&mut m, child, "nested/")
    };
    assert_eq!(
        doc.document_base_url(child_base).as_str(),
        "https://owner.test/parent-assets/nested/"
    );
    assert_eq!(
        doc.document_metadata(child).url.as_ref().unwrap().as_str(),
        "about:blank"
    );
}

#[test]
fn removing_multiple_bases_promotes_only_after_the_batch() {
    let (mut doc, body) = document();
    {
        let mut m = doc.mutate();
        base(&mut m, body, "one/");
        base(&mut m, body, "two/");
    }
    doc.set_base_url("https://owner.test/current/page");
    doc.mutate().set_text_content(body, "");
    assert_eq!(
        doc.document_base_url(body).as_str(),
        "https://owner.test/current/page"
    );
}

#[test]
fn frame_fallbacks_capture_insertion_order_and_share_unchanged_base() {
    let (mut doc, body) = document();
    let first = {
        let mut m = doc.mutate();
        let frame = m.create_element(QualName::new(None, ns!(html), "iframe".into()), vec![]);
        m.pre_insert(frame, body, None).unwrap();
        base(&mut m, body, "https://assets.test/one/");
        frame
    };
    let (second, third) = {
        let mut m = doc.mutate();
        let second = m.create_element(QualName::new(None, ns!(html), "iframe".into()), vec![]);
        let third = m.create_element(QualName::new(None, ns!(html), "iframe".into()), vec![]);
        m.pre_insert(second, body, None).unwrap();
        m.pre_insert(third, body, None).unwrap();
        (second, third)
    };
    assert_eq!(
        doc.initial_frame_base_url(first).unwrap().as_str(),
        "https://owner.test/old/page"
    );
    let second_base = doc.initial_frame_base_url(second).unwrap();
    assert_eq!(second_base.as_str(), "https://assets.test/one/");
    assert!(std::rc::Rc::ptr_eq(
        &second_base,
        &doc.initial_frame_base_url(third).unwrap()
    ));
    doc.mutate().remove_node(first);
    assert!(doc.initial_frame_base_url(first).is_none());
    doc.mutate().pre_insert(first, body, None).unwrap();
    assert!(std::rc::Rc::ptr_eq(
        &second_base,
        &doc.initial_frame_base_url(first).unwrap()
    ));
}

#[test]
fn resources_and_forms_use_effective_base_without_changing_authority() {
    use blitz_traits::navigation::{NavigationOptions, NavigationProvider};
    use blitz_traits::net::{NetHandler, NetProvider, Request};
    use std::sync::{Arc, Mutex};
    #[derive(Default)]
    struct Requests(Mutex<Vec<String>>);
    impl NetProvider for Requests {
        fn fetch(&self, _: usize, request: Request, _: Box<dyn NetHandler>) {
            self.0.lock().unwrap().push(request.url.to_string());
        }
    }
    impl NavigationProvider for Requests {
        fn navigate_to(&self, options: NavigationOptions) {
            self.0.lock().unwrap().push(options.url.to_string());
        }
    }
    let requests = Arc::new(Requests::default());
    let mut doc = BaseDocument::new(DocumentConfig {
        base_url: Some("https://owner.test/old/page".into()),
        net_provider: Some(requests.clone()),
        navigation_provider: Some(requests.clone()),
        ..Default::default()
    });
    let root = doc.root_node().id;
    let form = {
        let mut m = doc.mutate();
        let html = m.create_element(QualName::new(None, ns!(html), "html".into()), vec![]);
        m.pre_insert(html, root, None).unwrap();
        let head = m.create_element(QualName::new(None, ns!(html), "head".into()), vec![]);
        m.pre_insert(head, html, None).unwrap();
        base(&mut m, head, "https://assets.test/files/");
        let img = m.create_element(QualName::new(None, ns!(html), "img".into()), vec![]);
        m.set_attribute(img, QualName::new(None, ns!(), "src".into()), "image.png");
        m.pre_insert(img, html, None).unwrap();
        let link = m.create_element(QualName::new(None, ns!(html), "link".into()), vec![]);
        m.set_attribute(link, QualName::new(None, ns!(), "rel".into()), "stylesheet");
        m.set_attribute(link, QualName::new(None, ns!(), "href".into()), "style.css");
        m.pre_insert(link, head, None).unwrap();
        let form = m.create_element(QualName::new(None, ns!(html), "form".into()), vec![]);
        m.set_attribute(form, QualName::new(None, ns!(), "action".into()), "submit");
        m.pre_insert(form, html, None).unwrap();
        form
    };
    doc.submit_form(form, form);
    doc.mutate()
        .clear_attribute(form, QualName::new(None, ns!(), "action".into()));
    doc.submit_form(form, form);
    doc.mutate()
        .set_attribute(form, QualName::new(None, ns!(), "action".into()), "");
    doc.submit_form(form, form);
    let button = {
        let mut m = doc.mutate();
        let button = m.create_element(QualName::new(None, ns!(html), "button".into()), vec![]);
        m.set_attribute(button, QualName::new(None, ns!(), "formaction".into()), "");
        m.pre_insert(button, form, None).unwrap();
        button
    };
    doc.mutate().set_attribute(
        form,
        QualName::new(None, ns!(), "action".into()),
        "explicit",
    );
    doc.submit_form(form, button);
    let seen = requests.0.lock().unwrap();
    assert_eq!(
        seen.iter()
            .filter(|url| url.as_str() == "https://owner.test/old/page?")
            .count(),
        3,
        "{seen:?}"
    );
    assert!(
        seen.iter()
            .any(|url| url == "https://assets.test/files/image.png"),
        "{seen:?}"
    );
    assert!(
        seen.iter()
            .any(|url| url == "https://assets.test/files/style.css"),
        "{seen:?}"
    );
    assert!(
        seen.iter()
            .any(|url| url == "https://assets.test/files/submit?"),
        "{seen:?}"
    );
    assert_eq!(
        doc.url().origin().ascii_serialization(),
        "https://owner.test"
    );
}

#[test]
fn embedded_and_constructed_sheets_capture_effective_base() {
    let (mut doc, body) = document();
    base(&mut doc.mutate(), body, "https://assets.test/css/");
    let style = {
        let mut m = doc.mutate();
        let style = m.create_element(QualName::new(None, ns!(html), "style".into()), vec![]);
        m.set_text_content(style, "p {background-image: url(image.png)}");
        m.pre_insert(style, body, None).unwrap();
        style
    };
    let embedded = doc.retain_stylesheet(style).unwrap();
    let constructed = doc.create_constructed_stylesheet();
    assert_eq!(
        doc.retained_stylesheet_url(&embedded).as_deref(),
        Some("https://assets.test/css/")
    );
    assert_eq!(
        doc.retained_stylesheet_url(&constructed).as_deref(),
        Some("https://assets.test/css/")
    );
    doc.set_base_url("https://owner.test/moved/page");
    assert_eq!(
        doc.retained_stylesheet_url(&embedded).as_deref(),
        Some("https://assets.test/css/")
    );
}

#[test]
fn secondary_document_stylesheet_uses_its_captured_fallback() {
    let (mut doc, body) = document();
    let child = doc.mutate().create_document_node();
    doc.mutate().document_metadata_mut(child).inherited_base_url =
        Some(Url::parse("https://child.test/captured/").unwrap());
    base(&mut doc.mutate(), body, "https://parent.test/current/");
    let style = {
        let mut m = doc.mutate();
        let style = m.create_element(QualName::new(None, ns!(html), "style".into()), vec![]);
        m.pre_insert(style, child, None).unwrap();
        m.set_text_content(style, "p {background-image:url(image.png)}");
        style
    };
    doc.process_style_element(style);
    let sheet = doc.retain_stylesheet(style).unwrap();
    let constructed = doc.create_constructed_stylesheet_for_document(child);
    for sheet in [&sheet, &constructed] {
        assert_eq!(
            doc.retained_stylesheet_url(sheet).as_deref(),
            Some("https://child.test/captured/")
        );
    }
}

#[test]
fn adopted_inline_styles_reparse_with_the_destination_document_base() {
    use crate::node::Attribute;
    use style::properties::PropertyDeclaration;
    use style::values::generics::image::GenericImage;
    let (mut doc, body) = document();
    base(&mut doc.mutate(), body, "https://parent.test/current/");
    let child = doc.mutate().create_document_node();
    doc.mutate().document_metadata_mut(child).inherited_base_url =
        Some(Url::parse("https://child.test/captured/").unwrap());
    let element = {
        let mut m = doc.mutate();
        let element = m.create_element(
            QualName::new(None, ns!(html), "p".into()),
            vec![Attribute {
                name: QualName::new(None, ns!(), "style".into()),
                value: "background-image:url(image.png)".into(),
            }],
        );
        m.pre_insert(element, child, None).unwrap();
        element
    };
    let guard = doc.guard.read();
    let block = doc.nodes[element]
        .element_data()
        .unwrap()
        .style_attribute
        .as_ref()
        .unwrap()
        .read_with(&guard);
    let PropertyDeclaration::BackgroundImage(images) = &block.declarations()[0] else {
        panic!("background image declaration");
    };
    let GenericImage::Url(url) = &images.0[0] else {
        panic!("URL image");
    };
    assert_eq!(
        url.url().unwrap().as_str(),
        "https://child.test/captured/image.png"
    );
}

#[test]
fn native_inline_style_edits_survive_document_adoption() {
    let (mut doc, body) = document();
    let node = {
        let mut m = doc.mutate();
        let node = m.create_element(QualName::new(None, ns!(html), "p".into()), vec![]);
        m.set_attribute(
            node,
            QualName::new(None, ns!(), "style".into()),
            "width:10px; height:30px",
        );
        m.pre_insert(node, body, None).unwrap();
        node
    };
    doc.set_style_property(node, "width", "20px");
    doc.remove_style_property(node, "height");
    let child = doc.mutate().create_document_node();
    doc.mutate().pre_insert(node, child, None).unwrap();
    let attr = doc.nodes[node].attr(crate::local_name!("style")).unwrap();
    assert_eq!(doc.style_attr_get_property(attr, "width"), "20px");
    assert_eq!(doc.style_attr_get_property(attr, "height"), "");
    let guard = doc.guard.read();
    let block = doc.nodes[node]
        .element_data()
        .unwrap()
        .style_attribute
        .as_ref()
        .unwrap()
        .read_with(&guard);
    let mut css = String::new();
    block.to_css(&mut css).unwrap();
    assert!(css.contains("width: 20px"), "{css}");
    assert!(!css.contains("height"), "{css}");
}
