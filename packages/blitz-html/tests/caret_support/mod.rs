//! Shared helpers for the caret movement tests.
// Each test target uses a subset of the helpers.
#![allow(dead_code)]

pub mod recorded;

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

/// Apply steps to a selection, collecting each resulting (anchor, focus) as
/// the selection keeps it for the next step (see [`reported`] for what it
/// reports).
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

/// Apply steps like [`walk`], collecting the range the selection reports
/// after each: `None` once a step leaves no selection, after which steps
/// do nothing.
pub fn reported(
    doc: &BaseDocument,
    start: (Boundary, Boundary),
    steps: &[(Alter, Direction, Granularity)],
) -> Vec<Option<(Boundary, Boundary)>> {
    let (mut anchor, mut focus) = start;
    let mut last_reported = Some(start);
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
            if last_reported.is_some()
                && let Some(moved) = doc.modify_selection(anchor, focus, request)
            {
                (anchor, focus, memory) = (moved.anchor, moved.focus, moved.memory);
                last_reported = moved.reported.map(|r| (r.anchor, r.focus));
            }
            last_reported
        })
        .collect()
}

/// The node at `path` (child indices) below a host's shadow root.
pub fn shadow_child(doc: &BaseDocument, host: NodeId, path: &[usize]) -> NodeId {
    let element = doc.get_node(host).and_then(|node| node.element_data());
    let mut node = element
        .and_then(|element| element.shadow_root)
        .expect("test pages give the host a shadow root");
    for &index in path {
        node = doc.get_node(node).unwrap().children[index];
    }
    node
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
