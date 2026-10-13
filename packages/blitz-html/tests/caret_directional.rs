//! Extending a selection that is not directional, as Chrome's
//! `SelectionModifier::PrepareToModifySelection` does: the anchor moves to
//! the end opposite the side the step moves toward. Expected selections
//! were measured in Chrome 155 on macOS (submilli-browser's
//! `fixtures/local/selection-modify/directional.js`).

mod caret_support;
mod common;

use blitz_dom::BaseDocument;
use blitz_dom::caret::{Alter, CaretMemory, Direction, Granularity, ModifyRequest};
use blitz_dom::form_selection::{ControlSelection, SelectionDirection};
use blitz_dom::ranges::Boundary;
use caret_support::*;
use common::q;

const LEFT: Direction = Direction::Left;
const RIGHT: Direction = Direction::Right;

fn extend(direction: Direction, granularity: Granularity, directional: bool) -> ModifyRequest {
    ModifyRequest {
        alter: Alter::Extend,
        direction,
        granularity,
        directional,
        memory: CaretMemory::default(),
    }
}

/// Extend the range from `<p>`'s text at 1 to the second `<p>`'s at 4.
fn extended(doc: &BaseDocument, request: ModifyRequest) -> (Boundary, Boundary) {
    let (a, c) = (child(doc, "p", 0), child(doc, "p + p", 0));
    let moved = doc.modify_selection(at(a, 1), at(c, 4), request).unwrap();
    (moved.anchor, moved.focus)
}

#[test]
fn extensions_grow_a_range_without_a_direction_at_the_side_they_move_toward() {
    let doc = laid_out("<p>aaaa bbbb</p><p>cccc dddd</p>");
    let (a, c) = (child(&doc, "p", 0), child(&doc, "p + p", 0));
    let backward = (at(c, 4), at(a, 0));
    for (direction, granularity, chrome) in [
        (BACKWARD, Granularity::Character, backward),
        (FORWARD, Granularity::Character, (at(a, 1), at(c, 5))),
        (BACKWARD, Granularity::Word, backward),
        (BACKWARD, Granularity::Line, backward),
        (BACKWARD, Granularity::Paragraph, backward),
        (BACKWARD, Granularity::Sentence, backward),
        (BACKWARD, Granularity::LineBoundary, backward),
        (BACKWARD, Granularity::ParagraphBoundary, backward),
        (BACKWARD, Granularity::DocumentBoundary, backward),
        (BACKWARD, Granularity::SentenceBoundary, backward),
        (LEFT, Granularity::Character, backward),
        (RIGHT, Granularity::Character, (at(a, 1), at(c, 5))),
        (FORWARD, Granularity::Word, (at(a, 1), at(c, 9))),
        (FORWARD, Granularity::Line, (at(a, 1), at(c, 9))),
    ] {
        let request = extend(direction, granularity, false);
        assert_eq!(
            extended(&doc, request),
            chrome,
            "{direction:?} {granularity:?}"
        );
    }
    // A directional range extends from its focus.
    let request = extend(BACKWARD, Granularity::Character, true);
    assert_eq!(extended(&doc, request), (at(a, 1), at(c, 3)));
}

/// Left and right take the direction of the text at both ends of the
/// range when they agree, else the direction of the focus's block.
#[test]
fn left_and_right_orient_by_the_text_direction_at_the_ends() {
    let latin = "<p dir=rtl>aaaa bbbb</p><p dir=rtl>cccc dddd</p>";
    let hebrew = "<p dir=rtl>אבגדהוזח</p><p dir=rtl>אבגדהוזח</p>";
    let ltr_hebrew = "<p>אבגדהוזח</p><p>אבגדהוזח</p>";
    let mixed = "<p dir=rtl>אבגדהוזח</p><p dir=rtl>abcdefgh</p>";
    for (html, direction, chrome) in [
        (latin, LEFT, ((1, 4), (0, 2))),
        (latin, RIGHT, ((0, 1), (1, 3))),
        (hebrew, LEFT, ((0, 1), (1, 5))),
        (hebrew, RIGHT, ((1, 4), (0, 0))),
        (ltr_hebrew, LEFT, ((0, 1), (1, 3))),
        (mixed, LEFT, ((0, 1), (1, 5))),
        (mixed, RIGHT, ((1, 4), (0, 0))),
    ] {
        let doc = laid_out(html);
        let texts = [child(&doc, "p", 0), child(&doc, "p + p", 0)];
        let point = |(text, offset): (usize, usize)| at(texts[text], offset);
        let request = extend(direction, Granularity::Character, false);
        assert_eq!(
            extended(&doc, request),
            (point(chrome.0), point(chrome.1)),
            "{html} {direction:?}"
        );
    }
}

/// A text control's range is not directional while its
/// `selectionDirection` is "none" (`setSelectionRange` without a
/// direction, or `select()`).
#[test]
fn text_control_ranges_without_a_direction_grow_at_the_side_extensions_move_toward() {
    use SelectionDirection::{Backward as B, Forward as F, None as N};
    let latin = "<textarea>abcdefgh</textarea>";
    let rtl_latin = "<textarea dir=rtl>abcdefgh</textarea>";
    let rtl_hebrew = "<textarea dir=rtl>אבגדהוזח</textarea>";
    let hebrew = "<textarea>אבגדהוזח</textarea>";
    for (html, (start, end, given), direction, chrome) in [
        (latin, (2, 5, N), BACKWARD, (1, 5, B)),
        (latin, (2, 5, N), FORWARD, (2, 6, F)),
        (latin, (2, 5, F), BACKWARD, (2, 4, F)),
        (latin, (2, 5, B), FORWARD, (3, 5, B)),
        (latin, (0, 8, N), BACKWARD, (0, 8, N)),
        (rtl_hebrew, (2, 5, N), LEFT, (2, 6, F)),
        (rtl_hebrew, (2, 5, N), RIGHT, (1, 5, B)),
        // Chrome: 3-5 backward, 2-4 forward and 2-4 forward. Each range is
        // oriented as Chrome's (by its text's direction), but the step
        // moves the other way: the editor lays out in the direction its
        // text implies rather than the control's (SUB-1705).
        (rtl_latin, (2, 5, N), LEFT, (1, 5, B)),
        (rtl_latin, (2, 5, N), RIGHT, (2, 6, F)),
        (hebrew, (2, 5, N), LEFT, (2, 6, F)),
    ] {
        let mut doc = laid_out(html);
        let area = q(&doc, "textarea");
        let selection = ControlSelection {
            start,
            end,
            direction: given,
        };
        doc.mutate().set_control_selection(area, selection);
        doc.resolve(0.0);
        let request = extend(direction, Granularity::Character, true);
        let _ = doc.mutate().modify_control_selection(area, request);
        let s = doc.control_selection(area).unwrap();
        assert_eq!(
            (s.start, s.end, s.direction),
            chrome,
            "{html} {start}-{end} {given:?} {direction:?}"
        );
    }
}
