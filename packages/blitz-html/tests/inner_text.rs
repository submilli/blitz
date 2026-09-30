mod common;
use blitz_dom::{BaseDocument, DocumentConfig, DomString, NodeBudgetExceeded, QualName};
use common::{parse, q};

#[test]
fn rendered_text_uses_resolved_style_and_html_breaks() {
    for (content, style, expected) in [
        ("a<span hidden>b</span>", "", "a"),
        ("a<span style='display:inline-block'> b </span>c", "", "abc"),
        ("a <img> b", "", "a  b"),
        ("<img> b", "", " b"),
        ("a <img style='visibility:hidden'> b", "", "a  b"),
        ("a<span style='display:inline-flex'> b </span>c", "", "abc"),
        ("a<span style='display:inline-grid'> b </span>c", "", "abc"),
        (
            "foo_bar a’b áb",
            "text-transform:capitalize",
            "Foo_bar A’b Áb",
        ),
        ("ΟΣ", "text-transform:lowercase", "ος"),
        (
            "hello-world foo.bar",
            "text-transform:capitalize",
            "Hello-World Foo.Bar",
        ),
        (
            "<span lang='tr' style='text-transform:uppercase'>i</span>",
            "",
            "İ",
        ),
        ("a<div>b</div><div>c</div>d", "", "a\nb\nc\nd"),
        ("a<p>b</p><p>c</p>d", "", "a\n\nb\n\nc\n\nd"),
        ("a<div></div><div></div>b", "", "a\nb"),
        ("<br>a<br><br>b<br>", "", "\na\n\nb\n"),
        ("  a \t<span> b </span> \n c  ", "", "a b c"),
        ("  a \t b\n c  ", "white-space:pre", "  a \t b\n c  "),
        ("  a \t b\n c  ", "white-space:pre-line", "a b\nc"),
        (
            "a<span style='visibility:hidden'>hidden<b style='visibility:visible'>shown</b></span>z",
            "",
            "ashownz",
        ),
        (
            "a<span style='display:none'>hidden<b style='display:block'>hidden</b></span>z",
            "",
            "az",
        ),
        (
            "a <span style='text-transform:uppercase'>hello ß</span> z",
            "",
            "a HELLO SS z",
        ),
        (
            "<table><tr><td>a</td><td>b</td></tr><tr><td>c</td><td>d</td></tr></table>",
            "",
            "a\tb\nc\td",
        ),
    ] {
        let mut doc = parse(&format!("<div id='target' style='{style}'>{content}</div>"));
        let target = q(&doc, "#target");
        doc.resolve(0.0);
        assert_eq!(
            doc.inner_text(target).as_str_lossy(),
            expected,
            "{content}, {style}"
        );
    }
}

#[test]
fn unrendered_roots_return_raw_text_and_mutations_refresh_rendering() {
    let mut doc =
        parse("<div id='target' style='display:none'>a<div>b</div><span hidden>c</span></div>");
    let target = q(&doc, "#target");
    doc.resolve(0.0);
    assert_eq!(doc.inner_text(target).as_str_lossy(), "abc");
    doc.mutate().remove_node(target);
    assert_eq!(doc.inner_text(target).as_str_lossy(), "abc");
}

#[test]
fn setter_normalizes_breaks_and_preserves_utf16() {
    let mut doc = parse("<div id='target'>old</div>");
    let target = q(&doc, "#target");
    doc.mutate()
        .try_set_inner_text(target, "a\r\nb\rc\n\n")
        .unwrap();
    assert_eq!(
        doc.get_node(target).unwrap().inner_html(),
        "a<br>b<br>c<br><br>"
    );
    let text = DomString::from_utf16(vec![0xd800, 13, 10, 0xdfff]);
    doc.mutate().try_set_inner_text(target, text).unwrap();
    doc.resolve(0.0);
    assert_eq!(
        doc.inner_text(target).to_utf16().as_ref(),
        &[0xd800, 10, 0xdfff]
    );
    doc.mutate().try_set_inner_text(target, "").unwrap();
    assert!(doc.get_node(target).unwrap().children.is_empty());
}

#[test]
fn rejected_setter_keeps_tree_and_node_count() {
    let mut doc = BaseDocument::new(DocumentConfig {
        node_limit: Some(4),
        ..Default::default()
    });
    let target = doc.mutate().create_element(
        QualName::new(None, blitz_dom::ns!(html), "div".into()),
        vec![],
    );
    doc.mutate().try_set_inner_text(target, "old").unwrap();
    let count = doc.node_count();
    assert_eq!(
        doc.mutate().try_set_inner_text(target, "a\nb"),
        Err(NodeBudgetExceeded)
    );
    assert_eq!(doc.node_count(), count);
    assert_eq!(doc.text_content_of(target).as_deref(), Some("old"));
    doc.mutate().try_set_inner_text(target, "").unwrap();
}

#[test]
fn replaced_root_does_not_expose_dom_text() {
    let mut doc = parse("<textarea>hidden</textarea>");
    let target = q(&doc, "textarea");
    doc.resolve(0.0);
    assert!(doc.inner_text(target).is_empty());
}

#[test]
fn connected_shadow_descendants_use_rendered_text() {
    let mut doc = parse("<div id='host'></div>");
    let host = q(&doc, "#host");
    let shadow = doc
        .mutate()
        .attach_shadow(host, blitz_dom::shadow::ShadowRootInit::default())
        .unwrap();
    let target = doc.mutate().create_element(
        QualName::new(None, blitz_dom::ns!(html), "div".into()),
        vec![],
    );
    doc.mutate()
        .set_inner_html(target, "a<span hidden>b</span>");
    doc.mutate().append_children(shadow, &[target]);
    doc.resolve(0.0);
    assert_eq!(doc.inner_text(target).as_str_lossy(), "a");
}

#[test]
fn insertion_rejects_shadow_host_cycles() {
    let mut doc = parse("<div id='host'></div>");
    let host = q(&doc, "#host");
    doc.mutate().remove_node(host);
    let shadow = doc
        .mutate()
        .attach_shadow(host, blitz_dom::shadow::ShadowRootInit::default())
        .unwrap();
    assert_eq!(
        doc.check_pre_insert(host, shadow, None),
        Err(blitz_dom::dom_api::DomError::HierarchyRequest)
    );
    assert!(!doc.is_connected(host));
}
