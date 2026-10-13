//! Steps recorded as Chrome's probes record them: a page, a start from
//! `body` by child path and offset, and `;`-separated
//! `direction:granularity[:extend]` steps, each result named `#text:offset`
//! or `TAG:offset`.

use super::laid_out;
use crate::common::q;
use blitz_dom::caret::{Alter, CaretMemory, Direction, Granularity, ModifyRequest};
use blitz_dom::{BaseDocument, NodeId, ranges::Boundary};

/// The selection after each step, as the probes record it.
pub fn run(html: &str, path: &str, offset: usize, steps: &str) -> String {
    let doc = laid_out(&format!(
        "<!doctype html><html><head></head><body>{html}</body></html>"
    ));
    let mut node = q(&doc, "body");
    for index in path.split(',').filter(|i| !i.is_empty()) {
        let Some(&child) = doc
            .get_node(node)
            .unwrap()
            .children
            .get(index.parse::<usize>().unwrap())
        else {
            // `collapse` with an absent node empties the selection.
            return vec!["null"; steps.split(';').count()].join(" | ");
        };
        node = child;
    }
    let length = doc.get_node(node).map_or(0, |n| {
        n.text_data()
            .map_or(n.children.len(), |t| t.content.to_utf16().len())
    });
    if offset > length {
        // `collapse` throws for an offset past the node.
        return "ERR".into();
    }
    let start = Boundary { node, offset };
    let (mut anchor, mut focus) = (start, start);
    // What consecutive steps keep, as `selection.js` keeps it.
    let mut memory = CaretMemory::default();
    let mut results = Vec::new();
    for step in steps.split(';') {
        let request = request(step, memory);
        if let Some(moved) = doc.modify_selection(anchor, focus, request) {
            (anchor, focus, memory) = (moved.anchor, moved.focus, moved.memory);
        }
        let (a, f) = (name(&doc, anchor), name(&doc, focus));
        results.push(if a == f { f } else { format!("{a}..{f}") });
    }
    results.join(" | ")
}

fn request(step: &str, memory: CaretMemory) -> ModifyRequest {
    let parts: Vec<&str> = step.split(':').collect();
    let direction = match parts[0] {
        "forward" => Direction::Forward,
        "backward" => Direction::Backward,
        "left" => Direction::Left,
        "right" => Direction::Right,
        other => panic!("unknown direction {other:?} in step {step:?}"),
    };
    let granularity = match parts.get(1).copied() {
        Some("lineboundary") => Granularity::LineBoundary,
        Some("line") => Granularity::Line,
        Some("paragraph") => Granularity::Paragraph,
        Some("word") => Granularity::Word,
        Some("character") => Granularity::Character,
        other => panic!("unknown granularity {other:?} in step {step:?}"),
    };
    let alter = match parts.get(2).copied() {
        None => Alter::Move,
        Some("extend") => Alter::Extend,
        Some(other) => panic!("unknown alter {other:?} in step {step:?}"),
    };
    ModifyRequest {
        alter,
        direction,
        granularity,
        memory,
    }
}

/// A boundary point as the corpus names it: `#text:offset` or `TAG:offset`.
fn name(doc: &BaseDocument, point: Boundary) -> String {
    let node = doc.get_node(point.node).unwrap();
    match node.text_data() {
        Some(text) => format!(
            "#{}:{}",
            text.content.as_str_lossy().replace('\n', "\\n"),
            point.offset
        ),
        None => format!("{}:{}", tag(doc, point.node), point.offset),
    }
}

fn tag(doc: &BaseDocument, node: NodeId) -> String {
    doc.get_node(node)
        .and_then(|n| n.element_data())
        .map_or_else(String::new, |e| e.name.local.to_uppercase())
}
