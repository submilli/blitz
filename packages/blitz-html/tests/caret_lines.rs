//! Line steps and boundaries in Chrome's model (results from Chrome 154 on
//! macOS): canonical positions, line boxes and hit testing.

mod caret_support;
mod common;

use blitz_dom::caret::{Alter, CaretMemory, Direction, Granularity, ModifyRequest};
use caret_support::{
    BACKWARD, FORWARD, at, carets, child, laid_out, moves, reported, shadow_child, walk,
};
use common::{FLOAT, q};

#[test]
fn a_line_of_generated_content_takes_a_step_to_the_document_start() {
    // Chrome finds the generated line a caret container, as a DOM leaf
    // follows it in the context, but hits no DOM position on it.
    let doc = laid_out(
        "<style>.x::after{content:'a\\A';white-space:pre}</style><p>zz<br><span class=x></span>bb</p>",
    );
    let (zz, bb) = (child(&doc, "p", 0), child(&doc, "p", 3));
    assert_eq!(
        carets(&doc, at(bb, 1), &moves(BACKWARD, Granularity::Line, 1)),
        [at(zz, 0)]
    );
}

#[test]
fn a_step_out_of_an_inline_block_passes_its_own_line() {
    // A button lays out as an inline-block: lines are compared by the
    // outer block's inline contents.
    let doc = laid_out(&format!("<p>xx</p><p>aa{FLOAT}<button>bt</button></p>"));
    let xx = child(&doc, "p", 0);
    let bt = doc.get_node(q(&doc, "button")).unwrap().children[0];
    assert_eq!(
        carets(&doc, at(bt, 1), &moves(BACKWARD, Granularity::Line, 1)),
        [at(xx, 2)]
    );
}

#[test]
fn the_kept_horizontal_position_is_the_centred_caret() {
    // The bar caret after "b" sits half its width before the boundary, so
    // it hits the left half of the space above.
    let doc = laid_out(&format!("<p>zz <span>a<br></span>{FLOAT}bb</p>"));
    let (zz, bb) = (child(&doc, "p", 0), child(&doc, "p", 3));
    assert_eq!(
        carets(&doc, at(bb, 1), &moves(BACKWARD, Granularity::Line, 1)),
        [at(zz, 2)]
    );
}

#[test]
fn line_boundaries_of_right_to_left_lines_are_logical() {
    let doc = laid_out("<p dir=rtl>aa <span>bb</span></p>");
    let aa = child(&doc, "p", 0);
    let bb = doc.get_node(q(&doc, "span")).unwrap().children[0];
    assert_eq!(
        carets(
            &doc,
            at(aa, 1),
            &moves(FORWARD, Granularity::LineBoundary, 1)
        ),
        [at(bb, 2)]
    );
    assert_eq!(
        carets(
            &doc,
            at(aa, 1),
            &moves(BACKWARD, Granularity::LineBoundary, 1)
        ),
        [at(aa, 0)]
    );
}

#[test]
fn an_extended_range_reports_its_smallest_equivalent() {
    // The focus at "bb" start reports as the point after the <br>.
    let doc = laid_out(&format!("<p>aa<br>bb{FLOAT}</p><p>cc</p>"));
    let (p, aa) = (q(&doc, "p"), child(&doc, "p", 0));
    let extend = [(Alter::Extend, FORWARD, Granularity::Line)];
    assert_eq!(
        walk(&doc, (at(aa, 1), at(aa, 1)), &extend),
        [(at(aa, 1), at(p, 2))]
    );
}

#[test]
fn a_step_past_its_work_budget_leaves_the_selection() {
    // Each empty span is a leaf on the line whose canonical position walks
    // back over the spans before it.
    let spans = "<span></span>".repeat(3000);
    let doc = laid_out(&format!("<p>xx</p><p>aa{spans}bb</p>"));
    let p = q(&doc, "p + p");
    let bb = *doc.get_node(p).unwrap().children.last().unwrap();
    assert_eq!(
        carets(&doc, at(bb, 1), &moves(BACKWARD, Granularity::Line, 1)),
        [at(bb, 1)]
    );
}

#[test]
fn a_step_from_geometry_overflowing_to_infinity_does_not_panic() {
    // The block's edges sum to infinity, so the caret's clamp bounds are NaN.
    let doc = laid_out("<p style='margin-left:2e38px;padding-left:2e38px'>aa</p><p>bb</p>");
    let aa = child(&doc, "p", 0);
    let moved = carets(&doc, at(aa, 1), &moves(FORWARD, Granularity::Line, 1));
    assert_eq!(moved.len(), 1);
}

#[test]
fn steps_pass_many_empty_boxes_between_lines() {
    // Each empty box's position canonicalizes by scanning to the text on
    // either side; the scans share their result, so the steps stay within
    // the work budget (Chrome: "b" then "a").
    let empty = "<div></div>".repeat(3000);
    let doc = laid_out(&format!("<p>a</p>{empty}<p>b</p>"));
    let a = child(&doc, "p", 0);
    let b = child(&doc, "p:last-of-type", 0);
    assert_eq!(
        carets(&doc, at(a, 1), &moves(FORWARD, Granularity::Line, 1)),
        [at(b, 1)]
    );
    assert_eq!(
        carets(&doc, at(b, 1), &moves(BACKWARD, Granularity::Line, 1)),
        [at(a, 1)]
    );
}

#[test]
fn a_wrapped_left_to_right_run_in_a_right_to_left_line_keeps_its_place() {
    // UAX #9 L1 gives the space ending each wrapped line the paragraph's
    // direction, so it hangs at the line's left end and the words sit flush
    // right. A line step keeps the caret's x there: Chrome 154 goes from
    // offset 2 of the first line to offset 17 of the second, and back.
    let doc = laid_out(
        "<!doctype html><style>body{font-family:Arial;font-size:16px}</style>\
         <div dir=rtl style='width:120px'>aaa bbb ccc ddd eee fff ggg hhh</div>",
    );
    let text = child(&doc, "div", 0);
    for direction in [FORWARD, Direction::Right] {
        assert_eq!(
            carets(&doc, at(text, 2), &moves(direction, Granularity::Line, 1)),
            [at(text, 17)]
        );
    }
    assert_eq!(
        carets(&doc, at(text, 17), &moves(BACKWARD, Granularity::Line, 1)),
        [at(text, 2)]
    );
}

#[test]
fn a_wrapped_mixed_direction_right_to_left_line_ends_before_its_space() {
    // The first line holds "אבג abc אבג" and the space after it, which
    // takes the paragraph's direction (UAX #9 L1) and hangs: Chrome 154 ends
    // the line before the space, at offset 11.
    let doc = laid_out(
        "<!doctype html><style>body{font-family:Arial;font-size:16px}</style>\
         <div dir=rtl style='width:90px'>אבג abc אבג def אבג ghi</div>",
    );
    let text = child(&doc, "div", 0);
    for offset in [2, 5, 9] {
        assert_eq!(
            carets(
                &doc,
                at(text, offset),
                &moves(FORWARD, Granularity::LineBoundary, 1)
            ),
            [at(text, 11)],
            "from {offset}"
        );
    }
}

#[test]
fn right_and_left_by_line_and_paragraph_are_logical() {
    // Chrome 154 leaves `BidiVisualOrderCaretMovement` off: right is the
    // next line or paragraph and left the previous one, in any direction.
    let doc = laid_out("<div dir=rtl>aaa<br>bbb</div><p dir=rtl>ccc</p><p dir=rtl>ddd</p>");
    let (aaa, bbb) = (child(&doc, "div", 0), child(&doc, "div", 2));
    let (right, left) = (Direction::Right, Direction::Left);
    assert_eq!(
        carets(&doc, at(aaa, 1), &moves(right, Granularity::Line, 1)),
        [at(bbb, 1)]
    );
    assert_eq!(
        carets(&doc, at(bbb, 1), &moves(left, Granularity::Line, 1)),
        [at(aaa, 1)]
    );
    let (ccc, ddd) = (child(&doc, "p", 0), child(&doc, "p:last-of-type", 0));
    assert_eq!(
        carets(&doc, at(ccc, 1), &moves(right, Granularity::Paragraph, 1)),
        [at(ddd, 1)]
    );
    assert_eq!(
        carets(&doc, at(ddd, 1), &moves(left, Granularity::Paragraph, 1)),
        [at(ccc, 1)]
    );
}

#[test]
fn extension_by_line_keeps_ends_beside_shadow_hosts() {
    // Chrome keeps the visible selection of a step in the DOM tree, which
    // moves an end that a line step carries into a shadow tree beside its
    // host: the end of a forward range before it. Made visible again, that
    // end reports as `(<p>, 0)`, the paragraph after the host. The next
    // step starts from the kept end, which in the flat tree is on the
    // shadow line again (results from Chrome 155).
    for mode in ["open", "closed"] {
        let doc = laid_out(&format!(
            "<p>aa</p><div><template shadowrootmode={mode}><span>shadow</span></template></div><p>tail</p>"
        ));
        let aa = child(&doc, "p", 0);
        let tail_p = q(&doc, "p:last-of-type");
        let tail = child(&doc, "p:last-of-type", 0);
        let down = (Alter::Extend, FORWARD, Granularity::Line);
        let up = (Alter::Extend, BACKWARD, Granularity::Line);
        assert_eq!(
            reported(&doc, (at(aa, 1), at(aa, 1)), &[down, down, up]),
            [
                Some((at(aa, 1), at(tail_p, 0))),
                Some((at(aa, 1), at(tail, 1))),
                Some((at(aa, 1), at(tail_p, 0))),
            ],
            "{mode}"
        );
        // Up from `tail`, the start moves after the host, and the next step
        // goes on from the shadow line.
        assert_eq!(
            reported(&doc, (at(tail, 3), at(tail, 3)), &[up, up]),
            [
                Some((at(tail, 3), at(tail, 0))),
                Some((at(tail, 3), at(aa, 2))),
            ],
            "{mode}"
        );
    }
}

#[test]
fn a_position_before_a_host_is_on_the_line_before_it() {
    // Of the candidates around (body, 1), before the host, Chrome takes the
    // one in the body's own DOM subtree: shadow content is not inside it.
    let doc = laid_out(
        "<p>aa</p><div><template shadowrootmode=open><span>shadow</span></template></div><p>tail</p>",
    );
    let body = q(&doc, "body");
    let aa = child(&doc, "p", 0);
    let shadow = shadow_child(&doc, q(&doc, "div"), &[0, 0]);
    assert_eq!(
        carets(
            &doc,
            at(body, 1),
            &moves(FORWARD, Granularity::LineBoundary, 1)
        ),
        [at(aa, 2)]
    );
    assert_eq!(
        carets(
            &doc,
            at(body, 1),
            &moves(BACKWARD, Granularity::LineBoundary, 1)
        ),
        [at(aa, 0)]
    );
    // A line step down from there reaches the shadow line.
    let [down] = carets(&doc, at(body, 1), &moves(FORWARD, Granularity::Line, 1))[..] else {
        panic!("one step");
    };
    assert_eq!(down.node, shadow);
}

#[test]
fn an_extension_past_a_nested_host_keeps_re_entering_its_shadow_tree() {
    // Up from `tail`, the extension's start is kept after the outer host:
    // its last flat position, on the shadow tree's last line. A step up
    // from there reaches the inner host's line, which the DOM tree moves
    // after the outer host again (Chrome 155, `nested.extendUp`).
    let doc = laid_out(
        "<p>aa</p><div><template shadowrootmode=open><p>outer</p><div><template shadowrootmode=open><span>deep</span></template></div><p>after</p></template></div><p>tail</p>",
    );
    let tail = child(&doc, "p:last-of-type", 0);
    let up = (Alter::Extend, BACKWARD, Granularity::Line);
    assert_eq!(
        reported(&doc, (at(tail, 3), at(tail, 3)), &[up; 3]),
        [Some((at(tail, 3), at(tail, 0))); 3]
    );
}

#[test]
fn a_step_that_leaves_nothing_visible_in_the_dom_tree_drops_the_selection() {
    // Lines of `<hr>`s in a shadow tree have no caret position the DOM tree
    // can see: Chrome's selection has no range after a move onto one, and an
    // extension onto one leaves the caret where it was.
    let doc = laid_out(
        "<p>abc</p><div><template shadowrootmode=open><hr><hr></template></div><p>def</p>",
    );
    let def = child(&doc, "p:last-of-type", 0);
    let move_up = (Alter::Move, BACKWARD, Granularity::Line);
    assert_eq!(
        reported(&doc, (at(def, 1), at(def, 1)), &[move_up, move_up]),
        [None, None]
    );
    let extend_up = (Alter::Extend, BACKWARD, Granularity::Line);
    assert_eq!(
        reported(&doc, (at(def, 1), at(def, 1)), &[extend_up, extend_up]),
        [Some((at(def, 1), at(def, 1))); 2]
    );
}

#[test]
fn positions_at_a_shadow_host_are_dom_offsets() {
    // The host has three flat children (its shadow content) but two light
    // children: every result is a valid DOM boundary point.
    let doc = laid_out(
        "<div id=h><template shadowrootmode=open><b>S1</b><slot></slot><i>S2</i></template>light1<br>light2</div>",
    );
    let host = q(&doc, "#h");
    let dom_length = |boundary: blitz_dom::ranges::Boundary| {
        let node = doc.get_node(boundary.node).unwrap();
        node.text_data()
            .map_or(node.children.len(), |t| t.content.to_utf16().len())
    };
    for offset in 0..=3 {
        for direction in [FORWARD, BACKWARD] {
            for granularity in [
                Granularity::Line,
                Granularity::LineBoundary,
                Granularity::Paragraph,
            ] {
                for found in carets(&doc, at(host, offset), &moves(direction, granularity, 2)) {
                    assert!(found.offset <= dom_length(found), "{offset} {found:?}");
                }
            }
        }
    }
}

#[test]
fn steps_keep_the_position_after_an_hr_between_calls() {
    // Chrome keeps a selection's positions between `modify()` calls: from
    // after the second `<hr>` a step up passes the first, where the same
    // boundary point as an offset in the body would stay.
    let doc = laid_out("<p>abc</p><hr><hr><p>def</p>");
    let body = q(&doc, "body");
    let (abc, def) = (child(&doc, "p", 0), child(&doc, "p:last-of-type", 0));
    assert_eq!(
        carets(&doc, at(def, 1), &moves(BACKWARD, Granularity::Line, 3)),
        [at(body, 3), at(body, 2), at(abc, 1)]
    );
    let extend = [(Alter::Extend, BACKWARD, Granularity::Line); 3];
    assert_eq!(
        walk(&doc, (at(def, 1), at(def, 1)), &extend),
        [
            (at(def, 1), at(body, 3)),
            (at(def, 1), at(body, 2)),
            (at(def, 1), at(abc, 1)),
        ]
    );
    let block_images = laid_out(
        "<p>abc</p><img style='display:block;width:20px;height:20px'>\
         <img style='display:block;width:20px;height:20px'><p>def</p>",
    );
    let body = q(&block_images, "body");
    let abc = child(&block_images, "p", 0);
    let def = child(&block_images, "p:last-of-type", 0);
    assert_eq!(
        carets(
            &block_images,
            at(def, 1),
            &moves(BACKWARD, Granularity::Line, 3)
        ),
        [at(body, 3), at(abc, 1), at(abc, 0)]
    );
}

#[test]
fn steps_pass_spaces_removed_before_breaks() {
    // The space ending "ab " before a `<br>` is removed, and the white
    // space of the boxes after it collapses away (Chrome's results).
    let doc = laid_out("<p>ab <span> </span><br>cd <i> </i><b> </b><br>ef</p>");
    let (ab, cd, ef) = (
        child(&doc, "p", 0),
        child(&doc, "p", 3),
        child(&doc, "p", 7),
    );
    let span_space = child(&doc, "span", 0);
    let b_space = child(&doc, "b", 0);
    let steps = [
        (Alter::Move, FORWARD, Granularity::Line),
        (Alter::Move, BACKWARD, Granularity::Line),
        (Alter::Move, FORWARD, Granularity::Character),
    ];
    assert_eq!(
        carets(&doc, at(span_space, 0), &steps),
        [at(cd, 2), at(ab, 2), at(cd, 0)]
    );
    assert_eq!(
        carets(&doc, at(b_space, 0), &moves(BACKWARD, Granularity::Line, 1)),
        [at(ab, 2)]
    );
    let steps = [
        (Alter::Move, BACKWARD, Granularity::Line),
        (Alter::Move, FORWARD, Granularity::Paragraph),
        (Alter::Move, BACKWARD, Granularity::Paragraph),
    ];
    assert_eq!(
        carets(&doc, at(ef, 1), &steps),
        [at(cd, 1), at(ef, 1), at(cd, 1)]
    );
}

#[test]
fn steps_pass_a_button_holding_an_empty_inline() {
    // The empty `<a>` canonicalizes out of the button to "bb", 0. A line
    // step from the inline-block's start reaches its second line, then
    // "bb"'s, and a paragraph step goes straight to "bb"'s end.
    let doc = laid_out(
        "<p><em style='display:inline-block;width:15px'><img width=18 height=7>\
         <button>x<a></a></button></em>bb</p>",
    );
    let (em, bb) = (q(&doc, "em"), child(&doc, "p", 1));
    assert_eq!(
        carets(&doc, at(em, 0), &moves(FORWARD, Granularity::Line, 3)),
        [at(em, 1), at(bb, 2), at(bb, 2)]
    );
    assert_eq!(
        carets(&doc, at(em, 0), &moves(FORWARD, Granularity::Paragraph, 1)),
        [at(bb, 2)]
    );
}

#[test]
fn line_steps_pass_many_forced_breaks() {
    // Parley positions no cluster for these breaks; one walk over the
    // lines finds the line of each, not a walk per break.
    let doc = laid_out(&format!("<p>a{}b</p>", "<br>".repeat(20_000)));
    let p = q(&doc, "p");
    let b = doc.get_node(p).unwrap().children.last().copied().unwrap();
    let moved = carets(&doc, at(b, 1), &moves(BACKWARD, Granularity::Line, 1));
    assert_eq!(moved.len(), 1);
    assert_ne!(moved[0], at(b, 1));
}

#[test]
fn a_kept_position_beside_a_removed_node_becomes_an_offset() {
    // After a step up to after the second `<hr>` (body, 3), removing that
    // `<hr>` leaves (body, 2): Chrome turns the position after the removed
    // node into an offset, which stays put for one step. Removing the first
    // `<hr>` instead keeps the position after the second, which steps on.
    for (removed, expected) in [
        (2, ["BODY:2", "BODY:2", "abc:1"]),
        (1, ["BODY:2", "abc:1", "abc:0"]),
    ] {
        let mut doc = laid_out("<p>abc</p><hr><hr><p>def</p>");
        let body = q(&doc, "body");
        let (abc, def) = (child(&doc, "p", 0), child(&doc, "p:last-of-type", 0));
        let up = ModifyRequest {
            alter: Alter::Move,
            direction: BACKWARD,
            granularity: Granularity::Line,
            directional: true,
            memory: CaretMemory::default(),
        };
        let moved = doc.modify_selection(at(def, 1), at(def, 1), up).unwrap();
        assert_eq!(moved.focus, at(body, 3));
        let hr = doc.get_node(body).unwrap().children[removed];
        doc.mutate().remove_and_drop_node(hr);
        doc.resolve(0.0);
        let name = |point: blitz_dom::ranges::Boundary| match point.node {
            node if node == body => format!("BODY:{}", point.offset),
            node if node == abc => format!("abc:{}", point.offset),
            _ => format!("{point:?}"),
        };
        let (mut focus, mut memory) = (at(body, 2), moved.memory);
        let mut seen = vec![name(focus)];
        for _ in 0..2 {
            let request = ModifyRequest { memory, ..up };
            if let Some(moved) = doc.modify_selection(focus, focus, request) {
                (focus, memory) = (moved.focus, moved.memory);
            }
            seen.push(name(focus));
        }
        assert_eq!(seen, expected, "removed child {removed}");
    }
}
