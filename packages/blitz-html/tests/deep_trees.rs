//! Trees script nests deeper than layout recurses: boxes stop at
//! `MAX_BOX_DEPTH` (1,024) levels, as do `display: contents` and inline
//! content below a box, and the damage passes walk the DOM with explicit
//! stacks. Each test runs on an 8 MiB thread, the guest's stack.

mod common;

use blitz_dom::{BaseDocument, NodeId, QualName, local_name, node::Attribute, ns};
use common::{laid_out, q};

/// Past `MAX_BOX_DEPTH` levels below the root box.
const PAST_THE_CAP: usize = 1_100;

/// A chain of elements that are no boxes, deep enough that one native frame
/// per level overflowed 8 MiB.
const CHAIN: usize = 20_000;

/// The guest's stack. Unoptimized builds lay boxes out in frames several
/// times larger, so tests of the box depth need more there; the guest itself
/// is covered by `browser-host`'s tests.
const GUEST_STACK: usize = 8 << 20;
const BOX_DEPTH_STACK: usize = if cfg!(debug_assertions) {
    64 << 20
} else {
    GUEST_STACK
};

fn on_stack(bytes: usize, test: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(bytes)
        .spawn(test)
        .unwrap()
        .join()
        .unwrap();
}

fn page() -> BaseDocument {
    laid_out("<!doctype html><html><head></head><body></body></html>")
}

/// Nest `depth` `tag` elements (with `style`) under `parent`, each holding a
/// text node before its child; returns the elements, outermost first.
fn nest(
    doc: &mut BaseDocument,
    parent: NodeId,
    tag: &str,
    style: Option<&str>,
    depth: usize,
) -> Vec<NodeId> {
    let mut mutator = doc.mutate();
    let mut elements = Vec::with_capacity(depth);
    let mut node = parent;
    for _ in 0..depth {
        let attrs = style
            .map(|style| Attribute {
                name: QualName::new(None, ns!(), local_name!("style")),
                value: style.into(),
            })
            .into_iter()
            .collect();
        let element = mutator.create_element(QualName::new(None, ns!(html), tag.into()), attrs);
        let text = mutator.create_text_node("x");
        mutator.append_children(element, &[text]);
        mutator.append_children(node, &[element]);
        elements.push(element);
        node = element;
    }
    elements
}

fn has_size(doc: &BaseDocument, node: NodeId) -> bool {
    doc.get_client_bounding_rect(node)
        .is_some_and(|rect| rect.width > 0.0 && rect.height > 0.0)
}

#[test]
fn blocks_past_the_box_depth_get_no_boxes() {
    on_stack(BOX_DEPTH_STACK, || {
        let mut doc = page();
        let body = q(&doc, "body");
        let divs = nest(&mut doc, body, "div", None, PAST_THE_CAP);
        doc.resolve(0.0);
        assert!(has_size(&doc, divs[1_000]));
        assert!(!has_size(&doc, divs[PAST_THE_CAP - 1]));
    });
}

#[test]
fn nested_buttons_and_inline_blocks_stop_at_the_box_depth() {
    on_stack(BOX_DEPTH_STACK, || {
        for (tag, style) in [("button", None), ("span", Some("display:inline-block"))] {
            let mut doc = page();
            let body = q(&doc, "body");
            let boxes = nest(&mut doc, body, tag, style, PAST_THE_CAP);
            doc.resolve(0.0);
            assert!(has_size(&doc, boxes[100]), "{tag}");
            assert!(!has_size(&doc, boxes[PAST_THE_CAP - 1]), "{tag}");
        }
    });
}

#[test]
fn inline_and_contents_chains_stop_at_the_box_depth() {
    // Spans and `display: contents` elements are no boxes, but construction
    // follows them through the DOM, once per level.
    on_stack(GUEST_STACK, || {
        for style in [None, Some("display:contents")] {
            let mut doc = page();
            let body = q(&doc, "body");
            let chain = nest(&mut doc, body, "span", style, CHAIN);
            doc.resolve(0.0);
            // Text within the cap lays out; the deepest text is past it.
            assert!(has_size(&doc, body), "{style:?}");
            if style.is_none() {
                assert!(has_size(&doc, chain[500]));
            }
            assert!(!has_size(&doc, chain[CHAIN - 1]), "{style:?}");
        }
    });
}

#[test]
fn a_deep_chain_is_restyled_and_laid_out_again() {
    // The damage passes visit every level of a chain deeper than the native
    // stack would hold one frame per level.
    on_stack(GUEST_STACK, || {
        let mut doc = page();
        let body = q(&doc, "body");
        let chain = nest(&mut doc, body, "span", None, CHAIN);
        doc.resolve(0.0);
        let first = chain[0];
        let text = doc.get_node(*chain.last().unwrap()).unwrap().children[0];
        doc.mutate().set_attribute(
            first,
            QualName::new(None, ns!(), local_name!("style")),
            "display:block",
        );
        doc.resolve(0.0);
        assert!(doc.get_node(text).is_some());
        // The block holds the chain's first 1,024 levels.
        assert!(has_size(&doc, chain[500]));
    });
}

#[test]
fn boxes_omitted_past_the_depth_follow_their_tree_up_and_down() {
    on_stack(BOX_DEPTH_STACK, || {
        let mut doc = page();
        let body = q(&doc, "body");
        let divs = nest(&mut doc, body, "div", None, PAST_THE_CAP);
        doc.resolve(0.0);
        let deepest = divs[PAST_THE_CAP - 1];
        assert!(!has_size(&doc, deepest));

        // Move the tree below the 200th div to the body: every box is now
        // within the cap, including those omitted before.
        doc.mutate().append_children(body, &[divs[200]]);
        doc.resolve(0.0);
        assert!(has_size(&doc, deepest));

        // And back down: past the cap again.
        doc.mutate().append_children(divs[199], &[divs[200]]);
        doc.resolve(0.0);
        assert!(!has_size(&doc, deepest));
        assert!(has_size(&doc, divs[1_000]));
    });
}

#[test]
fn an_inline_block_omitted_in_deep_inline_content_returns_when_moved_up() {
    on_stack(BOX_DEPTH_STACK, || {
        let mut doc = page();
        let body = q(&doc, "body");
        let spans = nest(&mut doc, body, "span", None, PAST_THE_CAP);
        let inline_block = nest(
            &mut doc,
            spans[PAST_THE_CAP - 1],
            "span",
            Some("display:inline-block"),
            1,
        )[0];
        doc.resolve(0.0);
        assert!(!has_size(&doc, inline_block));

        doc.mutate().append_children(body, &[spans[200]]);
        doc.resolve(0.0);
        assert!(has_size(&doc, inline_block));
    });
}

#[test]
fn table_collection_stops_at_the_depth_limit_and_rebuilds_moved_cells() {
    on_stack(GUEST_STACK, || {
        for display in [
            "contents",
            "table-row-group",
            "table-row",
            "table-column-group",
        ] {
            let mut doc = page();
            let table = doc.mutate().create_element(
                QualName::new(None, ns!(html), local_name!("div")),
                vec![Attribute {
                    name: QualName::new(None, ns!(), local_name!("style")),
                    value: "display:table".into(),
                }],
            );
            let body = q(&doc, "body");
            doc.mutate().append_children(body, &[table]);
            let chain = nest(
                &mut doc,
                table,
                "div",
                Some(&format!("display:{display}")),
                PAST_THE_CAP,
            );
            let cell = doc.mutate().create_element(
                QualName::new(None, ns!(html), local_name!("div")),
                vec![Attribute {
                    name: QualName::new(None, ns!(), local_name!("style")),
                    value: "display:table-cell;width:40px;height:20px".into(),
                }],
            );
            doc.mutate()
                .append_children(*chain.last().unwrap(), &[cell]);
            doc.resolve(0.0);
            assert!(!has_size(&doc, cell), "{display}");
            assert!(
                doc.get_node(*chain.last().unwrap())
                    .unwrap()
                    .flags
                    .contains(blitz_dom::node::NodeFlags::HAS_OMITTED_BOXES),
                "{display}"
            );
            doc.mutate().append_children(table, &[cell]);
            doc.resolve(0.0);
            let size = doc.get_node(cell).unwrap().final_layout().size;
            assert_eq!((size.width, size.height), (40.0, 20.0), "{display}");
        }

        let mut doc = page();
        let table = doc.mutate().create_element(
            QualName::new(None, ns!(html), local_name!("div")),
            vec![Attribute {
                name: QualName::new(None, ns!(), local_name!("style")),
                value: "display:table".into(),
            }],
        );
        let row_group = doc.mutate().create_element(
            QualName::new(None, ns!(html), local_name!("div")),
            vec![Attribute {
                name: QualName::new(None, ns!(), local_name!("style")),
                value: "display:table-row-group".into(),
            }],
        );
        let body = q(&doc, "body");
        doc.mutate().append_children(body, &[table]);
        doc.mutate().append_children(table, &[row_group]);
        let chain = nest(
            &mut doc,
            row_group,
            "div",
            Some("display:contents"),
            PAST_THE_CAP,
        );
        let cell = doc.mutate().create_element(
            QualName::new(None, ns!(html), local_name!("div")),
            vec![Attribute {
                name: QualName::new(None, ns!(), local_name!("style")),
                value: "display:table-cell;width:40px;height:20px".into(),
            }],
        );
        doc.mutate()
            .append_children(*chain.last().unwrap(), &[cell]);
        doc.resolve(0.0);
        assert!(!has_size(&doc, cell), "contents below a row group");

        doc.mutate().append_children(table, &[cell]);
        doc.resolve(0.0);
        let size = doc.get_node(cell).unwrap().final_layout().size;
        assert_eq!((size.width, size.height), (40.0, 20.0));
    });
}

#[test]
fn table_collection_shares_the_outer_box_depth_budget() {
    // Debug frames are substantially larger than Wasm frames. The browser-host
    // component regression runs this same shape on the guest's 8 MiB stack.
    on_stack(BOX_DEPTH_STACK, || {
        for display in [
            "contents",
            "table-row-group",
            "table-row",
            "table-column-group",
        ] {
            let mut doc = page();
            let body = q(&doc, "body");
            let outer = nest(&mut doc, body, "div", None, 900);
            let table = doc.mutate().create_element(
                QualName::new(None, ns!(html), local_name!("div")),
                vec![Attribute {
                    name: QualName::new(None, ns!(), local_name!("style")),
                    value: "display:table".into(),
                }],
            );
            doc.mutate()
                .append_children(*outer.last().unwrap(), &[table]);
            let inner = nest(
                &mut doc,
                table,
                "div",
                Some(&format!("display:{display}")),
                300,
            );
            let cell = doc.mutate().create_element(
                QualName::new(None, ns!(html), local_name!("div")),
                vec![Attribute {
                    name: QualName::new(None, ns!(), local_name!("style")),
                    value: "display:table-cell;width:40px;height:20px".into(),
                }],
            );
            doc.mutate()
                .append_children(*inner.last().unwrap(), &[cell]);
            doc.resolve(0.0);
            assert!(!has_size(&doc, cell), "{display}");

            doc.mutate().append_children(table, &[cell]);
            doc.resolve(0.0);
            let size = doc.get_node(cell).unwrap().final_layout().size;
            assert_eq!((size.width, size.height), (40.0, 20.0), "{display}");
        }
    });
}

#[test]
fn inline_collection_shares_the_outer_box_depth_budget() {
    on_stack(BOX_DEPTH_STACK, || {
        for style in [None, Some("display:contents")] {
            let mut doc = page();
            let body = q(&doc, "body");
            let outer = nest(&mut doc, body, "div", None, 900);
            let inner = nest(&mut doc, *outer.last().unwrap(), "span", style, 300);
            let inline_block = nest(
                &mut doc,
                *inner.last().unwrap(),
                "span",
                Some("display:inline-block;width:40px;height:20px"),
                1,
            )[0];
            doc.resolve(0.0);
            assert!(!has_size(&doc, inline_block), "{style:?}");

            doc.mutate().append_children(body, &[inner[0]]);
            doc.resolve(0.0);
            let size = doc.get_node(inline_block).unwrap().final_layout().size;
            assert_eq!((size.width, size.height), (40.0, 20.0), "{style:?}");
        }
    });
}
