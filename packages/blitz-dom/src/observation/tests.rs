use crate::{BaseDocument, DomString, NodeId, ObservationBudget, ObservationLimit, QualName, ns};

fn element(doc: &mut BaseDocument, name: &str) -> NodeId {
    doc.mutate()
        .try_create_element(QualName::new(None, ns!(html), name.into()), vec![])
        .unwrap()
}

#[test]
fn bounded_text_matches_observations_and_preserves_redaction() {
    let mut doc = BaseDocument::new(Default::default());
    let root = element(&mut doc, "div");
    let first = doc.mutate().try_create_text_node("start😀").unwrap();
    let input = element(&mut doc, "input");
    doc.mutate()
        .set_attribute(input, QualName::new(None, ns!(), "type".into()), "PASSWORD");
    let textarea = element(&mut doc, "textarea");
    assert!(doc.mark_credential_field(textarea));
    let private = doc.mutate().try_create_text_node("private").unwrap();
    doc.mutate().append_children(textarea, &[private]);
    let last = doc
        .mutate()
        .try_create_text_node(DomString::from_utf16(vec![0xd800, 0x61]))
        .unwrap();
    doc.mutate()
        .append_children(root, &[first, input, textarea, last]);
    let node = doc.get_node(root).unwrap();
    let mut budget = ObservationBudget::new(100, 1000);
    assert_eq!(
        node.try_observation_text(&mut budget),
        Ok(node.observation_text())
    );
    assert!(
        !node
            .try_observation_text(&mut budget)
            .unwrap()
            .contains("private")
    );
}

#[test]
fn repeated_projections_share_a_sticky_budget_and_return_no_partial_text() {
    let mut doc = BaseDocument::new(Default::default());
    let text = doc.mutate().try_create_text_node("abc").unwrap();
    let node = doc.get_node(text).unwrap();
    let mut budget = ObservationBudget::new(10, 18);
    assert_eq!(node.try_observation_text(&mut budget), Ok("abc".into()));
    assert_eq!(node.try_observation_text(&mut budget), Ok("abc".into()));
    assert_eq!(budget.remaining(), (8, 0));
    assert_eq!(
        node.try_observation_text(&mut budget),
        Err(ObservationLimit)
    );
    let empty = element(&mut doc, "div");
    assert_eq!(
        doc.get_node(empty)
            .unwrap()
            .try_observation_text(&mut budget),
        Err(ObservationLimit)
    );
}

#[test]
fn oversized_utf16_is_rejected_before_projection_allocation() {
    let mut doc = BaseDocument::new(Default::default());
    let text = doc
        .mutate()
        .try_create_text_node(DomString::from_utf16(vec![0xd800; 1_000_000]))
        .unwrap();
    let peak = crate::test_allocator::peak_held(|| {
        let mut budget = ObservationBudget::new(10, 16);
        assert_eq!(
            doc.get_node(text)
                .unwrap()
                .try_observation_text(&mut budget),
            Err(ObservationLimit)
        );
    });
    assert!(peak < 64 * 1024, "rejected text allocated {peak} bytes");
}

#[test]
fn wide_subtrees_do_not_allocate_unadmitted_sibling_lists() {
    let mut doc = BaseDocument::new(Default::default());
    let root = element(&mut doc, "div");
    let children: Vec<_> = (0..20_000).map(|_| element(&mut doc, "span")).collect();
    doc.mutate().append_children(root, &children);
    let peak = crate::test_allocator::peak_held(|| {
        let mut budget = ObservationBudget::new(1, 100);
        assert_eq!(
            doc.get_node(root)
                .unwrap()
                .try_observation_text(&mut budget),
            Err(ObservationLimit)
        );
    });
    assert!(
        peak < 64 * 1024,
        "unvisited siblings allocated {peak} bytes"
    );
}

#[test]
fn password_classification_does_not_project_large_utf16_attributes() {
    let mut doc = BaseDocument::new(Default::default());
    let input = element(&mut doc, "input");
    doc.mutate().set_attribute(
        input,
        QualName::new(None, ns!(), "type".into()),
        DomString::from_utf16(vec![0xd800; 1_000_000]),
    );
    let peak = crate::test_allocator::peak_held(|| {
        assert!(!doc.get_node(input).unwrap().masks_observation());
    });
    assert_eq!(peak, 0, "password classification projected the attribute");
}

#[test]
fn deep_text_projection_is_iterative_and_attribute_work_is_charged() {
    let mut doc = BaseDocument::new(Default::default());
    let root = element(&mut doc, "div");
    let mut parent = root;
    for _ in 0..1000 {
        let child = element(&mut doc, "div");
        doc.mutate().append_children(parent, &[child]);
        parent = child;
    }
    let text = doc.mutate().try_create_text_node("end").unwrap();
    doc.mutate().append_children(parent, &[text]);
    let mut budget = ObservationBudget::new(1002, 9);
    assert_eq!(
        doc.get_node(root)
            .unwrap()
            .try_observation_text(&mut budget),
        Ok("end".into())
    );
    let input = element(&mut doc, "input");
    doc.mutate()
        .set_attribute(input, QualName::new(None, ns!(), "id".into()), "target");
    let mut budget = ObservationBudget::new(1, 100);
    assert_eq!(
        doc.get_node(input)
            .unwrap()
            .try_observation_text(&mut budget),
        Err(ObservationLimit)
    );
}

#[test]
fn nonmatching_attribute_queries_admit_before_lossy_projection() {
    let mut doc = BaseDocument::new(Default::default());
    let input = element(&mut doc, "div");
    let root = doc.root_node().id;
    doc.mutate().append_children(root, &[input]);
    doc.mutate().set_attribute(
        input,
        QualName::new(None, ns!(), "data-large".into()),
        DomString::from_utf16(vec![0xd800; 1_000_000]),
    );
    let peak = crate::test_allocator::peak_held(|| {
        let mut budget = ObservationBudget::new(100, 1024);
        assert_eq!(
            doc.try_observation_query("[data-large='missing']", &mut budget),
            Err(crate::ObservationQueryError::Limit)
        );
    });
    assert!(
        peak < 64 * 1024,
        "nonmatching selector projected {peak} bytes"
    );
    let mut budget = ObservationBudget::new(100, 4_000_000);
    assert_eq!(
        doc.try_observation_query("[data-large='missing']", &mut budget),
        Ok(vec![])
    );
}

#[test]
fn observation_queries_preserve_matching_and_charge_shared_payload() {
    let mut doc = BaseDocument::new(Default::default());
    let input = element(&mut doc, "div");
    let root = doc.root_node().id;
    doc.mutate().append_children(root, &[input]);
    doc.mutate()
        .set_attribute(input, QualName::new(None, ns!(), "class".into()), "item");
    let text = doc.mutate().try_create_text_node("visible").unwrap();
    doc.mutate().append_children(input, &[text]);
    let mut budget = ObservationBudget::new(100, 100);
    assert_eq!(
        doc.try_observation_query("div.item", &mut budget),
        Ok(vec![input])
    );
    assert_eq!(
        doc.get_node(input)
            .unwrap()
            .try_observation_text(&mut budget),
        Ok("visible".into())
    );
    assert_eq!(budget.remaining(), (93, 59));
    assert_eq!(
        doc.try_observation_query("[", &mut budget),
        Err(crate::ObservationQueryError::InvalidSelector)
    );
}
