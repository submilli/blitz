//! Shared helpers for the caret movement tests.
// Each test target uses a subset of the helpers.
#![allow(dead_code)]

pub use crate::common::laid_out;
use crate::common::q;
use blitz_dom::caret::{Alter, CaretMemory, Direction, Granularity, ModifyRequest};
use blitz_dom::{BaseDocument, NodeId, ranges::Boundary};

pub const FORWARD: Direction = Direction::Forward;
pub const BACKWARD: Direction = Direction::Backward;

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
    let mut memory = CaretMemory::default();
    steps
        .iter()
        .map(|&(alter, direction, granularity)| {
            let request = ModifyRequest {
                alter,
                direction,
                granularity,
                memory,
            };
            if let Some(moved) = doc.modify_selection(anchor, focus, request) {
                (anchor, focus, memory) = (moved.anchor, moved.focus, moved.memory);
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
