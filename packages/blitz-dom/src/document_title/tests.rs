use super::*;
use crate::{DocumentConfig, NodeData};

fn document() -> BaseDocument {
    BaseDocument::new(DocumentConfig::default())
}
fn html(doc: &mut BaseDocument, title: Option<&str>) -> NodeId {
    doc.mutate()
        .try_create_html_document(title.map(Into::into))
        .unwrap()
}
fn element(doc: &mut BaseDocument, name: &str, svg: bool) -> NodeId {
    doc.mutate().create_element(
        QualName::new(None, if svg { ns!(svg) } else { ns!(html) }, name.into()),
        Vec::new(),
    )
}
fn units(doc: &BaseDocument, d: NodeId, limit: usize) -> Option<Vec<u16>> {
    doc.document_title_with_limit(d, limit)
        .map(|s| s.utf16_units().collect())
}
#[test]
fn title_uses_first_ordinary_html_title_and_only_child_text() {
    let mut doc = document();
    let d = html(&mut doc, Some("ignored"));
    let first = doc.document_title_element(d).unwrap();
    let nested = element(&mut doc, "b", false);
    let hidden = doc.mutate().create_text_node("nested ignored");
    let a = doc.mutate().create_text_node("  a\t");
    let b = doc.mutate().create_text_node("\n b\u{b}\u{a0} \r");
    let comment = doc.mutate().create_comment_node("comment ignored");
    let mut m = doc.mutate();
    m.append_children(nested, &[hidden]);
    m.replace_all(first, None);
    m.append_children(first, &[a, nested, comment, b]);
    drop(m);
    assert_eq!(
        doc.document_title_with_limit(d, 64).unwrap(),
        DomString::from("a b\u{b}\u{a0}")
    );
    let second = element(&mut doc, "title", false);
    let t = doc.mutate().create_text_node("second");
    doc.mutate().append_children(second, &[t]);
    let head = doc.document_head(d).unwrap();
    doc.mutate().append_children(head, &[second]);
    assert_eq!(doc.document_title_element(d), Some(first));
    doc.mutate().remove_node(first);
    assert_eq!(doc.document_title_element(d), Some(second));
    doc.mutate().remove_node(second);
    assert_eq!(units(&doc, d, 0), Some(vec![]));
}
#[test]
fn title_output_admits_normalized_units_and_preserves_split_surrogates() {
    let mut doc = document();
    let d = html(&mut doc, Some("  a   b  "));
    assert_eq!(units(&doc, d, 2), None);
    assert_eq!(units(&doc, d, 3), Some(vec![97, 32, 98]));
    let title = doc.document_title_element(d).unwrap();
    doc.mutate().set_text_content(title, " ".repeat(100_000));
    assert_eq!(units(&doc, d, 0), Some(vec![]));
    doc.mutate().set_text_content(title, "");
    for value in [vec![0xd800], vec![0xdc00, 32, 0xdfff]] {
        let text = doc.mutate().create_text_node(DomString::from_utf16(value));
        doc.mutate().append_children(title, &[text]);
    }
    assert_eq!(units(&doc, d, 3), None);
    assert_eq!(units(&doc, d, 4), Some(vec![0xd800, 0xdc00, 32, 0xdfff]));
}
#[test]
fn title_setter_creates_in_head_and_preserves_inert_document_ownership() {
    let mut doc = document();
    let d = html(&mut doc, None);
    let before = doc.node_count();
    doc.mutate()
        .try_set_document_title(d, "raw<&\0\r\n".into())
        .unwrap();
    let title = doc.document_title_element(d).unwrap();
    let text = doc.get_node(title).unwrap().children[0];
    assert_eq!(doc.node_count(), before + 2);
    assert_eq!(doc.node_document(title), d);
    assert_eq!(doc.node_document(text), d);
    assert_eq!(
        doc.character_data_dom(text),
        Some(&DomString::from("raw<&\0\r\n"))
    );
    doc.mutate()
        .try_set_document_title(d, DomString::default())
        .unwrap();
    assert!(doc.get_node(title).unwrap().children.is_empty());
    assert!(doc.get_node(text).is_some());
    let head = doc.document_head(d).unwrap();
    doc.mutate().remove_node(head);
    let before = doc.node_count();
    doc.mutate()
        .try_set_document_title(d, "ignored".into())
        .unwrap();
    assert_eq!(doc.node_count(), before);
    assert_eq!(units(&doc, d, 0), Some(vec![]));
}
#[test]
fn svg_title_uses_first_direct_svg_child_and_inserts_before_other_children() {
    let mut doc = document();
    let d = doc.mutate().create_document_node();
    let root = element(&mut doc, "svg", true);
    let group = element(&mut doc, "g", true);
    let nested = element(&mut doc, "title", true);
    doc.mutate().append_children(group, &[nested]);
    doc.mutate().append_children(root, &[group]);
    doc.mutate().append_children(d, &[root]);
    assert_eq!(units(&doc, d, 0), Some(vec![]));
    doc.mutate()
        .try_set_document_title(d, "svg".into())
        .unwrap();
    let title = doc.document_title_element(d).unwrap();
    assert_ne!(title, nested);
    assert_eq!(doc.get_node(root).unwrap().children[0], title);
    assert_eq!(doc.node_document(title), d);
    assert_eq!(
        doc.get_node(title).unwrap().element_data().unwrap().name.ns,
        ns!(svg)
    );
    doc.mutate()
        .try_set_document_title(d, DomString::from_utf16(vec![0xdfff]))
        .unwrap();
    assert_eq!(units(&doc, d, 1), Some(vec![0xdfff]));
}
#[test]
fn unsupported_document_roots_ignore_title_writes_even_with_an_html_title() {
    let mut doc = document();
    let d = doc.mutate().create_document_node();
    let root = doc.mutate().create_element(
        QualName::new(None, "urn:test".into(), "root".into()),
        Vec::new(),
    );
    let title = element(&mut doc, "title", false);
    let t = doc.mutate().create_text_node("before");
    doc.mutate().append_children(title, &[t]);
    doc.mutate().append_children(root, &[title]);
    doc.mutate().append_children(d, &[root]);
    let before = doc.node_count();
    doc.mutate()
        .try_set_document_title(d, "ignored".into())
        .unwrap();
    assert_eq!(doc.node_count(), before);
    assert_eq!(doc.document_title_with_limit(d, 6), Some("before".into()));
}
#[test]
fn title_quota_rejection_is_atomic_and_empty_replacement_needs_no_slot() {
    let mut doc = BaseDocument::new(DocumentConfig {
        node_limit: Some(9),
        ..Default::default()
    });
    let d = html(&mut doc, None);
    let before = doc.node_count();
    assert_eq!(
        doc.mutate()
            .try_set_document_title(d, "requires two slots".into()),
        Err(NodeBudgetExceeded)
    );
    assert_eq!(doc.node_count(), before);
    assert_eq!(doc.document_title_element(d), None);
    doc.mutate()
        .try_set_document_title(d, DomString::default())
        .unwrap();
    assert_eq!(doc.node_count(), 9);
    let title = doc.document_title_element(d).unwrap();
    assert_eq!(
        doc.mutate()
            .try_set_document_title(d, "requires text".into()),
        Err(NodeBudgetExceeded)
    );
    assert!(doc.get_node(title).unwrap().children.is_empty());
    doc.mutate()
        .try_set_document_title(d, DomString::default())
        .unwrap();
    assert_eq!(doc.node_count(), 9);
}
#[test]
fn document_url_output_checks_bytes_before_copy_and_never_uses_the_base() {
    let mut doc = document();
    let root = doc.root_node().id;
    doc.set_base_url("https://root.test/page#hash");
    let value = "https://root.test/page#hash";
    assert_eq!(
        doc.document_url_str_with_limit(root, value.len()),
        Some(value)
    );
    assert_eq!(doc.document_url_str_with_limit(root, value.len() - 1), None);
    let d = html(&mut doc, None);
    assert_eq!(doc.document_url_str_with_limit(d, 11), Some("about:blank"));
    assert_eq!(doc.document_url_str_with_limit(d, 10), None);
    doc.mutate().document_metadata_mut(d).url =
        Some(url::Url::parse("https://inert.test/doc").unwrap());
    doc.mutate().document_metadata_mut(d).inherited_base_url =
        Some(url::Url::parse("https://base.test/dir/").unwrap());
    assert_eq!(
        doc.document_url_str_with_limit(d, 128),
        Some("https://inert.test/doc")
    );
    assert_eq!(doc.document_base_url(d).as_str(), "https://base.test/dir/");
    assert!(matches!(
        doc.get_node(d).unwrap().data,
        NodeData::Document(_)
    ));
}

#[test]
fn html_div_root_preserves_the_existing_browser_head_lookup_behavior() {
    let mut doc = document();
    let d = doc.mutate().create_document_node();
    let root = element(&mut doc, "div", false);
    let head = element(&mut doc, "head", false);
    doc.mutate().append_children(root, &[head]);
    doc.mutate().append_children(d, &[root]);
    doc.mutate()
        .try_set_document_title(d, "created".into())
        .unwrap();
    assert_eq!(doc.document_title_with_limit(d, 7), Some("created".into()));
    assert_eq!(doc.get_node(head).unwrap().children.len(), 1);
}
