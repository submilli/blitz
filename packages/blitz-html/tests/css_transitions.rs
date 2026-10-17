//! Transition completion is observable once, after the animation timeline advances.
mod common;
use common::{parse, q};

#[test]
fn transition_end_has_duration_and_geometry_reads_do_not_rewind_time() {
    let mut doc =
        parse("<style>#x { opacity:0; transition:opacity .2s linear .1s }</style><div id=x></div>");
    doc.resolve(0.0);
    let node = q(&doc, "#x");
    doc.mutate()
        .set_attribute_by_name(node, "style", "opacity:1")
        .unwrap();
    doc.resolve(0.0);
    assert!(doc.has_pending_css_animation_work());
    doc.resolve(0.15);
    assert!(doc.take_transition_ends().is_empty());
    doc.resolve(0.31);
    assert!(doc.has_pending_css_animation_work());
    let events = doc.take_transition_ends();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].target.id(), node);
    assert_eq!(events[0].property_name, "opacity");
    assert_eq!(events[0].pseudo_element, "");
    assert!((events[0].elapsed_time - 0.2).abs() < 1e-6);
    doc.resolve(0.0);
    doc.resolve(1.0);
    assert!(doc.take_transition_ends().is_empty());
    assert!(!doc.has_pending_css_animation_work());
    assert_eq!(doc.resolved_style_value(node, "opacity"), "1");
}

#[test]
fn removed_and_cancelled_transitions_do_not_complete() {
    for remove in [false, true] {
        let mut doc =
            parse("<style>#x { opacity:0; transition:opacity .2s }</style><div id=x></div>");
        doc.resolve(0.0);
        let node = q(&doc, "#x");
        doc.mutate()
            .set_attribute_by_name(node, "style", "opacity:1")
            .unwrap();
        doc.resolve(0.0);
        if remove {
            doc.mutate().remove_node(node);
        } else {
            doc.mutate()
                .set_attribute_by_name(node, "style", "opacity:0;transition:none")
                .unwrap();
        }
        doc.resolve(0.1);
        doc.resolve(1.0);
        assert!(doc.take_transition_ends().is_empty());
    }
}

#[test]
fn transition_queue_is_bounded_and_reports_overflow() {
    let mut doc = parse(&format!(
        "<style>.go div {{opacity:1}} div {{opacity:0;transition:opacity .1s}}</style><main id=root>{}</main>",
        "<div></div>".repeat(1030)
    ));
    doc.resolve(0.0);
    let root = q(&doc, "#root");
    doc.mutate()
        .set_attribute_by_name(root, "class", "go")
        .unwrap();
    doc.resolve(0.0);
    doc.resolve(1.0);
    assert_eq!(doc.take_transition_ends().len(), 1024);
    assert!(doc.take_transition_end_overflow());
    assert!(!doc.take_transition_end_overflow());
    doc.resolve(f64::NAN);
    doc.resolve(f64::INFINITY);
    assert!(doc.take_transition_ends().is_empty());
}

#[test]
fn completions_sort_by_deadline_tree_order_and_property() {
    let mut doc = parse(
        "<style>div{width:10px;opacity:0;transition:width .2s,opacity .2s}#c{transition:opacity .1s}</style><main id=root><div id=a></div><div id=b></div><div id=c></div></main>",
    );
    doc.resolve(0.0);
    for id in ["#a", "#b", "#c"] {
        let n = q(&doc, id);
        doc.mutate()
            .set_attribute_by_name(n, "style", "width:20px;opacity:1")
            .unwrap();
    }
    doc.resolve(0.0);
    doc.resolve(1.0);
    let events = doc.take_transition_ends();
    let actual: Vec<_> = events
        .iter()
        .map(|e| (e.target.id(), e.property_name.as_str()))
        .collect();
    assert_eq!(
        actual,
        [
            (q(&doc, "#c"), "opacity"),
            (q(&doc, "#a"), "opacity"),
            (q(&doc, "#a"), "width"),
            (q(&doc, "#b"), "opacity"),
            (q(&doc, "#b"), "width")
        ]
    );
}

#[test]
fn simultaneous_completions_sort_by_style_generation_before_property() {
    let mut doc = parse(
        "<style>#x{width:10px;opacity:0;transition:width 1s,opacity .5s}</style><div id=x></div>",
    );
    doc.resolve(0.0);
    let node = q(&doc, "#x");
    doc.mutate()
        .set_attribute_by_name(node, "style", "width:20px")
        .unwrap();
    doc.resolve(0.0);
    doc.resolve(0.5);
    doc.mutate()
        .set_attribute_by_name(node, "style", "width:20px;opacity:1")
        .unwrap();
    doc.resolve(0.5);
    doc.resolve(1.1);
    assert_eq!(
        doc.take_transition_ends()
            .iter()
            .map(|e| e.property_name.as_str())
            .collect::<Vec<_>>(),
        ["width", "opacity"]
    );
}

#[test]
fn dropping_an_animating_node_removes_opaque_state_before_allocation() {
    let mut doc = parse("<style>#x { opacity:0; transition:opacity .2s }</style><div id=x></div>");
    doc.resolve(0.0);
    let node = q(&doc, "#x");
    doc.mutate()
        .set_attribute_by_name(node, "style", "opacity:1")
        .unwrap();
    doc.resolve(0.0);
    assert!(doc.has_pending_css_animation_work());
    doc.mutate().remove_and_drop_node(node);
    for _ in 0..128 {
        doc.mutate().create_text_node("reuse allocations");
    }
    doc.resolve(1.0);
    assert!(doc.take_transition_ends().is_empty());
    assert!(!doc.has_pending_css_animation_work());
}
