use super::*;
use crate::{DocumentType, QualName};

fn element(doc: &mut BaseDocument, namespace: markup5ever::Namespace, local: &str) -> NodeId {
    doc.mutate()
        .try_create_element(QualName::new(None, namespace, local.into()), Vec::new())
        .unwrap()
}
fn attribute(doc: &mut BaseDocument, id: NodeId, name: &str, value: DomString) {
    doc.mutate()
        .set_attribute(id, QualName::new(None, ns!(), name.into()), value);
}
fn query(doc: &BaseDocument, root: NodeId, filter: ElementFilter) -> Vec<NodeId> {
    doc.filtered_descendant_elements(root, &filter, usize::MAX)
        .unwrap()
}

#[test]
fn descendant_queries_preserve_preorder_and_exclude_the_scope() {
    let mut doc = BaseDocument::new(Default::default());
    let root = doc.root_node().id;
    let scope = element(&mut doc, ns!(html), "div");
    let first = element(&mut doc, ns!(html), "p");
    let nested = element(&mut doc, ns!(html), "span");
    let second = element(&mut doc, ns!(html), "p");
    doc.mutate().append_children(root, &[scope]);
    doc.mutate().append_children(scope, &[first, second]);
    doc.mutate().append_children(first, &[nested]);
    assert_eq!(
        query(&doc, scope, ElementFilter::qualified_name("*".into())),
        [first, nested, second]
    );
    assert!(query(&doc, scope, ElementFilter::qualified_name("div".into())).is_empty());
    doc.mutate().pre_insert(second, scope, Some(first)).unwrap();
    assert_eq!(
        query(&doc, scope, ElementFilter::qualified_name("p".into())),
        [second, first]
    );
}

#[test]
fn qualified_names_fold_only_html_names_in_html_documents() {
    let mut doc = BaseDocument::new(Default::default());
    let root = doc.root_node().id;
    let html = element(&mut doc, ns!(html), "div");
    let svg = element(&mut doc, ns!(svg), "linearGradient");
    let upper = element(&mut doc, ns!(html), "DIV");
    doc.mutate().append_children(root, &[html]);
    doc.mutate().append_children(html, &[svg, upper]);
    assert_eq!(
        query(&doc, root, ElementFilter::qualified_name("DIV".into())),
        [html]
    );
    assert_eq!(
        query(
            &doc,
            root,
            ElementFilter::qualified_name("linearGradient".into())
        ),
        [svg]
    );
    assert!(
        query(
            &doc,
            root,
            ElementFilter::qualified_name("lineargradient".into())
        )
        .is_empty()
    );
    doc.mutate().document_metadata_mut(root).document_type = DocumentType::Xml;
    assert_eq!(
        query(&doc, root, ElementFilter::qualified_name("DIV".into())),
        [upper]
    );
    assert_eq!(
        query(&doc, root, ElementFilter::qualified_name("div".into())),
        [html]
    );
}

#[test]
fn qualified_names_include_prefixes_and_never_project_lone_surrogates() {
    let mut doc = BaseDocument::new(Default::default());
    let root = doc.root_node().id;
    let id = doc
        .mutate()
        .try_create_element(
            QualName::new(Some("p".into()), "urn:x".into(), "x".into()),
            Vec::new(),
        )
        .unwrap();
    let replacement = element(&mut doc, ns!(svg), "\u{fffd}");
    doc.mutate().append_children(root, &[id]);
    doc.mutate().append_children(id, &[replacement]);
    assert_eq!(
        query(&doc, root, ElementFilter::qualified_name("p:x".into())),
        [id]
    );
    assert!(query(&doc, root, ElementFilter::qualified_name("x".into())).is_empty());
    assert!(
        query(
            &doc,
            root,
            ElementFilter::qualified_name(DomString::from_utf16(vec![0xd800]))
        )
        .is_empty()
    );
}

#[test]
fn classes_use_exact_infra_space_tokens_and_reject_an_empty_query() {
    let mut doc = BaseDocument::new(Default::default());
    let root = doc.root_node().id;
    let id = element(&mut doc, ns!(html), "div");
    doc.mutate().append_children(root, &[id]);
    attribute(&mut doc, id, "class", "a\tb\nc\rd\u{c}e f\u{b}g".into());
    assert_eq!(
        query(
            &doc,
            root,
            ElementFilter::class_names(&" a b c d e a ".into())
        ),
        [id]
    );
    assert_eq!(
        query(&doc, root, ElementFilter::class_names(&"f\u{b}g".into())),
        [id]
    );
    assert!(query(&doc, root, ElementFilter::class_names(&"f g".into())).is_empty());
    assert!(query(&doc, root, ElementFilter::class_names(&" \t\n".into())).is_empty());
}

#[test]
fn quirks_folds_ascii_class_units_without_unicode_case_folding() {
    let mut doc = BaseDocument::new(Default::default());
    let root = doc.root_node().id;
    let id = element(&mut doc, ns!(svg), "g");
    doc.mutate().append_children(root, &[id]);
    attribute(&mut doc, id, "class", "UPPER É".into());
    assert!(query(&doc, root, ElementFilter::class_names(&"upper".into())).is_empty());
    doc.mutate().document_metadata_mut(root).quirks = true;
    assert_eq!(
        query(&doc, root, ElementFilter::class_names(&"upper É".into())),
        [id]
    );
    assert!(query(&doc, root, ElementFilter::class_names(&"upper é".into())).is_empty());
}

#[test]
fn lossless_classes_distinguish_surrogates_from_replacement_characters() {
    let mut doc = BaseDocument::new(Default::default());
    let root = doc.root_node().id;
    let lone = element(&mut doc, ns!(html), "div");
    let replacement = element(&mut doc, ns!(html), "span");
    doc.mutate().append_children(root, &[lone]);
    doc.mutate().append_children(lone, &[replacement]);
    let value = DomString::from_utf16(vec![0xd800]);
    attribute(&mut doc, lone, "class", value.clone());
    attribute(&mut doc, replacement, "class", "\u{fffd}".into());
    assert_eq!(
        query(&doc, root, ElementFilter::class_names(&value)),
        [lone]
    );
    assert_eq!(
        query(&doc, root, ElementFilter::class_names(&"\u{fffd}".into())),
        [replacement]
    );
}

#[test]
fn many_classes_use_exact_hashed_buckets_and_quirks_folding() {
    let mut doc = BaseDocument::new(Default::default());
    let root = doc.root_node().id;
    let id = element(&mut doc, ns!(html), "div");
    doc.mutate().append_children(root, &[id]);
    let classes = (0..64)
        .map(|n| format!("c{n}"))
        .collect::<Vec<_>>()
        .join(" ");
    attribute(&mut doc, id, "class", classes.clone().into());
    assert_eq!(
        query(
            &doc,
            root,
            ElementFilter::class_names(&classes.clone().into())
        ),
        [id]
    );
    assert!(
        query(
            &doc,
            root,
            ElementFilter::class_names(&classes.to_ascii_uppercase().into())
        )
        .is_empty()
    );
    doc.mutate().document_metadata_mut(root).quirks = true;
    assert_eq!(
        query(
            &doc,
            root,
            ElementFilter::class_names(&classes.to_ascii_uppercase().into())
        ),
        [id]
    );
    assert!(
        query(
            &doc,
            root,
            ElementFilter::class_names(&(classes + " missing").into())
        )
        .is_empty()
    );
}

#[test]
fn names_match_only_exact_html_null_namespace_attribute_values() {
    let mut doc = BaseDocument::new(Default::default());
    let root = doc.root_node().id;
    let html = element(&mut doc, ns!(html), "input");
    let svg = element(&mut doc, ns!(svg), "g");
    doc.mutate().append_children(root, &[html]);
    doc.mutate().append_children(html, &[svg]);
    let value = DomString::from_utf16(vec![0xd800]);
    attribute(&mut doc, html, "name", value.clone());
    attribute(&mut doc, svg, "name", value.clone());
    assert_eq!(query(&doc, root, ElementFilter::name(value)), [html]);
    assert!(query(&doc, root, ElementFilter::name("\u{fffd}".into())).is_empty());
    assert!(query(&doc, root, ElementFilter::name("".into())).is_empty());
    doc.mutate()
        .clear_attribute(html, QualName::new(None, ns!(), "name".into()));
    doc.mutate().set_attribute(
        html,
        QualName::new(Some("p".into()), "urn:x".into(), "name".into()),
        DomString::from_utf16(vec![0xd800]),
    );
    assert!(
        query(
            &doc,
            root,
            ElementFilter::name(DomString::from_utf16(vec![0xd800]))
        )
        .is_empty()
    );
    attribute(&mut doc, html, "name", "".into());
    assert_eq!(query(&doc, root, ElementFilter::name("".into())), [html]);
}

#[test]
fn output_limits_admit_exact_matches_and_keep_inert_trees_separate() {
    let mut doc = BaseDocument::new(Default::default());
    let root = doc.root_node().id;
    let template = element(&mut doc, ns!(html), "template");
    doc.mutate().append_children(root, &[template]);
    let contents = doc.mutate().try_ensure_template_contents(template).unwrap();
    let inert = element(&mut doc, ns!(html), "p");
    doc.mutate().append_children(contents, &[inert]);
    let filter = ElementFilter::qualified_name("*".into());
    assert!(doc.filtered_descendant_elements(root, &filter, 0).is_err());
    assert_eq!(
        doc.filtered_descendant_elements(root, &filter, 1).unwrap(),
        [template]
    );
    assert_eq!(
        doc.filtered_descendant_elements(contents, &filter, 1)
            .unwrap(),
        [inert]
    );
    assert!(
        doc.filtered_descendant_elements(
            contents,
            &ElementFilter::qualified_name("missing".into()),
            0
        )
        .unwrap()
        .is_empty()
    );
}
