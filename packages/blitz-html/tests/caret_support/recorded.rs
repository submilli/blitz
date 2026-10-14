//! Steps recorded as Chrome's probes record them: a page, a start from
//! `body` by child path and offset, and `;`-separated
//! `direction:granularity[:extend]` steps, each result named `#text:offset`
//! or `TAG:offset`.

use super::laid_out;
use crate::common::laid_out_with_browser_fonts;
use crate::common::q;
use blitz_dom::caret::{Alter, CaretMemory, Direction, Granularity, ModifyRequest};
use blitz_dom::{BaseDocument, NodeId, ranges::Boundary};

/// The selection after each step, as the probes record it.
pub fn run(html: &str, path: &str, offset: usize, steps: &str) -> String {
    let doc = laid_out(&format!(
        "<!doctype html><html><head></head><body>{html}</body></html>"
    ));
    run_on(&doc, path, offset, steps)
}

/// `run` with the fonts the browser lays pages out with, the bullet font
/// among them.
pub fn run_with_browser_fonts(html: &str, path: &str, offset: usize, steps: &str) -> String {
    let doc = laid_out_with_browser_fonts(&format!(
        "<!doctype html><html><head></head><body>{html}</body></html>"
    ));
    run_on(&doc, path, offset, steps)
}

fn run_on(doc: &BaseDocument, path: &str, offset: usize, steps: &str) -> String {
    let mut node = q(doc, "body");
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
    let mut directional = false;
    let mut results = Vec::new();
    for step in steps.split(';') {
        let request = ModifyRequest {
            directional,
            ..request(step, memory)
        };
        if let Some(moved) = doc.modify_selection(anchor, focus, request) {
            let next_directional = request.alter == Alter::Extend;
            let unchanged = moved.anchor == anchor
                && moved.focus == focus
                && moved.memory.upstream == memory.upstream
                && moved.memory.anchor_type == memory.anchor_type
                && moved.memory.focus_type == memory.focus_type
                && directional == next_directional;
            let line_x = if unchanged {
                moved.memory.line_x.or(memory.line_x)
            } else {
                moved.memory.line_x
            };
            (anchor, focus, memory) = (
                moved.anchor,
                moved.focus,
                CaretMemory {
                    line_x,
                    ..moved.memory
                },
            );
            directional = next_directional;
        }
        let (a, f) = (name(doc, anchor), name(doc, focus));
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
        Some("sentence") => Granularity::Sentence,
        Some("sentenceboundary") => Granularity::SentenceBoundary,
        Some("paragraphboundary") => Granularity::ParagraphBoundary,
        Some("documentboundary") => Granularity::DocumentBoundary,
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
        directional: true,
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
