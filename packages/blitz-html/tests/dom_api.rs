//! DOM Standard node operations in `blitz_dom::dom_api`.

mod common;

use blitz_dom::dom_api::{DomError, node_type};
use common::{parse, q};

const PAGE: &str = "<!DOCTYPE html><html><head></head><body><div id=a><p id=p1>one</p><p id=p2>two</p></div></body></html>";

#[test]
fn node_type_and_name() {
    let doc = parse(PAGE);
    let root = doc.root_node().id;
    assert_eq!(doc.node_type(root), node_type::DOCUMENT);
    assert_eq!(doc.node_name(root), "#document");
    assert_eq!(doc.node_name(q(&doc, "#a")), "DIV");
    let text = doc.get_node(q(&doc, "#p1")).unwrap().children[0];
    assert_eq!(doc.node_type(text), node_type::TEXT);
    assert_eq!(doc.node_name(text), "#text");
}

#[test]
fn text_content_get_and_set() {
    let mut doc = parse(PAGE);
    let a = q(&doc, "#a");
    assert_eq!(doc.text_content_of(a).as_deref(), Some("onetwo"));
    assert_eq!(doc.text_content_of(doc.root_node().id), None);

    let old_child = q(&doc, "#p1");
    doc.mutate().set_text_content(a, "replaced");
    assert_eq!(doc.text_content_of(a).as_deref(), Some("replaced"));
    // Removed children are detached, not dropped.
    let old = doc.get_node(old_child).expect("old child still valid");
    assert!(old.parent.is_none());
}

#[test]
fn pre_insert_enforces_hierarchy_rules() {
    let mut doc = parse(PAGE);
    let a = q(&doc, "#a");
    let body = q(&doc, "body");
    let root = doc.root_node().id;
    let mut m = doc.mutate();
    // An ancestor cannot be inserted into its descendant.
    assert_eq!(m.pre_insert(body, a, None), Err(DomError::HierarchyRequest));
    // A doctype cannot go into an element; text cannot go into a document.
    let doctype = m.create_doctype("html", "", "");
    assert_eq!(
        m.pre_insert(doctype, a, None),
        Err(DomError::HierarchyRequest)
    );
    let text = m.create_text_node("x");
    assert_eq!(
        m.pre_insert(text, root, None),
        Err(DomError::HierarchyRequest)
    );
    // A second document element is rejected.
    let extra = m.create_element(
        blitz_dom::QualName::new(None, blitz_dom::ns!(html), "div".into()),
        vec![],
    );
    assert_eq!(
        m.pre_insert(extra, root, None),
        Err(DomError::HierarchyRequest)
    );
    // The reference child must be a child of the parent.
    assert_eq!(m.pre_insert(text, a, Some(body)), Err(DomError::NotFound));
}

#[test]
fn inserting_an_element_before_the_doctype_preserves_document_order() {
    let mut doc = parse(PAGE);
    let root = doc.root_node().id;
    let doctype = doc.root_node().children[0];
    let element = q(&doc, "html");
    let mut m = doc.mutate();
    m.remove_child(root, element).unwrap();
    assert_eq!(
        m.pre_insert(element, root, Some(doctype)),
        Err(DomError::HierarchyRequest)
    );
    let fragment = m.create_document_fragment();
    m.pre_insert(element, fragment, None).unwrap();
    assert_eq!(
        m.pre_insert(fragment, root, Some(doctype)),
        Err(DomError::HierarchyRequest)
    );
    // Replacement excludes the doctype that will be removed.
    assert_eq!(m.replace_child(root, fragment, doctype), Ok(doctype));
}

#[test]
fn foreign_fragment_validity_uses_source_children_and_destination_order() {
    let mut source = parse(PAGE);
    let mut target = parse(PAGE);
    let root = target.root_node().id;
    let doctype = target.root_node().children[0];
    let target_element = q(&target, "html");
    target.mutate().remove_child(root, target_element).unwrap();
    let element = q(&source, "#a");
    let mut m = source.mutate();
    let fragment = m.create_document_fragment();
    m.pre_insert(element, fragment, None).unwrap();
    drop(m);
    assert_eq!(
        target.check_pre_insert_from(&source, fragment, root, Some(doctype)),
        Err(DomError::HierarchyRequest)
    );
    assert_eq!(
        target.check_pre_insert_from(&source, fragment, root, None),
        Ok(())
    );
    assert_eq!(source.get_node(element).unwrap().parent, Some(fragment));
}

#[test]
fn foreign_replacement_excludes_the_outgoing_document_element() {
    let mut source = parse(PAGE);
    let target = parse(PAGE);
    let root = target.root_node().id;
    let outgoing = q(&target, "html");
    let incoming = q(&source, "#a");
    let fragment = source.mutate().create_document_fragment();
    source.mutate().append_children(fragment, &[incoming]);
    assert_eq!(
        target.check_replace_child_from(&source, root, fragment, outgoing),
        Ok(())
    );
    let text = source.mutate().create_text_node("invalid");
    source.mutate().append_children(fragment, &[text]);
    assert_eq!(
        target.check_replace_child_from(&source, root, fragment, outgoing),
        Err(DomError::HierarchyRequest)
    );
    assert_eq!(
        source.get_node(fragment).unwrap().children.to_vec(),
        vec![incoming, text]
    );
    assert_eq!(target.get_node(outgoing).unwrap().parent, Some(root));
}

#[test]
fn fragments_insert_their_children() {
    let mut doc = parse(PAGE);
    let a = q(&doc, "#a");
    let p2 = q(&doc, "#p2");
    let mut m = doc.mutate();
    let fragment = m.create_document_fragment();
    let x = m.create_text_node("x");
    let y = m.create_text_node("y");
    m.pre_insert(x, fragment, None).unwrap();
    m.pre_insert(y, fragment, None).unwrap();
    m.pre_insert(fragment, a, Some(p2)).unwrap();
    drop(m);
    assert!(doc.get_node(fragment).unwrap().children.is_empty());
    assert_eq!(doc.text_content_of(a).as_deref(), Some("onexytwo"));
}

#[test]
fn replace_and_remove_child() {
    let mut doc = parse(PAGE);
    let a = q(&doc, "#a");
    let (p1, p2) = (q(&doc, "#p1"), q(&doc, "#p2"));
    let mut m = doc.mutate();
    let text = m.create_text_node("new");
    assert_eq!(m.replace_child(a, text, p1), Ok(p1));
    assert_eq!(m.remove_child(a, p1), Err(DomError::NotFound));
    assert_eq!(m.remove_child(a, p2), Ok(p2));
    drop(m);
    assert_eq!(doc.text_content_of(a).as_deref(), Some("new"));
}

#[test]
fn fragment_replacement_records_both_parents_once() {
    use blitz_dom::mutations::MutationRecord::ChildList;
    let mut doc = parse(PAGE);
    let parent = q(&doc, "#a");
    let old = q(&doc, "#p1");
    let next = q(&doc, "#p2");
    let mut m = doc.mutate();
    let fragment = m.create_document_fragment();
    let x = m.create_text_node("x");
    let y = m.create_text_node("y");
    m.append_children(fragment, &[x, y]);
    drop(m);
    doc.set_mutation_recording(true);
    assert_eq!(doc.mutate().replace_child(parent, fragment, old), Ok(old));
    assert_eq!(
        doc.take_mutation_records(),
        vec![
            ChildList {
                target: fragment,
                removed: vec![x, y],
                added: vec![],
                previous_sibling: None,
                next_sibling: None
            },
            ChildList {
                target: parent,
                removed: vec![old],
                added: vec![x, y],
                previous_sibling: None,
                next_sibling: Some(next)
            },
        ]
    );
}

#[test]
fn replacing_with_a_sibling_or_itself_records_adoption_first() {
    use blitz_dom::mutations::MutationRecord::ChildList;
    let mut doc = parse(PAGE);
    let parent = q(&doc, "#a");
    let old = q(&doc, "#p1");
    let next = q(&doc, "#p2");
    doc.set_mutation_recording(true);
    let record = |removed, added, previous_sibling| ChildList {
        target: parent,
        removed,
        added,
        previous_sibling,
        next_sibling: None,
    };
    doc.mutate().replace_child(parent, next, old).unwrap();
    assert_eq!(
        doc.take_mutation_records(),
        vec![
            record(vec![next], vec![], Some(old)),
            record(vec![old], vec![next], None)
        ]
    );
    doc.mutate().replace_child(parent, next, next).unwrap();
    assert_eq!(
        doc.take_mutation_records(),
        vec![
            record(vec![next], vec![], None),
            record(vec![], vec![next], None)
        ]
    );
}

#[test]
fn replacing_with_previous_sibling_records_the_remaining_neighbors() {
    use blitz_dom::mutations::MutationRecord::ChildList;
    let mut doc = parse("<body><i></i><b></b><p></p><a></a></body>");
    let parent = q(&doc, "body");
    let before = q(&doc, "i");
    let incoming = q(&doc, "b");
    let outgoing = q(&doc, "p");
    let after = q(&doc, "a");
    doc.set_mutation_recording(true);
    doc.mutate()
        .replace_child(parent, incoming, outgoing)
        .unwrap();
    assert_eq!(
        doc.take_mutation_records(),
        vec![
            ChildList {
                target: parent,
                removed: vec![incoming],
                added: vec![],
                previous_sibling: Some(before),
                next_sibling: Some(outgoing)
            },
            ChildList {
                target: parent,
                removed: vec![outgoing],
                added: vec![incoming],
                previous_sibling: Some(before),
                next_sibling: Some(after)
            },
        ]
    );
}

#[test]
fn clone_node_shallow_and_deep_including_template_contents() {
    let mut doc = parse("<div id=a title=t><b>x</b></div><template id=t><i>in</i></template>");
    let (a, t) = (q(&doc, "#a"), q(&doc, "#t"));
    let mut m = doc.mutate();
    let shallow = m.clone_node(a, false);
    let deep = m.clone_node(a, true);
    let template = m.clone_node(t, true);
    drop(m);
    assert!(doc.get_node(shallow).unwrap().children.is_empty());
    assert_eq!(
        doc.get_node(deep).unwrap().outer_html(),
        r#"<div id="a" title="t"><b>x</b></div>"#
    );
    assert_eq!(doc.get_node(template).unwrap().inner_html(), "<i>in</i>");
    assert!(doc.get_node(deep).unwrap().parent.is_none());
}

#[test]
fn attributes_by_qualified_name() {
    let mut doc = parse(PAGE);
    let a = q(&doc, "#a");
    let mut m = doc.mutate();
    // HTML elements lowercase names.
    m.set_attribute_by_name(a, "Data-X", "1").unwrap();
    assert_eq!(
        m.set_attribute_by_name(a, "bad name", "1"),
        Err(DomError::InvalidCharacter)
    );
    assert_eq!(m.toggle_attribute(a, "hidden", None), Ok(true));
    assert_eq!(m.toggle_attribute(a, "hidden", Some(true)), Ok(true));
    assert_eq!(m.toggle_attribute(a, "hidden", None), Ok(false));
    drop(m);
    assert_eq!(
        doc.attribute_by_name(a, "DATA-x")
            .map(|a| a.value.as_str_lossy()),
        Some("1")
    );
    assert_eq!(doc.attribute_names(a), vec!["id", "data-x"]);
    doc.mutate().remove_attribute_by_name(a, "data-x");
    assert!(doc.attribute_by_name(a, "data-x").is_none());
}

#[test]
fn inner_html_setter_detaches_old_children() {
    let mut doc = parse(PAGE);
    let a = q(&doc, "#a");
    let p1 = q(&doc, "#p1");
    doc.mutate().set_inner_html_detaching(a, "<span>new</span>");
    assert_eq!(doc.get_node(a).unwrap().inner_html(), "<span>new</span>");
    assert!(doc.get_node(p1).is_some(), "old children stay valid");
}

#[test]
fn dom_generation_changes_after_mutation() {
    let mut doc = parse(PAGE);
    let before = doc.dom_generation();
    let a = q(&doc, "#a");
    doc.mutate().set_text_content(a, "x");
    assert!(doc.dom_generation() > before);
}

#[test]
fn inner_html_setter_on_a_template_replaces_its_contents() {
    // Parsed and script-created templates alike: the markup parses in the
    // template's context into its contents, and children script gave the
    // template element itself stay.
    let mut doc = parse("<template id=t><i>old</i></template><div id=a></div>");
    let (t, a) = (q(&doc, "#t"), q(&doc, "#a"));
    let mut m = doc.mutate();
    let own = m.create_comment_node("own");
    m.append_children(t, &[own]);
    m.set_inner_html_detaching(t, "<tr><td>row</td></tr>text");
    let created = m.create_element(html_name("template"), Vec::new());
    m.append_children(a, &[created]);
    m.set_inner_html_detaching(created, "<b>new</b>");
    drop(m);
    assert_eq!(
        doc.get_node(t).unwrap().inner_html(),
        "<tr><td>row</td></tr>text"
    );
    assert_eq!(doc.get_node(t).unwrap().children.as_slice(), &[own]);
    assert_eq!(doc.get_node(created).unwrap().inner_html(), "<b>new</b>");
    assert!(doc.get_node(created).unwrap().children.is_empty());
}

#[test]
fn deep_trees_serialize_clone_and_collect_text_without_recursion() {
    // Scripts can nest nodes far deeper than the parser would. On a small
    // stack, recursing once per level would overflow long before this
    // depth.
    const DEPTH: usize = 20_000;
    let result = on_small_stack(|| {
        let mut doc = parse(PAGE);
        let a = q(&doc, "#a");
        let mut m = doc.mutate();
        let chain = deep_chain(&mut m, DEPTH, "leaf");
        m.append_children(a, &[chain]);
        let copy = m.clone_node(a, true);
        drop(m);
        let html = doc.get_node(a).unwrap().inner_html();
        let outer = doc.get_node(copy).unwrap().outer_html();
        let text = doc.text_content_of(copy).unwrap();
        (html.len(), outer.len(), text)
    });
    let inner_len =
        "<p id=\"p1\">one</p><p id=\"p2\">two</p>".len() + DEPTH * "<i></i>".len() + "leaf".len();
    assert_eq!(result.0, inner_len);
    assert_eq!(result.1, inner_len + "<div id=\"a\"></div>".len());
    assert_eq!(result.2, "onetwoleaf");
}

#[test]
fn connecting_a_deep_subtree_takes_linear_time() {
    // Connecting must not walk to the tree root for each node: that costs
    // time in the chain's depth squared, seconds at this depth even
    // optimized. The bound is loose enough for unoptimized builds.
    const DEPTH: usize = 40_000;
    let mut doc = parse(PAGE);
    let a = q(&doc, "#a");
    let mut m = doc.mutate();
    let top = deep_chain(&mut m, DEPTH, "leaf");
    let start = std::time::Instant::now();
    m.append_children(a, &[top]);
    drop(m);
    let elapsed = start.elapsed();
    assert!(doc.is_connected(top));
    assert!(
        elapsed < std::time::Duration::from_secs(2),
        "connecting took {elapsed:?}"
    );
}

#[test]
fn insert_adjacent_html_on_the_root_element_parses_as_body_content() {
    // HTML § 8.5, insertAdjacentHTML step 2: an `html` context element
    // parses as `body`, so no `head` or `body` is invented.
    let mut doc = parse(PAGE);
    let html = q(&doc, "html");
    let mut m = doc.mutate();
    m.insert_adjacent_html(html, "afterbegin", "<p>x</p>")
        .unwrap();
    m.insert_adjacent_html(html, "beforeend", "<i>y</i>")
        .unwrap();
    drop(m);
    let names: Vec<String> = doc
        .get_node(html)
        .unwrap()
        .children
        .iter()
        .map(|&child| doc.node_name(child))
        .collect();
    assert_eq!(names, ["P", "HEAD", "BODY", "I"]);
}

#[test]
fn deep_text_in_title_and_style_elements_collects_without_recursion() {
    // A document reads a `<title>`'s and a `<style>`'s text when their
    // children change; on a small stack, recursing once per level would
    // overflow long before this depth.
    const DEPTH: usize = 20_000;
    let texts = on_small_stack(|| {
        let mut doc = parse(PAGE);
        let head = q(&doc, "head");
        let mut m = doc.mutate();
        let mut elements = Vec::new();
        for tag in ["title", "style"] {
            let element = m.create_element(html_name(tag), Vec::new());
            m.append_children(head, &[element]);
            let chain = deep_chain(&mut m, DEPTH, "leaf");
            m.append_children(element, &[chain]);
            elements.push(element);
        }
        drop(m);
        elements
            .iter()
            .map(|&id| doc.get_node(id).unwrap().text_content())
            .collect::<Vec<_>>()
    });
    assert_eq!(texts, ["leaf", "leaf"]);
}

#[test]
fn emptying_and_moving_many_children_takes_linear_time() {
    // Detaching must not search the child list once per child: that costs
    // time in its length squared, seconds at this size even optimized.
    // The bound is loose enough for unoptimized builds.
    const CHILDREN: usize = 100_000;
    let mut doc = parse(PAGE);
    let a = q(&doc, "#a");
    let mut m = doc.mutate();
    let children: Vec<_> = (0..CHILDREN).map(|_| m.create_text_node("x")).collect();
    m.append_children(a, &children);
    let start = std::time::Instant::now();
    m.replace_all(a, None);
    let emptied = start.elapsed();
    let fragment = m.create_document_fragment();
    m.append_children(fragment, &children);
    let start = std::time::Instant::now();
    m.pre_insert(fragment, a, None).unwrap();
    let moved = start.elapsed();
    drop(m);
    assert_eq!(doc.get_node(a).unwrap().children.len(), CHILDREN);
    assert!(doc.get_node(fragment).unwrap().children.is_empty());
    let bound = std::time::Duration::from_secs(2);
    assert!(
        emptied < bound && moved < bound,
        "emptied in {emptied:?}, moved in {moved:?}"
    );
}

#[test]
fn many_elements_sharing_an_id_connect_in_linear_time() {
    // The id map must not search every element with the same id on each
    // insertion: seconds at this count even optimized. The first in tree
    // order still wins.
    const COUNT: usize = 100_000;
    let mut doc = parse(PAGE);
    let a = q(&doc, "#a");
    let mut m = doc.mutate();
    let attrs = || {
        vec![blitz_dom::node::Attribute {
            name: blitz_dom::QualName::new(
                None,
                blitz_dom::ns!(),
                blitz_dom::LocalName::from("id"),
            ),
            value: "dup".into(),
        }]
    };
    let elements: Vec<_> = (0..COUNT)
        .map(|_| m.create_element(html_name("i"), attrs()))
        .collect();
    let start = std::time::Instant::now();
    m.append_children(a, &elements);
    drop(m);
    let elapsed = start.elapsed();
    assert_eq!(doc.get_element_by_id("dup"), Some(elements[0]));
    assert!(
        elapsed < std::time::Duration::from_secs(2),
        "connecting took {elapsed:?}"
    );
}

fn html_name(local: &str) -> blitz_dom::QualName {
    blitz_dom::QualName::new(
        None,
        blitz_dom::ns!(html),
        blitz_dom::LocalName::from(local),
    )
}

/// A detached chain of `depth` nested `<i>` elements with a `leaf` text
/// node at the bottom, built bottom-up; returns the top.
fn deep_chain(
    m: &mut blitz_dom::DocumentMutator<'_>,
    depth: usize,
    leaf: &str,
) -> blitz_dom::NodeId {
    let text = m.create_text_node(leaf);
    let mut top = m.create_element(html_name("i"), Vec::new());
    m.append_children(top, &[text]);
    for _ in 1..depth {
        let parent = m.create_element(html_name("i"), Vec::new());
        m.append_children(parent, &[top]);
        top = parent;
    }
    top
}

/// Run `test` on a 256 KiB stack, where recursing once per tree level
/// overflows at a few thousand levels.
fn on_small_stack<T: Send + 'static>(test: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(256 << 10)
        .spawn(test)
        .unwrap()
        .join()
        .unwrap()
}

#[test]
fn fragment_drain_announces_original_children_before_reentrant_edits() {
    use blitz_dom::mutations::MutationRecord;
    let mut doc = parse("<div id=a><b></b><b slot=n></b></div><slot></slot><slot name=n></slot>");
    let host = q(&doc, "#a");
    let root = doc
        .mutate()
        .attach_shadow(host, Default::default())
        .unwrap();
    let original = q(&doc, "slot:not([name])");
    let extra = q(&doc, "slot[name]");
    doc.mutate().remove_node(extra);
    doc.mutate().pre_insert(original, root, None).unwrap();
    assert_eq!(doc.take_slot_changes(), vec![original]);
    doc.set_mutation_recording(true);
    doc.mutate().announce_fragment_drain(root).unwrap();
    assert!(doc.take_slot_changes().is_empty());
    assert_eq!(doc.get_node(original).unwrap().parent, Some(root));
    assert_eq!(
        doc.take_mutation_records(),
        vec![MutationRecord::ChildList {
            target: root,
            added: vec![],
            removed: vec![original],
            previous_sibling: None,
            next_sibling: None,
        }]
    );
    doc.mutate().pre_insert(extra, root, None).unwrap();
    assert_eq!(doc.take_slot_changes(), vec![extra]);
    doc.mutate().finish_fragment_drain(root).unwrap();
    let changes = doc.take_slot_changes();
    assert_eq!(changes.len(), 2);
    assert!(changes.contains(&extra) && changes.contains(&original));
    assert!(doc.get_node(root).unwrap().children.is_empty());
    assert_eq!(doc.get_node(extra).unwrap().parent, None);
    assert_eq!(doc.get_node(original).unwrap().parent, None);
    assert!(matches!(doc.take_mutation_records().as_slice(),
        [MutationRecord::ChildList { added, removed, .. }] if added == &[extra] && removed.is_empty()));
}
