//! Staged admission must precede visible replacement and release every scratch node.
mod common;
use blitz_dom::{BaseDocument, DocumentConfig, NodeBudgetExceeded};
use blitz_html::{DocumentHtmlParser, HtmlProvider};
use common::{parse, q};
use std::{cell::Cell, sync::Arc};
#[derive(Debug, PartialEq)]
enum Rejected {
    Quota,
    Policy,
}
impl From<NodeBudgetExceeded> for Rejected {
    fn from(_: NodeBudgetExceeded) -> Self {
        Self::Quota
    }
}

#[test]
fn rejection_preserves_old_children_and_releases_nested_template_allocations() {
    let mut doc = parse("<div id=a><b>old</b></div>");
    let a = q(&doc, "#a");
    let old = doc.get_node(a).unwrap().children[0];
    let before = doc.node_count();
    let result = doc.mutate().try_set_inner_html_admitted(
        a,
        "<template><p>new</p></template>",
        |document, scratch| {
            let scratch = scratch.unwrap();
            assert_eq!(document.get_node(old).unwrap().parent, Some(a));
            assert!(!document.get_node(scratch).unwrap().children.is_empty());
            Err::<(), _>(Rejected::Policy)
        },
    );
    assert_eq!(result, Err(Rejected::Policy));
    assert_eq!(doc.node_count(), before);
    assert_eq!(doc.get_node(a).unwrap().children.as_slice(), &[old]);
    assert_eq!(doc.get_node(old).unwrap().outer_html(), "<b>old</b>");
}

#[test]
fn accepted_callback_result_survives_commit_and_old_nodes_are_detached() {
    let mut doc = parse("<table><tbody id=a><tr><td>old</td></tr></tbody></table>");
    let a = q(&doc, "#a");
    let old = doc.get_node(a).unwrap().children[0];
    let value = doc
        .mutate()
        .try_set_inner_html_admitted(a, "<tr><td>new</td></tr>", |document, scratch| {
            let first = document.get_node(scratch.unwrap()).unwrap().children[0];
            assert_eq!(document.node_name(first), "TR");
            Ok::<_, Rejected>(42)
        })
        .unwrap();
    assert_eq!(value, 42);
    assert_eq!(doc.get_node(old).unwrap().parent, None);
    assert_eq!(
        doc.get_node(a).unwrap().inner_html(),
        "<tr><td>new</td></tr>"
    );
}

#[test]
fn clearing_still_requires_admission_before_detaching() {
    let mut doc = parse("<div id=a>old</div>");
    let a = q(&doc, "#a");
    let old = doc.get_node(a).unwrap().children[0];
    assert_eq!(
        doc.mutate()
            .try_set_inner_html_admitted(a, "", |_, scratch| {
                assert!(scratch.is_none());
                Err::<(), _>(Rejected::Policy)
            }),
        Err(Rejected::Policy)
    );
    assert_eq!(doc.get_node(old).unwrap().parent, Some(a));
    doc.mutate()
        .try_set_inner_html_admitted(a, "", |_, scratch| {
            assert!(scratch.is_none());
            Ok::<_, Rejected>(())
        })
        .unwrap();
    assert_eq!(doc.get_node(old).unwrap().parent, None);
}

#[test]
fn template_rejection_preserves_contents_and_ordinary_children() {
    let mut doc = parse("<template id=a><b>old</b></template>");
    let a = q(&doc, "#a");
    let contents = doc
        .get_node(a)
        .unwrap()
        .element_data()
        .unwrap()
        .template_contents
        .unwrap();
    let old = doc.get_node(contents).unwrap().children[0];
    let own = doc.mutate().try_create_comment_node("own").unwrap();
    doc.mutate().append_children(a, &[own]);
    let before = doc.node_count();
    assert_eq!(
        doc.mutate()
            .try_set_inner_html_admitted(a, "<i>new</i>", |_, _| Err::<(), _>(Rejected::Policy)),
        Err(Rejected::Policy)
    );
    assert_eq!(doc.node_count(), before);
    assert_eq!(doc.get_node(old).unwrap().parent, Some(contents));
    assert_eq!(doc.get_node(own).unwrap().parent, Some(a));
}

#[test]
fn node_quota_failure_cleans_up_before_admission_callback() {
    let mut doc = BaseDocument::new(DocumentConfig {
        node_limit: Some(32),
        html_parser_provider: Some(Arc::new(HtmlProvider)),
        ..Default::default()
    });
    DocumentHtmlParser::parse_into_mutator(&mut doc.mutate(), "<div id=a>old</div>");
    let a = q(&doc, "#a");
    let before = doc.node_count();
    let called = Cell::new(false);
    let result = doc
        .mutate()
        .try_set_inner_html_admitted(a, &"<b>new</b>".repeat(32), |_, _| {
            called.set(true);
            Ok::<_, Rejected>(())
        });
    assert_eq!(result, Err(Rejected::Quota));
    assert!(!called.get());
    assert_eq!(doc.node_count(), before);
    assert_eq!(doc.get_node(a).unwrap().inner_html(), "old");
}

#[test]
fn separately_adopted_template_contents_relink_on_return() {
    use blitz_dom::foreign_import::ImportMode;
    let mut source = parse("<template id=t>original</template>");
    let mut other = parse("<body></body>");
    let host = q(&source, "#t");
    let contents = source.mutate().template_contents(host);
    let owner = other.root_node().id;
    let moved = other
        .mutate()
        .try_import_foreign(&source, contents, owner, ImportMode::Transfer)
        .unwrap()
        .root();
    assert_eq!(
        source.mutate().unlink_template_contents(contents),
        Some(host)
    );
    assert_eq!(source.host_including_parent(contents), None);
    let child = other.get_node(moved).unwrap().children[0];
    other
        .mutate()
        .try_set_adopted_template_html(moved, "<b>changed</b>", false, |_, _| {
            Ok::<(), NodeBudgetExceeded>(())
        })
        .unwrap();
    assert_eq!(
        other.get_node(moved).unwrap().inner_html(),
        "<b>changed</b>"
    );
    assert!(other.get_node(child).unwrap().parent.is_none());
    assert_eq!(
        other
            .mutate()
            .try_set_adopted_template_html(moved, "", false, |_, _| Err::<(), _>(
                NodeBudgetExceeded
            )),
        Err(NodeBudgetExceeded)
    );
    assert_eq!(
        other.get_node(moved).unwrap().inner_html(),
        "<b>changed</b>"
    );

    let owner = source.root_node().id;
    let returned = source
        .mutate()
        .try_import_foreign(&other, moved, owner, ImportMode::Transfer)
        .unwrap()
        .root();
    source
        .mutate()
        .relink_template_contents(host, returned)
        .unwrap();
    assert_eq!(source.node_document(returned), owner);
    assert_eq!(source.host_including_parent(returned), Some(host));
    assert_eq!(
        source.get_node(host).unwrap().inner_html(),
        "<b>changed</b>"
    );
    source.reclaim_detached_nodes(&[host]);
    assert!(source.get_node(contents).is_none());
    assert!(source.get_node(returned).is_some());
}
