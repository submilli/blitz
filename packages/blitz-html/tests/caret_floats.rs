//! `Selection.modify` caret movement around floats, which have no line
//! box: their stops stand for the inline content beside them. Expected
//! positions were measured in Chrome 154 on macOS.

mod caret_support;
mod common;

use blitz_dom::caret::{Alter, Granularity};
use caret_support::*;
use common::q;

#[test]
fn line_boundaries_continue_around_floats() {
    let doc =
        laid_out("<p>hello <img style='float:left;width:10px;height:10px'> world</p><p>next</p>");
    let (hello, world) = (child(&doc, "p", 0), child(&doc, "p", 2));
    let steps = [
        (Alter::Move, FORWARD, Granularity::LineBoundary),
        (Alter::Move, BACKWARD, Granularity::LineBoundary),
        (Alter::Move, FORWARD, Granularity::ParagraphBoundary),
    ];
    assert_eq!(
        carets(&doc, at(hello, 2), &steps),
        [at(world, 6), at(hello, 0), at(hello, 6)]
    );
}

#[test]
fn floats_on_later_lines_do_not_end_the_line() {
    let doc = laid_out("<p>aaa<br>hello <img style='float:left;width:10px;height:10px'> world</p>");
    let (hello, world) = (child(&doc, "p", 2), child(&doc, "p", 4));
    let moved = carets(
        &doc,
        at(hello, 2),
        &moves(FORWARD, Granularity::LineBoundary, 1),
    );
    assert_eq!(moved, [at(world, 6)]);
}

#[test]
fn unselectable_floats_take_no_stops() {
    let doc =
        laid_out("<p>ab<img style='float:left;user-select:none;width:10px;height:10px'>cd</p>");
    let (ab, cd) = (child(&doc, "p", 0), child(&doc, "p", 2));
    assert_eq!(
        carets(&doc, at(ab, 0), &moves(FORWARD, Granularity::Character, 4)),
        [at(ab, 1), at(ab, 2), at(cd, 0), at(cd, 1)]
    );
}

#[test]
fn floats_opening_a_line_belong_to_it_and_locate_back() {
    let doc = laid_out("<p>aaa<br><img style='float:left;width:10px;height:10px'>hello world</p>");
    let (p, aaa, hello) = (q(&doc, "p"), child(&doc, "p", 0), child(&doc, "p", 3));
    let forward = moves(FORWARD, Granularity::LineBoundary, 1);
    assert_eq!(carets(&doc, at(p, 3), &forward), [at(hello, 11)]);
    let lines = moves(FORWARD, Granularity::Line, 2);
    let moved = carets(&doc, at(aaa, 1), &lines);
    assert_ne!(moved[0], moved[1], "line steps must not stall on the float");
}

#[test]
fn breaks_in_nested_boxes_and_unselectable_runs_place_later_floats() {
    let forward = moves(FORWARD, Granularity::LineBoundary, 1);

    let doc = laid_out(&format!(
        "<p>aaa <span style='display:inline-block'>x<br></span>{FLOAT}bbb</p>"
    ));
    let (p, bbb) = (q(&doc, "p"), child(&doc, "p", 3));
    assert_eq!(carets(&doc, at(p, 3), &forward), [at(bbb, 3)]);

    let doc = laid_out(&format!(
        "<p>q <span style='display:inline-block'>{FLOAT}yy</span> zz</p>"
    ));
    let (first, zz) = (child(&doc, "p", 0), child(&doc, "p", 2));
    assert_eq!(carets(&doc, at(first, 0), &forward), [at(zz, 3)]);

    let doc = laid_out(&format!(
        "<p>aaa<span style='user-select:none'>b<br></span>{FLOAT}cc</p>"
    ));
    let (p, cc) = (q(&doc, "p"), child(&doc, "p", 3));
    assert_eq!(carets(&doc, at(p, 3), &forward), [at(cc, 2)]);
}

#[test]
fn floats_sit_on_the_line_content_continues_on() {
    let forward = moves(FORWARD, Granularity::LineBoundary, 1);
    let cases = [
        // Generated content ending in a preserved newline.
        format!(
            "<style>.x::after{{content:'a\\A';white-space:pre}}</style><p>zz<span class=x></span>{FLOAT}bb</p>"
        ),
        // A soft wrap after hidden text and after visible text: a boundary
        // at the wrap belongs to the following line.
        format!(
            "<p style='width:1px'>zz <span style='visibility:hidden'>hidden words</span> {FLOAT}bb</p>"
        ),
        format!("<p style='width:1px'>zz <span>visible words</span> {FLOAT}bb</p>"),
        // A preserved newline inside unselectable text.
        format!("<p>zz<span style='user-select:none;white-space:pre'>a\n</span>{FLOAT}bb</p>"),
        // An unselectable inline-block ending in a forced break.
        format!(
            "<p>zz<span style='display:inline-block;user-select:none'>a<br></span>{FLOAT}bb</p>"
        ),
    ];
    // Each case has the float and a final "bb" Text as children of the `p`.
    for html in cases {
        let doc = laid_out(&html);
        let p = q(&doc, "p");
        let children = &doc.get_node(p).unwrap().children;
        let img = children
            .iter()
            .position(|&id| id == q(&doc, "img"))
            .unwrap();
        let bb = *children.last().unwrap();
        let moved = carets(&doc, at(p, img + 1), &forward);
        assert_eq!(moved, [at(bb, 2)], "{html}");
    }
}

#[test]
fn float_stops_stand_for_the_inline_content_beside_them() {
    let back = moves(BACKWARD, Granularity::LineBoundary, 1);
    let forward = moves(FORWARD, Granularity::LineBoundary, 1);

    // After a float, the content following it on its line; before it, the
    // end of the content preceding it.
    let doc = laid_out(&format!("<p>aa{FLOAT}</p>"));
    let (p, aa) = (q(&doc, "p"), child(&doc, "p", 0));
    assert_eq!(carets(&doc, at(p, 1), &back), [at(aa, 0)]);
    assert_eq!(carets(&doc, at(p, 2), &back), [at(p, 2)]);
    assert_eq!(carets(&doc, at(aa, 1), &forward), [at(aa, 2)]);

    // A float opening a line has no line box: its stops stay.
    let doc = laid_out(&format!("<p>{FLOAT}aa</p>"));
    let (p, aa) = (q(&doc, "p"), child(&doc, "p", 1));
    assert_eq!(carets(&doc, at(p, 0), &forward), [at(p, 0)]);
    assert_eq!(carets(&doc, at(p, 1), &back), [at(aa, 0)]);
    assert_eq!(carets(&doc, at(aa, 1), &back), [at(aa, 0)]);

    let doc = laid_out(&format!("<p>aaa<br>{FLOAT}bb</p>"));
    let (p, bb) = (q(&doc, "p"), child(&doc, "p", 3));
    assert_eq!(carets(&doc, at(p, 2), &forward), [at(p, 2)]);
    assert_eq!(carets(&doc, at(p, 3), &back), [at(bb, 0)]);
    assert_eq!(carets(&doc, at(bb, 1), &back), [at(bb, 0)]);
}

#[test]
fn spaces_before_floats_hang_at_soft_wraps() {
    let doc = laid_out(&format!("<p style='width:1px'>zz {FLOAT}bb cc</p>"));
    let (zz, p, bb) = (child(&doc, "p", 0), q(&doc, "p"), child(&doc, "p", 2));
    let forward = moves(FORWARD, Granularity::LineBoundary, 1);
    assert_eq!(carets(&doc, at(zz, 0), &forward), [at(zz, 2)]);
    // The stop before the float follows the wrap onto the next line.
    let back = moves(BACKWARD, Granularity::LineBoundary, 1);
    assert_eq!(carets(&doc, at(p, 1), &back), [at(bb, 0)]);
    assert_eq!(carets(&doc, at(p, 1), &forward), [at(bb, 2)]);
}

#[test]
fn lines_holding_only_boxes_keep_their_place() {
    let back = moves(BACKWARD, Granularity::LineBoundary, 1);

    let doc = laid_out(&format!("<p>aaa<br>{FLOAT}</p>"));
    let p = q(&doc, "p");
    assert_eq!(carets(&doc, at(p, 2), &back), [at(p, 2)]);
    assert_eq!(carets(&doc, at(p, 3), &back), [at(p, 3)]);

    let doc = laid_out(&format!(
        "<p style='width:1px'>aa<span style='display:inline-block'>x</span>{FLOAT}bbbb</p>"
    ));
    let (p, aa, bbbb) = (q(&doc, "p"), child(&doc, "p", 0), child(&doc, "p", 3));
    assert_eq!(carets(&doc, at(bbbb, 1), &back), [at(bbbb, 0)]);
    assert_eq!(carets(&doc, at(p, 2), &back), [at(aa, 2)]);
}

#[test]
fn line_steps_reach_the_root_line_past_a_float() {
    let doc = laid_out(&format!("<p>aaa<br>{FLOAT}bb</p>"));
    let (aaa, bb) = (child(&doc, "p", 0), child(&doc, "p", 3));
    assert_eq!(
        carets(&doc, at(bb, 0), &moves(BACKWARD, Granularity::Line, 1)),
        [at(aaa, 0)]
    );
    assert_eq!(
        carets(&doc, at(aaa, 0), &moves(FORWARD, Granularity::Line, 1)),
        [at(bb, 0)]
    );
}

#[test]
fn float_stops_stay_beside_breaks_floats_and_boxes() {
    let back = moves(BACKWARD, Granularity::LineBoundary, 1);
    let forward = moves(FORWARD, Granularity::LineBoundary, 1);

    // A forced break inside unselectable content opens the float's line.
    let doc = laid_out(&format!(
        "<p>zz<span style='user-select:none'>a<br></span>{FLOAT}bb</p>"
    ));
    let p = q(&doc, "p");
    assert_eq!(carets(&doc, at(p, 2), &back), [at(p, 2)]);
    assert_eq!(carets(&doc, at(p, 2), &forward), [at(p, 2)]);

    // Between two floats, neither stop has inline content beside it.
    let doc = laid_out(&format!("<p>aa {FLOAT}{FLOAT}bb</p>"));
    let p = q(&doc, "p");
    assert_eq!(carets(&doc, at(p, 2), &back), [at(p, 2)]);
    assert_eq!(carets(&doc, at(p, 2), &forward), [at(p, 2)]);

    // After a float, the content of a box that follows it; before a float,
    // a box that precedes it leaves the stop where it is.
    let doc = laid_out(&format!(
        "<p>aa{FLOAT}<span style='display:inline-block'><span style='display:block'>X</span></span>bb</p>"
    ));
    let (p, x) = (q(&doc, "p"), child(&doc, "span span", 0));
    assert_eq!(carets(&doc, at(p, 2), &forward), [at(x, 1)]);
    assert_eq!(carets(&doc, at(p, 2), &back), [at(x, 0)]);
    let doc = laid_out(&format!(
        "<p>aa<span style='float:left'>XY</span>{FLOAT}bb</p>"
    ));
    let p = q(&doc, "p");
    assert_eq!(carets(&doc, at(p, 2), &back), [at(p, 2)]);
}

#[test]
fn line_steps_from_and_into_floats() {
    // The stop before a float opening a line steps from the line before.
    let doc = laid_out(&format!("<p>aaaa<br>{FLOAT}cc<br>dddd</p>"));
    let (p, cc) = (q(&doc, "p"), child(&doc, "p", 3));
    let down = moves(FORWARD, Granularity::Line, 1);
    assert_eq!(carets(&doc, at(p, 2), &down), [at(cc, 0)]);

    // A paragraph opening with a float is entered at its stop facing the
    // movement, whatever line the caret left.
    let doc = laid_out(&format!("<p>aaaa<br>aaaa</p><p>{FLOAT}bbbb</p>"));
    let (second, next) = (child(&doc, "p", 2), q(&doc, "p + p"));
    assert_eq!(carets(&doc, at(second, 2), &down), [at(next, 0)]);
    let doc = laid_out(&format!("<p>bb<br>bbbb{FLOAT}</p><p>aaaa<br>aaaa</p>"));
    let (first, aaaa) = (q(&doc, "p"), child(&doc, "p + p", 0));
    let up = moves(BACKWARD, Granularity::Line, 1);
    assert_eq!(carets(&doc, at(aaaa, 2), &up), [at(first, 4)]);
}

#[test]
fn lines_after_floats_end_at_breaks_and_content_without_stops() {
    let back = moves(BACKWARD, Granularity::LineBoundary, 1);
    let forward = moves(FORWARD, Granularity::LineBoundary, 1);

    // A forced break after a float leaves both stops with the content
    // before it.
    let doc = laid_out(&format!("<p>aa{FLOAT}<br>bb</p>"));
    let (p, aa) = (q(&doc, "p"), child(&doc, "p", 0));
    assert_eq!(carets(&doc, at(p, 2), &back), [at(aa, 0)]);
    assert_eq!(carets(&doc, at(p, 2), &forward), [at(aa, 2)]);

    // Content without stops after a float ends the line at its stop; a
    // block-level box after it does not.
    let doc = laid_out(&format!(
        "<p>aa{FLOAT}<span style='visibility:hidden'>hh</span></p>"
    ));
    let (p, aa) = (q(&doc, "p"), child(&doc, "p", 0));
    assert_eq!(carets(&doc, at(aa, 1), &forward), [at(p, 2)]);
    let doc = laid_out(&format!(
        "<style>.cf::after{{content:'';display:table;clear:both}}</style><p class=cf>aa{FLOAT}</p><p>bb</p>"
    ));
    let (p, aa) = (q(&doc, "p"), child(&doc, "p", 0));
    assert_eq!(carets(&doc, at(p, 1), &forward), [at(aa, 2)]);
    assert_eq!(carets(&doc, at(p, 2), &forward), [at(p, 2)]);

    // A space before a forced break in generated content does not hang.
    let doc = laid_out(&format!(
        "<style>.x::after{{content:'a\\A';white-space:pre}}</style><p>zz <span class=x></span>{FLOAT}bb</p>"
    ));
    let zz = child(&doc, "p", 0);
    assert_eq!(carets(&doc, at(zz, 0), &forward), [at(zz, 3)]);
}

#[test]
fn line_steps_leave_floats_that_open_the_root_or_end_a_box() {
    let down = moves(FORWARD, Granularity::Line, 1);
    let doc = laid_out(&format!("<p>{FLOAT}aa<br>bb</p>"));
    let (p, aa) = (q(&doc, "p"), child(&doc, "p", 1));
    assert_eq!(carets(&doc, at(p, 0), &down), [at(aa, 0)]);

    let doc = laid_out(&format!(
        "<p>q <span style='display:inline-block'>yy{FLOAT}</span></p><p>bb</p>"
    ));
    let (span, bb) = (q(&doc, "span"), child(&doc, "p + p", 0));
    assert_eq!(carets(&doc, at(span, 1), &down), [at(bb, 2)]);
}

#[test]
fn generated_breaks_and_preserved_spaces_keep_float_stops_on_the_earlier_line() {
    let back = moves(BACKWARD, Granularity::LineBoundary, 1);
    let forward = moves(FORWARD, Granularity::LineBoundary, 1);
    let generated = "<style>.x::after{content:'a\\A';white-space:pre}</style>";

    // Only a forced break from the DOM opens the float's line.
    let doc = laid_out(&format!(
        "{generated}<p>zz<span class=x></span>{FLOAT}bb</p>"
    ));
    let (p, zz) = (q(&doc, "p"), child(&doc, "p", 0));
    assert_eq!(carets(&doc, at(p, 2), &back), [at(zz, 0)]);
    assert_eq!(carets(&doc, at(p, 2), &forward), [at(zz, 2)]);
    let doc = laid_out(&format!(
        "{generated}<p>zz<br><span class=x></span>{FLOAT}bb</p>"
    ));
    let p = q(&doc, "p");
    assert_eq!(carets(&doc, at(p, 3), &back), [at(p, 3)]);

    // A preserved space at a soft wrap does not hang.
    let doc = laid_out(&format!(
        "<p style='white-space:pre-wrap;width:60px'>aaa bbb {FLOAT}ccc</p>"
    ));
    let (p, text) = (q(&doc, "p"), child(&doc, "p", 0));
    assert_eq!(carets(&doc, at(p, 1), &back), [at(text, 0)]);

    // A stop never stands for another float's.
    let doc = laid_out(&format!(
        "<style>.g::before{{content:'zz'}}</style><p>aa {FLOAT}<span class=g></span>{FLOAT}bb</p>"
    ));
    let p = q(&doc, "p");
    assert_eq!(carets(&doc, at(p, 2), &back), [at(p, 2)]);
    assert_eq!(carets(&doc, at(p, 2), &forward), [at(p, 2)]);

    // A box that wraps below the float leaves the line's end at the text.
    let doc = laid_out(&format!(
        "<p style='width:50px'>aa {FLOAT}<span style='display:inline-block'><span style='display:block'>XXXXXXXX</span></span>bb</p>"
    ));
    let aa = child(&doc, "p", 0);
    assert_eq!(carets(&doc, at(aa, 0), &forward), [at(aa, 2)]);
}

#[test]
fn float_runs_before_breaks_stand_for_the_content_before_them() {
    let doc = laid_out(&format!("<p>aa{FLOAT}{FLOAT}<br>bb</p>"));
    let (p, aa) = (q(&doc, "p"), child(&doc, "p", 0));
    let back = moves(BACKWARD, Granularity::LineBoundary, 1);
    let forward = moves(FORWARD, Granularity::LineBoundary, 1);
    assert_eq!(carets(&doc, at(p, 3), &back), [at(aa, 0)]);
    assert_eq!(carets(&doc, at(p, 3), &forward), [at(aa, 2)]);
    assert_eq!(carets(&doc, at(p, 1), &forward), [at(aa, 2)]);
    // Between the floats, a stop stands for nothing.
    assert_eq!(carets(&doc, at(p, 2), &back), [at(p, 2)]);
}

#[test]
fn float_runs_after_generated_breaks_and_at_the_end_of_boxes() {
    let back = moves(BACKWARD, Granularity::LineBoundary, 1);
    let forward = moves(FORWARD, Granularity::LineBoundary, 1);

    // A run opening a line after a generated break keeps its last stop.
    let doc = laid_out(&format!(
        "<style>.x::after{{content:'a\\A';white-space:pre}}</style><p>aa<span class=x></span>{FLOAT}{FLOAT}<br>bb</p>"
    ));
    let p = q(&doc, "p");
    assert_eq!(carets(&doc, at(p, 4), &forward), [at(p, 4)]);

    // A float ending a box stands for the point after the box.
    let doc = laid_out(&format!(
        "<p>xx<span style='display:inline-block'>aa{FLOAT}</span><br>yy</p>"
    ));
    let (p, span, xx) = (q(&doc, "p"), q(&doc, "span"), child(&doc, "p", 0));
    assert_eq!(carets(&doc, at(span, 2), &forward), [at(p, 2)]);
    assert_eq!(carets(&doc, at(span, 2), &back), [at(xx, 0)]);
}
