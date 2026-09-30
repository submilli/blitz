//! Parser amplification must stop allocating without inventing live node IDs.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use blitz_dom::{BaseDocument, DocumentConfig};
use blitz_html::{DocumentHtmlParser, HtmlProvider, ParseStep, StreamingParser};

fn document(limit: usize) -> BaseDocument {
    BaseDocument::new(DocumentConfig {
        node_limit: Some(limit),
        html_parser_provider: Some(Arc::new(HtmlProvider)),
        ..DocumentConfig::default()
    })
}

fn amplified_markup() -> String {
    format!(
        "<p>{}</p>{}<div id=tail></div>",
        "<b>".repeat(64),
        "<p>x</p>".repeat(64)
    )
}

#[test]
fn adjacent_markup_records_only_the_real_target() {
    use blitz_dom::mutations::MutationRecord;

    for position in ["beforebegin", "afterbegin", "beforeend", "afterend"] {
        let mut doc = document(64);
        DocumentHtmlParser::parse_into_mutator(&mut doc.mutate(), "<body><div></div>");
        let target = doc.query_selector("div").unwrap().unwrap();
        let body = doc.query_selector("body").unwrap().unwrap();
        let parent = if matches!(position, "beforebegin" | "afterend") {
            body
        } else {
            target
        };
        doc.set_mutation_recording(true);
        doc.mutate()
            .insert_adjacent_html(target, position, "<b>x</b><i>y</i>")
            .unwrap();
        let records = doc.take_mutation_records();
        assert_eq!(records.len(), 1, "{position}: {records:?}");
        assert!(matches!(
            &records[0],
            MutationRecord::ChildList { target, added, removed, .. }
                if *target == parent && added.len() == 2 && removed.is_empty()
        ));
    }
}

#[test]
fn active_formatting_amplification_stays_within_budget() {
    let mut doc = document(128);
    DocumentHtmlParser::parse_into_mutator(&mut doc.mutate(), &amplified_markup());
    assert_eq!(doc.node_count(), 128);
    assert!(doc.query_selector("#tail").unwrap().is_none());
    let body = doc.query_selector("body").unwrap().unwrap();
    doc.mutate().remove_and_drop_all_children(body);
    let text = doc.mutate().try_create_text_node("still usable").unwrap();
    doc.mutate().append_children(body, &[text]);
    assert_eq!(doc.text_content_of(body).as_deref(), Some("still usable"));
}

#[test]
fn fragment_limits_handle_scratch_roots_templates_and_foster_parenting() {
    for spare in 0..16 {
        let mut doc = document(6 + spare);
        DocumentHtmlParser::parse_into_mutator(&mut doc.mutate(), "<body><div></div>");
        let target = doc.query_selector("div").unwrap().unwrap();
        DocumentHtmlParser::parse_inner_html_into_mutator(
            &mut doc.mutate(),
            target,
            "<template><b><i>x</b>y</i></template><table>a<tr><td>b</table>",
        );
        assert!(doc.node_count() <= doc.node_limit(), "spare={spare}");
        assert!(doc.get_node(target).is_some());
    }
}

#[test]
fn streaming_does_not_surface_a_script_after_admission_stops() {
    let doc = Rc::new(RefCell::new(document(128)));
    let mut parser = StreamingParser::new(doc.clone());
    parser.feed(&format!(
        "{}<script>unreachable()</script>",
        amplified_markup()
    ));
    parser.end_of_input();
    assert!(matches!(parser.run(), ParseStep::Done));
    assert!(parser.is_finished());
    assert_eq!(doc.borrow().node_count(), 128);
}

#[test]
fn xml_and_minimal_document_budgets_never_need_a_fake_node() {
    for limit in 1..16 {
        let mut doc = document(limit);
        DocumentHtmlParser::parse_xml_into_mutator(
            &mut doc.mutate(),
            "<html><body><a/><b/><c/><d/></body></html>",
        );
        assert!(doc.node_count() <= limit);
        let mut doc = document(limit);
        DocumentHtmlParser::parse_into_mutator(&mut doc.mutate(), "<template><p>x</p></template>");
        assert!(doc.node_count() <= limit);
    }
}

#[test]
fn exhausted_streaming_parser_refuses_later_network_and_written_input() {
    let doc = Rc::new(RefCell::new(document(1)));
    let mut parser = StreamingParser::new(doc.clone());
    parser.feed("<div>");
    assert!(matches!(parser.run(), ParseStep::Done));
    assert!(parser.is_finished());
    for _ in 0..1_000 {
        parser.feed("<div><b><template>");
        parser.write("<section><script>bad()</script>");
        assert!(matches!(parser.run(), ParseStep::Done));
        assert!(matches!(parser.run_written(), ParseStep::Done));
    }
    assert_eq!(doc.borrow().node_count(), 1);
}

#[test]
fn implied_eof_nodes_and_detached_documents_obey_admission() {
    for limit in 1..8 {
        let mut doc = document(limit);
        DocumentHtmlParser::parse_into_mutator(&mut doc.mutate(), "");
        assert!(doc.node_count() <= limit);
        let mut doc = document(limit);
        let detached = doc.mutate().try_create_document_node();
        if let Ok(detached) = detached {
            DocumentHtmlParser::parse_into_document_node(
                &mut doc.mutate(),
                detached,
                &amplified_markup(),
                false,
            );
        }
        assert!(doc.node_count() <= limit);
    }
}

#[test]
fn exact_capacity_parsing_keeps_coalescing_text() {
    let doc = Rc::new(RefCell::new(document(6)));
    let mut parser = StreamingParser::new(doc.clone());
    parser.feed("<body>first");
    assert!(matches!(parser.run(), ParseStep::Done));
    assert!(!parser.is_finished());
    assert_eq!(doc.borrow().node_count(), 6);
    parser.feed(" second");
    parser.end_of_input();
    assert!(matches!(parser.run(), ParseStep::Done));
    assert!(parser.is_finished());
    let body = doc.borrow().query_selector("body").unwrap().unwrap();
    assert_eq!(
        doc.borrow().text_content_of(body).as_deref(),
        Some("first second")
    );
    assert_eq!(doc.borrow().node_count(), 6);
}

#[test]
fn document_write_exhaustion_discards_queued_network_tail() {
    let doc = Rc::new(RefCell::new(document(8)));
    let mut parser = StreamingParser::new(doc.clone());
    parser.feed("<script>pause</script><div id=tail></div><script>bad()</script>");
    parser.end_of_input();
    assert!(matches!(parser.run(), ParseStep::Script(_)));
    parser.write(&amplified_markup());
    assert!(matches!(parser.run_written(), ParseStep::Done));
    assert!(parser.is_finished());
    assert!(matches!(parser.run(), ParseStep::Done));
    assert!(doc.borrow().query_selector("#tail").unwrap().is_none());
    assert_eq!(doc.borrow().node_count(), 8);
}

#[test]
fn form_control_generated_children_share_parser_node_admission() {
    for limit in 1..32 {
        let mut document = document(limit);
        DocumentHtmlParser::parse_into_mutator(
            &mut document.mutate(),
            "<input type=button value=label><input type=submit value=send><input type=file>",
        );
        assert!(
            document.node_count() <= limit,
            "limit {limit}: {}",
            document.node_count()
        );
    }
}

#[test]
fn shadow_attachment_rejects_before_changing_the_host() {
    use blitz_dom::shadow::{AttachShadowError, ShadowRootInit};
    let mut doc = document(6);
    DocumentHtmlParser::parse_into_mutator(&mut doc.mutate(), "<div></div>");
    let host = doc.query_selector("div").unwrap().unwrap();
    assert_eq!(doc.node_count(), 6);
    assert_eq!(
        doc.mutate().attach_shadow(host, ShadowRootInit::default()),
        Err(AttachShadowError::NodeBudgetExceeded)
    );
    assert!(doc.shadow_root_of(host).is_none());
    assert_eq!(doc.node_count(), 6);
}

#[test]
fn template_shadow_and_adjacent_scratch_contexts_obey_budget() {
    use blitz_dom::shadow::ShadowRootInit;
    for spare in 0..12 {
        let mut doc = document(8 + spare);
        DocumentHtmlParser::parse_into_mutator(
            &mut doc.mutate(),
            "<div></div><template></template>",
        );
        let host = doc.query_selector("div").unwrap().unwrap();
        let template = doc.query_selector("template").unwrap().unwrap();
        let shadow = doc
            .mutate()
            .attach_shadow(host, ShadowRootInit::default())
            .unwrap();
        while doc.node_count() < doc.node_limit() - spare {
            doc.mutate().try_create_comment_node("fill").unwrap();
        }
        for target in [template, shadow] {
            doc.mutate().set_inner_html_detaching(target, "<b>new</b>");
            assert!(doc.node_count() <= doc.node_limit());
        }
        for position in ["beforebegin", "afterbegin", "beforeend", "afterend"] {
            let _ = doc
                .mutate()
                .insert_adjacent_html(host, position, "<i>new</i>");
            assert!(doc.node_count() <= doc.node_limit());
        }
    }
}

#[test]
fn layout_boxes_and_pseudo_text_share_the_node_ceiling() {
    for limit in 10..24 {
        let mut doc = document(limit);
        DocumentHtmlParser::parse_into_mutator(
            &mut doc.mutate(),
            "<style>div{display:flex}div::before{content:'before'}div::after{content:'after'}</style><div>text<span>inside</span></div>",
        );
        for _ in 0..3 {
            doc.resolve(0.0);
            assert!(
                doc.node_count() <= limit,
                "limit={limit}, count={}",
                doc.node_count()
            );
        }
    }
}

#[test]
fn layout_reconstruction_at_capacity_releases_old_positioning_links() {
    let mut doc = document(64);
    DocumentHtmlParser::parse_into_mutator(
        &mut doc.mutate(),
        "<style>.generated::before{content:'before'}p{display:block}</style><div id=x><span id=a>one</span><p>middle</p><span id=b>two</span></div>",
    );
    doc.resolve(0.0);
    while doc.node_count() < doc.node_limit() {
        doc.mutate().try_create_comment_node("fill").unwrap();
    }
    let root = doc.query_selector("#x").unwrap().unwrap();
    doc.mutate()
        .set_attribute_by_name(root, "class", "generated")
        .unwrap();
    doc.resolve(0.0);
    assert!(doc.node_count() <= doc.node_limit());
    for (id, node) in doc.tree().iter() {
        if let Some(parent) = node.containing_block() {
            assert!(
                doc.get_node(parent).is_some(),
                "{id:?} retains freed layout parent {parent:?}"
            );
        }
        if matches!(
            node.data,
            blitz_dom::node::NodeData::Element(_)
                | blitz_dom::node::NodeData::AnonymousBlock(_)
                | blitz_dom::node::NodeData::Document(_)
        ) {
            let _ = node.absolute_position(0.0, 0.0);
        }
        let _ = node.inline_root_ancestor();
    }
}

#[test]
fn fragment_rollback_discards_unpublished_handles_and_pending_title_style_work() {
    for markup in [
        "<template><p>",
        "<title>new</title><style>b{color:red}</style><div><b><i>tail</i></b></div>",
    ] {
        for spare in 0..10 {
            let mut doc = document(64);
            DocumentHtmlParser::parse_into_mutator(&mut doc.mutate(), "<div id=x>old</div>");
            let target = doc.query_selector("#x").unwrap().unwrap();
            while doc.node_count() < 64 - spare {
                doc.mutate().try_create_comment_node("fill").unwrap();
            }
            let before = doc.node_count();
            for _ in 0..3 {
                let result = doc.mutate().try_set_inner_html_detaching(target, markup);
                if result.is_err() {
                    assert_eq!(doc.node_count(), before, "spare={spare}: {markup}");
                    assert_eq!(doc.text_content_of(target).as_deref(), Some("old"));
                } else {
                    break;
                }
            }
        }
    }
}

#[test]
fn style_fragment_commit_flushes_only_the_surviving_style_owner() {
    let mut doc = document(64);
    DocumentHtmlParser::parse_into_mutator(&mut doc.mutate(), "<style id=s></style><b>text</b>");
    let style = doc.query_selector("#s").unwrap().unwrap();
    doc.mutate()
        .try_set_inner_html_detaching(style, "b{color:red}")
        .unwrap();
    assert_eq!(doc.text_content_of(style).as_deref(), Some("b{color:red}"));
    assert!(doc.retain_stylesheet(style).is_some());
    doc.resolve(0.0);
}
