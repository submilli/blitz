use crate::{BaseDocument, DomString, HtmlOutputLimit, NodeId, QualName, ns};
fn element(doc: &mut BaseDocument, name: &str) -> NodeId {
    doc.mutate()
        .try_create_element(QualName::new(None, ns!(html), name.into()), vec![])
        .unwrap()
}

#[test]
fn bounded_output_matches_existing_markup_at_exact_utf16_boundary() {
    let mut doc = BaseDocument::new(Default::default());
    let root = element(&mut doc, "div");
    doc.mutate()
        .set_attribute(root, QualName::new(None, ns!(), "title".into()), "\"&<> ");
    let text = doc.mutate().try_create_text_node("a<&> 😀").unwrap();
    let comment = doc.mutate().try_create_comment_node("note").unwrap();
    doc.mutate().append_children(root, &[text, comment]);
    let expected = doc.get_node(root).unwrap().outer_html_dom();
    let units = expected.utf16_units().count();
    assert_eq!(doc.try_outer_html(root, units), Ok(expected));
    assert_eq!(doc.try_outer_html(root, units - 1), Err(HtmlOutputLimit));
    assert_eq!(doc.try_inner_html(root, 0), Err(HtmlOutputLimit));
}

#[test]
fn bounded_output_keeps_lone_surrogates_and_raw_text() {
    let mut doc = BaseDocument::new(Default::default());
    for name in ["div", "script"] {
        let root = element(&mut doc, name);
        let text = doc
            .mutate()
            .try_create_text_node(DomString::from_utf16(vec![0xd800, 60, 38, 0xdc00]))
            .unwrap();
        doc.mutate().append_children(root, &[text]);
        let expected = doc.get_node(root).unwrap().inner_html_dom();
        let units = expected.utf16_units().count();
        assert_eq!(doc.try_inner_html(root, units), Ok(expected));
        assert_eq!(doc.try_inner_html(root, units - 1), Err(HtmlOutputLimit));
    }
}

#[test]
fn output_rejection_does_not_copy_an_oversized_input_projection() {
    let mut doc = BaseDocument::new(Default::default());
    let root = element(&mut doc, "div");
    let text = doc
        .mutate()
        .try_create_text_node("x".repeat(4_000_000))
        .unwrap();
    doc.mutate().append_children(root, &[text]);
    let peak = crate::test_allocator::peak_held(|| {
        assert_eq!(doc.try_inner_html(root, 16), Err(HtmlOutputLimit));
    });
    assert!(
        peak < 64 * 1024,
        "oversized escaped input allocated {peak} bytes"
    );
    let script = element(&mut doc, "script");
    doc.mutate().append_children(script, &[text]);
    let peak = crate::test_allocator::peak_held(|| {
        assert_eq!(doc.try_inner_html(script, 16), Err(HtmlOutputLimit));
    });
    assert!(
        peak < 64 * 1024,
        "oversized raw input allocated {peak} bytes"
    );
}

#[test]
fn escaping_amplification_is_counted_before_growing_output() {
    let mut doc = BaseDocument::new(Default::default());
    let root = element(&mut doc, "div");
    let text = doc.mutate().try_create_text_node("&".repeat(1000)).unwrap();
    doc.mutate().append_children(root, &[text]);
    assert_eq!(doc.try_inner_html(root, 4999), Err(HtmlOutputLimit));
    assert_eq!(
        doc.try_inner_html(root, 5000).unwrap(),
        "&amp;".repeat(1000).as_str()
    );
}

#[test]
fn templates_void_tags_and_namespaces_use_the_same_bounded_serializer() {
    let mut doc = BaseDocument::new(Default::default());
    let template = element(&mut doc, "template");
    let contents = doc.mutate().try_ensure_template_contents(template).unwrap();
    let br = element(&mut doc, "br");
    let own = doc.mutate().try_create_comment_node("own").unwrap();
    doc.mutate().append_children(contents, &[br]);
    doc.mutate().append_children(template, &[own]);
    assert_eq!(
        doc.try_outer_html(template, 25).unwrap(),
        "<template><br></template>"
    );
    let xml = doc
        .mutate()
        .try_create_element(
            QualName::new(Some("p".into()), "urn:a".into(), "x".into()),
            vec![],
        )
        .unwrap();
    assert_eq!(doc.try_outer_html(xml, 11).unwrap(), "<p:x></p:x>");
    assert_eq!(doc.try_outer_html(xml, 10), Err(HtmlOutputLimit));
}

#[test]
fn large_depth_remains_iterative_when_output_is_bounded() {
    let mut doc = BaseDocument::new(Default::default());
    let root = element(&mut doc, "b");
    let mut leaf = root;
    for _ in 0..10_000 {
        let child = element(&mut doc, "b");
        doc.mutate().append_children(leaf, &[child]);
        leaf = child;
    }
    assert_eq!(
        doc.try_outer_html(root, 70_007)
            .unwrap()
            .utf16_units()
            .count(),
        70_007
    );
    assert_eq!(doc.try_outer_html(root, 70_006), Err(HtmlOutputLimit));
}
