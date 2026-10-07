//! Parser-owned roots, hydration and transactional admission.
use blitz_dom::shadow::{AttachShadowError, ShadowRootInit};
use blitz_dom::{BaseDocument, DocumentConfig};
use blitz_html::{DocumentHtmlParser, HtmlProvider};
use std::sync::Arc;

fn document(limit: usize) -> BaseDocument {
    let mut doc = BaseDocument::new(DocumentConfig {
        node_limit: Some(limit),
        html_parser_provider: Some(Arc::new(HtmlProvider)),
        ..Default::default()
    });
    DocumentHtmlParser::parse_into_mutator(
        &mut doc.mutate(),
        "<!doctype html><body><div id=host></div>",
    );
    doc
}

#[test]
fn navigation_parses_nested_roots_and_matching_hydration_clears_once() {
    let mut doc = document(128);
    let body = doc.query_selector("body").unwrap().unwrap();
    doc.mutate().try_set_html_unsafe(body,
        "<div id=parsed><template shadowrootmode=OPEN shadowrootclonable shadowrootserializable shadowrootdelegatesfocus><span>one</span><div id=nested><template shadowrootmode=closed>secret</template></div></template><template shadowrootmode=CLOSED>duplicate</template></div>"
    ).unwrap();
    let host = doc.query_selector("#parsed").unwrap().unwrap();
    let root = doc.shadow_root_of(host).unwrap();
    let metadata = doc
        .get_node(root)
        .unwrap()
        .shadow_root_data
        .as_ref()
        .unwrap();
    assert!(
        metadata.open
            && metadata.clonable
            && metadata.serializable
            && metadata.delegates_focus
            && metadata.declarative
    );
    let nested = doc.query_selector_in(root, "#nested").unwrap().unwrap();
    let closed = doc.shadow_root_of(nested).unwrap();
    assert!(
        !doc.get_node(closed)
            .unwrap()
            .shadow_root_data
            .as_ref()
            .unwrap()
            .open
    );
    assert_eq!(
        doc.query_selector_all_in(host, "template").unwrap().len(),
        1
    );
    assert!(
        doc.get_html(host, false, &[])
            .to_string()
            .contains("shadowrootmode=\"CLOSED\"")
    );
    assert_eq!(
        doc.mutate().attach_shadow(host, ShadowRootInit::default()),
        Err(AttachShadowError::AlreadyAttached)
    );
    assert_eq!(doc.shadow_root_of(host), Some(root));
    assert_eq!(
        doc.mutate().attach_shadow(
            host,
            ShadowRootInit {
                open: true,
                ..Default::default()
            }
        ),
        Ok(root)
    );
    assert!(doc.get_node(root).unwrap().children.is_empty());
    assert_eq!(
        doc.mutate().attach_shadow(
            host,
            ShadowRootInit {
                open: true,
                ..Default::default()
            }
        ),
        Err(AttachShadowError::AlreadyAttached)
    );
}

#[test]
fn inner_html_is_inert_but_document_parser_admits_roots() {
    let mut doc = document(128);
    let host = doc.query_selector("#host").unwrap().unwrap();
    doc.mutate()
        .try_set_inner_html_detaching(
            host,
            "<div><template shadowrootmode=open>inert</template></div>",
        )
        .unwrap();
    let inner = doc.query_selector_in(host, "div").unwrap().unwrap();
    assert_eq!(doc.shadow_root_of(inner), None);
    assert_eq!(
        doc.query_selector_all_in(inner, "template").unwrap().len(),
        1
    );
    let mut doc = document(128);
    DocumentHtmlParser::parse_into_mutator(
        &mut doc.mutate(),
        "<div id=parsed><template shadowrootmode=open>document</template></div>",
    );
    let host = doc.query_selector("#parsed").unwrap().unwrap();
    assert!(doc.shadow_root_of(host).is_some());
}

#[test]
fn failed_unsafe_fragments_release_shadow_allocations_and_keep_old_tree() {
    for spare in 0..24 {
        let mut doc = document(8 + spare);
        let host = doc.query_selector("#host").unwrap().unwrap();
        let before = doc.node_count();
        let html = "<div><template shadowrootmode=open><b>new</b></template></div>".repeat(100);
        assert!(doc.mutate().try_set_html_unsafe(host, &html).is_err());
        assert!(doc.get_node(host).unwrap().children.is_empty());
        assert_eq!(doc.node_count(), before, "spare={spare}");
    }
}

#[test]
fn registered_disabled_shadow_hosts_keep_the_template() {
    let mut doc = document(128);
    let document = doc.root_node().id;
    std::rc::Rc::make_mut(
        &mut doc
            .mutate()
            .document_metadata_mut(document)
            .disabled_shadow_definitions,
    )
    .insert("no-shadow".into(), "no-shadow".into());
    std::rc::Rc::make_mut(
        &mut doc
            .mutate()
            .document_metadata_mut(document)
            .disabled_shadow_definitions,
    )
    .insert("no-div-shadow".into(), "div".into());
    let host = doc.query_selector("#host").unwrap().unwrap();
    doc.mutate()
        .try_set_html_unsafe(
            host,
            "<no-shadow is=unknown><template shadowrootmode=open>secret</template></no-shadow><div is=no-div-shadow><template shadowrootmode=open>secret</template></div><div id=ordinary><template shadowrootmode=open>public</template></div>",
        )
        .unwrap();
    let custom = doc.query_selector("no-shadow").unwrap().unwrap();
    let customized = doc.query_selector("div[is]").unwrap().unwrap();
    assert_eq!(doc.shadow_root_of(customized), None);
    let ordinary = doc.query_selector("#ordinary").unwrap().unwrap();
    assert!(doc.shadow_root_of(ordinary).is_some());
    assert_eq!(doc.shadow_root_of(custom), None);
    assert_eq!(
        doc.query_selector_all_in(custom, "template").unwrap().len(),
        1
    );
}

#[test]
fn structural_selectors_on_shadow_children_survive_sibling_mutations() {
    let mut doc = document(128);
    let host = doc.query_selector("#host").unwrap().unwrap();
    doc.mutate().try_set_html_unsafe(host,
        "<div id=styled><template shadowrootmode=open><style>b { display:block; width:11px } b:first-child { width:31px } b:nth-child(2) { width:22px }</style><b id=first>one</b><b id=second>two</b></template></div>"
    ).unwrap();
    let styled = doc.query_selector("#styled").unwrap().unwrap();
    let root = doc.shadow_root_of(styled).unwrap();
    let first = doc.query_selector_in(root, "#first").unwrap().unwrap();
    let second = doc.query_selector_in(root, "#second").unwrap().unwrap();
    doc.resolve(0.0);
    assert_eq!(doc.get_node(first).unwrap().final_layout().size.width, 22.0);
    assert_eq!(
        doc.get_node(second).unwrap().final_layout().size.width,
        11.0
    );
    doc.mutate().remove_node(second);
    doc.resolve(0.0);
    assert_eq!(doc.get_node(first).unwrap().final_layout().size.width, 22.0);
    doc.mutate().insert_nodes_before(first, &[second]);
    doc.resolve(0.0);
    assert_eq!(
        doc.get_node(second).unwrap().final_layout().size.width,
        22.0
    );
    assert_eq!(doc.get_node(first).unwrap().final_layout().size.width, 11.0);
}
