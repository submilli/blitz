//! Shared helpers for the caret movement tests. Expected positions were
//! measured in Chrome 154 on macOS.
#![allow(dead_code)]

use crate::common::{parse, q};
use blitz_dom::caret::{Alter, Direction, Granularity, ModifyRequest};
use blitz_dom::{BaseDocument, NodeId, ranges::Boundary};

pub const FORWARD: Direction = Direction::Forward;
pub const BACKWARD: Direction = Direction::Backward;
pub const FLOAT: &str = "<img style='float:left;width:10px;height:10px'>";

pub fn laid_out(html: &str) -> BaseDocument {
    let mut doc = parse(html);
    *doc.font_ctx().lock().unwrap() = blitz_dom::build_single_font_ctx(include_bytes!(
        "../../../blitz-dom/assets/test-fonts/LiberationSans-Regular.ttf"
    ));
    doc.resolve(0.0);
    doc
}

pub fn child(doc: &BaseDocument, selector: &str, index: usize) -> NodeId {
    doc.get_node(q(doc, selector)).unwrap().children[index]
}

pub fn at(node: NodeId, offset: usize) -> Boundary {
    Boundary { node, offset }
}

/// Apply steps to a selection, collecting each resulting (anchor, focus).
pub fn walk(
    doc: &BaseDocument,
    start: (Boundary, Boundary),
    steps: &[(Alter, Direction, Granularity)],
) -> Vec<(Boundary, Boundary)> {
    let (mut anchor, mut focus) = start;
    let (mut line_x, mut upstream) = (None, false);
    steps
        .iter()
        .map(|&(alter, direction, granularity)| {
            let request = ModifyRequest {
                alter,
                direction,
                granularity,
                line_x,
                upstream,
            };
            if let Some(moved) = doc.modify_selection(anchor, focus, request) {
                (anchor, focus, line_x, upstream) =
                    (moved.anchor, moved.focus, moved.line_x, moved.upstream);
            }
            (anchor, focus)
        })
        .collect()
}

pub fn carets(
    doc: &BaseDocument,
    start: Boundary,
    steps: &[(Alter, Direction, Granularity)],
) -> Vec<Boundary> {
    walk(doc, (start, start), steps)
        .into_iter()
        .map(|(anchor, focus)| {
            assert_eq!(anchor, focus);
            focus
        })
        .collect()
}

pub fn moves(
    direction: Direction,
    granularity: Granularity,
    count: usize,
) -> Vec<(Alter, Direction, Granularity)> {
    vec![(Alter::Move, direction, granularity); count]
}
