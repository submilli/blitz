use super::*;
use crate::{DocumentConfig, DocumentType, HtmlParserProvider, QualName, ns};
use std::sync::Arc;

struct Provider {
    fail: bool,
}
impl HtmlParserProvider for Provider {
    fn parse_inner_html<'a, 'b>(&self, _: &'a mut DocumentMutator<'b>, _: NodeId, _: &str) {}
    fn try_parse_into_document_node(
        &self,
        m: &mut DocumentMutator<'_>,
        document: NodeId,
        markup: &str,
        xml: bool,
    ) -> Result<(), NodeBudgetExceeded> {
        assert_eq!(
            xml,
            !m.doc.document_metadata(document).document_type.is_html()
        );
        let element = m.try_create_element(QualName::new(None, ns!(html), "div".into()), vec![])?;
        m.append_children(document, &[element]);
        let text = m.try_create_text_node(markup)?;
        m.append_children(element, &[text]);
        if self.fail {
            Err(NodeBudgetExceeded)
        } else {
            Ok(())
        }
    }
}
fn document(limit: usize, fail: bool) -> BaseDocument {
    BaseDocument::new(DocumentConfig {
        node_limit: Some(limit),
        html_parser_provider: Some(Arc::new(Provider { fail })),
        ..Default::default()
    })
}
#[test]
fn parsed_document_admission_observes_metadata_complete_tree_and_inert_ownership() {
    let mut doc = document(16, false);
    let metadata = DocumentMetadata {
        document_type: DocumentType::Svg,
        domain: "example.test".into(),
        url: Some("https://example.test/path".parse().unwrap()),
        ..Default::default()
    };
    let (id, count) = doc
        .mutate()
        .try_parse_document_admitted("literal", metadata, |dom, id| {
            assert_eq!(dom.document_metadata(id).domain, "example.test");
            assert_eq!(dom.document_url(id).as_str(), "https://example.test/path");
            let element = dom.get_node(id).unwrap().children[0];
            let text = dom.get_node(element).unwrap().children[0];
            assert_eq!(dom.node_document(text), id);
            assert_eq!(dom.text_content_of(text).unwrap(), "literal");
            assert!(!dom.get_node(text).unwrap().flags.is_in_document());
            Ok::<_, NodeBudgetExceeded>(dom.node_count())
        })
        .unwrap();
    assert_eq!(count, 6);
    assert_eq!(doc.node_count(), 6);
    assert_eq!(doc.node_document(id), id);
    assert!(doc.root_node().children.is_empty());
}
#[test]
fn rejected_parsed_document_releases_complete_tree_and_preserves_other_nodes() {
    let mut doc = document(16, false);
    let held = doc.mutate().try_create_text_node("held").unwrap();
    let before = doc.node_count();
    for _ in 0..4 {
        let result =
            doc.mutate()
                .try_parse_document_admitted("discard", Default::default(), |_, _| {
                    Err::<(), _>(NodeBudgetExceeded)
                });
        assert_eq!(result, Err(NodeBudgetExceeded));
        assert_eq!(doc.node_count(), before);
        assert_eq!(doc.character_data_dom(held).unwrap().as_str_lossy(), "held");
    }
}
#[test]
fn parser_failure_never_calls_admission_and_releases_the_document() {
    let mut doc = document(16, true);
    let before = doc.node_count();
    let result = doc.mutate().try_parse_document_admitted(
        "discard",
        Default::default(),
        |_, _| -> Result<(), NodeBudgetExceeded> {
            panic!("failed parse must not reach admission")
        },
    );
    assert_eq!(result, Err(NodeBudgetExceeded));
    assert_eq!(doc.node_count(), before);
}
#[test]
fn document_and_parser_node_exhaustion_are_atomic_at_each_boundary() {
    for limit in 2..6 {
        let mut doc = document(limit, false);
        let before = doc.node_count();
        let result = doc
            .mutate()
            .try_parse_document_admitted("x", Default::default(), |_, _| {
                Ok::<_, NodeBudgetExceeded>(())
            });
        assert_eq!(result, Err(NodeBudgetExceeded));
        assert_eq!(doc.node_count(), before);
    }
    let mut doc = document(6, false);
    doc.mutate()
        .try_parse_document_admitted("x", Default::default(), |_, _| {
            Ok::<_, NodeBudgetExceeded>(())
        })
        .unwrap();
    assert_eq!(doc.node_count(), 6);
}
#[test]
fn parse_and_rejection_preserve_existing_mutation_records_and_recording_state() {
    let mut doc = document(16, false);
    doc.set_mutation_recording(true);
    let active = doc.root_node().id;
    let node = doc.mutate().try_create_comment_node("existing").unwrap();
    doc.mutate().append_children(active, &[node]);
    let expected = doc.take_mutation_records();
    assert!(!expected.is_empty());
    doc.mutate().append_children(active, &[node]);
    let before = doc.mutation_log.as_ref().unwrap().clone();
    for reject in [false, true] {
        let result =
            doc.mutate()
                .try_parse_document_admitted("hidden", Default::default(), |_, _| {
                    if reject {
                        Err(NodeBudgetExceeded)
                    } else {
                        Ok(())
                    }
                });
        assert_eq!(result.is_err(), reject);
        assert!(doc.is_recording_mutations());
        assert_eq!(doc.mutation_log.as_ref().unwrap(), &before);
    }
}
