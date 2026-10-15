//! Isolate style traversal from DOM construction and box layout.
mod common;

use blitz_dom::{BaseDocument, QualName, local_name, ns};
use std::time::{Duration, Instant};

fn tree(count: usize, deep: bool) -> (BaseDocument, blitz_dom::NodeId) {
    let mut doc = common::parse("<!doctype html><html><head></head><body></body></html>");
    let body = common::q(&doc, "body");
    let mut parent = body;
    let mut last = body;
    let mut dom = doc.mutate();
    for _ in 0..count {
        let node = dom.create_element(QualName::new(None, ns!(html), local_name!("div")), vec![]);
        dom.append_children(parent, &[node]);
        last = node;
        if deep {
            parent = node;
        }
    }
    drop(dom);
    (doc, last)
}

fn measure(count: usize, deep: bool) -> (Duration, Duration) {
    let (mut doc, last) = tree(count, deep);
    let start = Instant::now();
    doc.resolve_stylist(0.0);
    let initial = start.elapsed();
    let body = common::q(&doc, "body");
    doc.mutate().set_attribute(
        body,
        QualName::new(None, ns!(), local_name!("style")),
        "color:rgb(12, 34, 56)",
    );
    let start = Instant::now();
    doc.resolve_stylist(0.0);
    let restyle = start.elapsed();
    assert_eq!(doc.resolved_style_value(last, "color"), "rgb(12, 34, 56)");
    (initial, restyle)
}

#[test]
fn deep_style_cost_is_comparable_to_flat() {
    for count in [2_000, 20_000] {
        let flat = measure(count, false);
        let deep = measure(count, true);
        for (phase, flat, deep) in [("initial", flat.0, deep.0), ("restyle", flat.1, deep.1)] {
            eprintln!(
                "nodes={count} {phase} flat={flat:?} deep={deep:?} ratio={:.2}",
                deep.as_secs_f64() / flat.as_secs_f64()
            );
            // A broad ratio plus a small scheduling allowance rejects the old
            // multi-second ancestor walk without depending on a fast machine.
            assert!(
                deep <= flat * 8 + Duration::from_millis(100),
                "deep={deep:?}, flat={flat:?}"
            );
        }
    }
}

#[test]
fn style_roots_follow_moves_between_shadow_and_light_trees() {
    use blitz_dom::shadow::ShadowRootInit;
    use style::dom::TElement;
    let mut doc = common::parse(
        "<style>div {color:rgb(1, 2, 3)}</style><div id=host></div><div id=branch><div id=leaf></div></div>",
    );
    let host = common::q(&doc, "#host");
    let branch = common::q(&doc, "#branch");
    let leaf = common::q(&doc, "#leaf");
    let body = common::q(&doc, "body");
    let root = doc
        .mutate()
        .attach_shadow(host, ShadowRootInit::default())
        .unwrap();
    doc.mutate()
        .set_inner_html_detaching(root, "<style>div {color:rgb(4, 5, 6)}</style>");
    for (parent, shadow, color) in [
        (root, Some(root), "rgb(4, 5, 6)"),
        (body, None, "rgb(1, 2, 3)"),
        (root, Some(root), "rgb(4, 5, 6)"),
    ] {
        doc.resolve(0.0);
        doc.mutate().append_children(parent, &[branch]);
        // Queries before preparation must not reuse the previous root.
        assert_eq!(
            doc.get_node(leaf)
                .unwrap()
                .containing_shadow()
                .map(|n| n.id),
            shadow
        );
        doc.resolve(0.0);
        assert_eq!(doc.resolved_style_value(leaf, "color"), color);
        assert_eq!(
            doc.get_node(leaf)
                .unwrap()
                .containing_shadow()
                .map(|n| n.id),
            shadow
        );
    }
}

#[test]
fn contents_chain_style_cost_is_comparable_to_flat() {
    let measure_contents = |deep| {
        let (mut doc, last) = tree(20_000, deep);
        let head = common::q(&doc, "head");
        doc.mutate().set_inner_html_detaching(
            head,
            "<style>div {display:contents;color:rgb(12,34,56)}</style>",
        );
        let start = Instant::now();
        doc.resolve_stylist(0.0);
        let elapsed = start.elapsed();
        assert_eq!(doc.resolved_style_value(last, "color"), "rgb(12, 34, 56)");
        elapsed
    };
    let flat = measure_contents(false);
    let deep = measure_contents(true);
    eprintln!("contents nodes=20000 flat={flat:?} deep={deep:?}");
    assert!(
        deep <= flat * 8 + Duration::from_millis(100),
        "deep={deep:?}, flat={flat:?}"
    );
}

#[test]
fn contents_ancestors_use_the_current_box_parent_for_blockification() {
    for display in ["flex", "grid"] {
        let mut doc = common::parse(&format!(
            "<div style='display:{display}'><div id=outer style='display:contents'><div style='display:contents'><span id=leaf>x</span></div></div></div>"
        ));
        let outer = common::q(&doc, "#outer");
        let leaf = common::q(&doc, "#leaf");
        for (ancestor, expected) in [
            ("contents", "block"),
            ("block", "inline"),
            ("contents", "block"),
        ] {
            doc.mutate().set_attribute(
                outer,
                QualName::new(None, ns!(), local_name!("style")),
                &format!("display:{ancestor}"),
            );
            doc.resolve_stylist(0.0);
            assert_eq!(
                doc.resolved_style_value(leaf, "display"),
                expected,
                "{display}, ancestor={ancestor}"
            );
        }
    }
}
