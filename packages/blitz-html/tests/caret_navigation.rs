//! `Selection.modify` caret movement over laid-out content. Expected
//! positions were measured in Chrome 154 on macOS.

mod common;

use blitz_dom::caret::{Alter, Direction, Granularity, ModifyRequest};
use blitz_dom::{BaseDocument, NodeId, ranges::Boundary};
use common::{parse, q};

const FORWARD: Direction = Direction::Forward;
const BACKWARD: Direction = Direction::Backward;
const FLOAT: &str = "<img style='float:left;width:10px;height:10px'>";

fn laid_out(html: &str) -> BaseDocument {
    let mut doc = parse(html);
    *doc.font_ctx().lock().unwrap() = blitz_dom::build_single_font_ctx(include_bytes!(
        "../../blitz-dom/assets/test-fonts/LiberationSans-Regular.ttf"
    ));
    doc.resolve(0.0);
    doc
}

fn child(doc: &BaseDocument, selector: &str, index: usize) -> NodeId {
    doc.get_node(q(doc, selector)).unwrap().children[index]
}

fn at(node: NodeId, offset: usize) -> Boundary {
    Boundary { node, offset }
}

/// Apply steps to a selection, collecting each resulting (anchor, focus).
fn walk(
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

fn carets(
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

fn moves(
    direction: Direction,
    granularity: Granularity,
    count: usize,
) -> Vec<(Alter, Direction, Granularity)> {
    vec![(Alter::Move, direction, granularity); count]
}

#[test]
fn characters_cross_blocks_and_prefer_the_end_of_a_preceding_text_node() {
    let doc = laid_out("<p>abc</p><p>def</p>");
    let (abc, def) = (child(&doc, "p", 0), child(&doc, "p + p", 0));
    let steps = [
        moves(FORWARD, Granularity::Character, 1),
        moves(BACKWARD, Granularity::Character, 2),
    ]
    .concat();
    assert_eq!(
        carets(&doc, at(abc, 3), &steps),
        [at(def, 0), at(abc, 3), at(abc, 2)]
    );

    let doc = laid_out("<p><b>abc</b>def</p>");
    let (abc, def) = (child(&doc, "b", 0), child(&doc, "p", 1));
    let steps = [
        moves(FORWARD, Granularity::Character, 2),
        moves(BACKWARD, Granularity::Character, 2),
    ]
    .concat();
    assert_eq!(
        carets(&doc, at(abc, 2), &steps),
        [at(abc, 3), at(def, 1), at(abc, 3), at(abc, 2)]
    );
}

#[test]
fn collapsed_hidden_and_generated_content_take_no_caret_positions() {
    let doc = laid_out("<p>a   b</p><p>   ab</p><p>ab </p><p>cd</p>");
    let spaced = child(&doc, "p", 0);
    let steps = [
        moves(FORWARD, Granularity::Character, 2),
        moves(BACKWARD, Granularity::Character, 1),
    ]
    .concat();
    assert_eq!(
        carets(&doc, at(spaced, 0), &steps),
        [at(spaced, 1), at(spaced, 2), at(spaced, 1)]
    );
    let leading = child(&doc, "p:nth-child(2)", 0);
    assert_eq!(
        carets(
            &doc,
            at(leading, 0),
            &moves(FORWARD, Granularity::Character, 1)
        ),
        [at(leading, 4)]
    );
    let (trailing, next) = (
        child(&doc, "p:nth-child(3)", 0),
        child(&doc, "p:nth-child(4)", 0),
    );
    let steps = [
        moves(FORWARD, Granularity::Character, 1),
        moves(BACKWARD, Granularity::Character, 1),
    ]
    .concat();
    assert_eq!(
        carets(&doc, at(trailing, 2), &steps),
        [at(next, 0), at(trailing, 2)]
    );

    for style in ["display:none", "visibility:hidden"] {
        let doc = laid_out(&format!("<p>ab<span style='{style}'>XYZ</span>cd</p>"));
        let (ab, cd) = (child(&doc, "p", 0), child(&doc, "p", 2));
        let steps = [
            moves(FORWARD, Granularity::Character, 1),
            moves(BACKWARD, Granularity::Character, 1),
        ]
        .concat();
        assert_eq!(
            carets(&doc, at(ab, 2), &steps),
            [at(cd, 1), at(ab, 2)],
            "{style}"
        );
    }

    let doc =
        laid_out("<style>q::before{content:'XX'}q::after{content:'YY'}</style><p><q>ab</q>cd</p>");
    let (ab, cd) = (child(&doc, "q", 0), child(&doc, "p", 1));
    let steps = [
        moves(BACKWARD, Granularity::Character, 1),
        moves(FORWARD, Granularity::Character, 3),
    ]
    .concat();
    assert_eq!(
        carets(&doc, at(ab, 0), &steps),
        [at(ab, 0), at(ab, 1), at(ab, 2), at(cd, 1)]
    );
}

#[test]
fn atoms_breaks_and_inline_blocks_are_stepped_like_chrome() {
    let doc = laid_out("<p>ab<img style='width:10px;height:10px'>cd</p>");
    let (p, ab, cd) = (q(&doc, "p"), child(&doc, "p", 0), child(&doc, "p", 2));
    let steps = [
        moves(FORWARD, Granularity::Character, 2),
        moves(BACKWARD, Granularity::Character, 2),
    ]
    .concat();
    assert_eq!(
        carets(&doc, at(ab, 2), &steps),
        [at(p, 2), at(cd, 1), at(p, 2), at(ab, 2)]
    );

    let doc = laid_out("<p>a<br><br>b</p>");
    let (p, a, b) = (q(&doc, "p"), child(&doc, "p", 0), child(&doc, "p", 3));
    let steps = moves(FORWARD, Granularity::Character, 2);
    assert_eq!(carets(&doc, at(a, 1), &steps), [at(p, 2), at(b, 0)]);

    let doc = laid_out("<p>ab<span style='display:inline-block'>cd</span>ef</p>");
    let (ab, cd, ef) = (
        child(&doc, "p", 0),
        child(&doc, "span", 0),
        child(&doc, "p", 2),
    );
    let steps = moves(FORWARD, Granularity::Character, 4);
    assert_eq!(
        carets(&doc, at(ab, 2), &steps),
        [at(cd, 1), at(cd, 2), at(ef, 1), at(ef, 2)]
    );

    let doc = laid_out("<p>ab<span style='user-select:none'>XY</span>cd</p>");
    let (ab, cd) = (child(&doc, "p", 0), child(&doc, "p", 2));
    assert_eq!(
        carets(&doc, at(ab, 2), &moves(FORWARD, Granularity::Character, 2)),
        [at(cd, 0), at(cd, 1)]
    );
}

#[test]
fn grapheme_clusters_are_single_steps() {
    let doc = laid_out("<p>e\u{301}x\u{1F600}y\u{1F1EF}\u{1F1F5}z</p>");
    let t = child(&doc, "p", 0);
    let steps = [
        moves(FORWARD, Granularity::Character, 5),
        moves(BACKWARD, Granularity::Character, 2),
    ]
    .concat();
    let offsets: Vec<_> = carets(&doc, at(t, 0), &steps)
        .iter()
        .map(|b| b.offset)
        .collect();
    assert_eq!(offsets, [2, 3, 5, 6, 10, 6, 5]);
}

#[test]
fn words_and_sentences_follow_icu_segments() {
    let doc = laid_out("<p>hello world, foo</p>");
    let t = child(&doc, "p", 0);
    let steps = [
        moves(FORWARD, Granularity::Word, 4),
        moves(BACKWARD, Granularity::Word, 4),
    ]
    .concat();
    let offsets: Vec<_> = carets(&doc, at(t, 0), &steps)
        .iter()
        .map(|b| b.offset)
        .collect();
    assert_eq!(offsets, [5, 11, 12, 16, 13, 11, 6, 0]);

    let doc = laid_out("<p>One two. Three four. Five.</p>");
    let t = child(&doc, "p", 0);
    let steps = [
        (Alter::Move, FORWARD, Granularity::Sentence),
        (Alter::Move, FORWARD, Granularity::SentenceBoundary),
        (Alter::Move, BACKWARD, Granularity::SentenceBoundary),
        (Alter::Move, BACKWARD, Granularity::Sentence),
    ];
    let offsets: Vec<_> = carets(&doc, at(t, 1), &steps)
        .iter()
        .map(|b| b.offset)
        .collect();
    assert_eq!(offsets, [9, 21, 9, 0]);
}

#[test]
fn lines_keep_their_horizontal_position_and_end_at_the_document_edges() {
    let doc = laid_out("<p>abcd<br>efgh<br>ij</p>");
    let (abcd, efgh, ij) = (
        child(&doc, "p", 0),
        child(&doc, "p", 2),
        child(&doc, "p", 4),
    );
    let steps = [
        moves(FORWARD, Granularity::Line, 3),
        moves(BACKWARD, Granularity::Line, 2),
    ]
    .concat();
    assert_eq!(
        carets(&doc, at(abcd, 0), &steps),
        [at(efgh, 0), at(ij, 0), at(ij, 2), at(efgh, 0), at(abcd, 0)]
    );

    let doc = laid_out("<p>abc</p><p>def</p><p>ghi</p>");
    let (abc, def, ghi) = (
        child(&doc, "p", 0),
        child(&doc, "p + p", 0),
        child(&doc, "p + p + p", 0),
    );
    let steps = [
        moves(FORWARD, Granularity::Line, 2),
        moves(BACKWARD, Granularity::Line, 1),
    ]
    .concat();
    assert_eq!(
        carets(&doc, at(abc, 0), &steps),
        [at(def, 0), at(ghi, 0), at(def, 0)]
    );

    let doc = laid_out("<p>ab cd<br>ef</p>");
    let t = child(&doc, "p", 0);
    let steps = [
        (Alter::Move, FORWARD, Granularity::LineBoundary),
        (Alter::Move, FORWARD, Granularity::LineBoundary),
        (Alter::Move, BACKWARD, Granularity::LineBoundary),
        (Alter::Extend, FORWARD, Granularity::LineBoundary),
    ];
    assert_eq!(
        walk(&doc, (at(t, 1), at(t, 1)), &steps),
        [
            (at(t, 5), at(t, 5)),
            (at(t, 5), at(t, 5)),
            (at(t, 0), at(t, 0)),
            (at(t, 0), at(t, 5))
        ]
    );
}

#[test]
fn extension_keeps_the_anchor_side_and_boundaries_grow_the_selection() {
    let doc = laid_out("<p>one two three</p>");
    let t = child(&doc, "p", 0);
    let steps = [
        (Alter::Extend, FORWARD, Granularity::Word),
        (Alter::Extend, BACKWARD, Granularity::Word),
        (Alter::Extend, BACKWARD, Granularity::Word),
    ];
    assert_eq!(
        walk(&doc, (at(t, 5), at(t, 5)), &steps),
        [
            (at(t, 5), at(t, 7)),
            (at(t, 5), at(t, 5)),
            (at(t, 5), at(t, 4))
        ]
    );

    let doc = laid_out("<p>ab cd</p>");
    let t = child(&doc, "p", 0);
    let steps = [(Alter::Extend, BACKWARD, Granularity::LineBoundary)];
    assert_eq!(
        walk(&doc, (at(t, 2), at(t, 4)), &steps),
        [(at(t, 0), at(t, 4))]
    );

    let doc = laid_out("<p>abcdef</p>");
    let t = child(&doc, "p", 0);
    let collapse = [(Alter::Move, FORWARD, Granularity::Character)];
    assert_eq!(
        walk(&doc, (at(t, 4), at(t, 1)), &collapse),
        [(at(t, 4), at(t, 4))]
    );
    let word = [(Alter::Move, BACKWARD, Granularity::Word)];
    let doc = laid_out("<p>one two three</p>");
    let t = child(&doc, "p", 0);
    assert_eq!(
        walk(&doc, (at(t, 5), at(t, 9)), &word),
        [(at(t, 8), at(t, 8))]
    );
}

#[test]
fn element_positions_and_empty_documents() {
    let doc = laid_out("<p>ab</p><div>cd<p>ef</p></div>");
    let (body, ab, ef) = (
        q(&doc, "body"),
        child(&doc, "p", 0),
        child(&doc, "div p", 0),
    );
    let cd = child(&doc, "div", 0);
    let steps = [
        (Alter::Move, FORWARD, Granularity::DocumentBoundary),
        (Alter::Move, BACKWARD, Granularity::DocumentBoundary),
    ];
    assert_eq!(carets(&doc, at(cd, 1), &steps), [at(ef, 2), at(ab, 0)]);
    let steps = [(Alter::Move, FORWARD, Granularity::Character)];
    assert_eq!(carets(&doc, at(body, 1), &steps), [at(cd, 1)]);

    let doc = laid_out("<body></body>");
    let body = q(&doc, "body");
    let request = ModifyRequest {
        alter: Alter::Move,
        direction: FORWARD,
        granularity: Granularity::Character,
        line_x: None,
        upstream: false,
    };
    assert_eq!(
        doc.modify_selection(at(body, 0), at(body, 0), request),
        None
    );
}

#[test]
fn right_to_left_paragraphs_map_left_to_logical_forward() {
    let doc = laid_out("<p dir=rtl>\u{5D0}\u{5D1}\u{5D2}</p>");
    let t = child(&doc, "p", 0);
    let steps = [
        (Alter::Move, FORWARD, Granularity::Character),
        (Alter::Move, Direction::Left, Granularity::Character),
        (Alter::Move, Direction::Right, Granularity::Character),
        (Alter::Move, Direction::Right, Granularity::Character),
    ];
    let offsets: Vec<_> = carets(&doc, at(t, 1), &steps)
        .iter()
        .map(|b| b.offset)
        .collect();
    assert_eq!(offsets, [2, 3, 2, 1]);
}

#[test]
fn deeply_nested_inline_content_is_navigable() {
    let depth = 300;
    let html = format!(
        "<p>{}x{}</p>",
        "<span>".repeat(depth),
        "</span>".repeat(depth)
    );
    let doc = laid_out(&html);
    let p = q(&doc, "p");
    let request = ModifyRequest {
        alter: Alter::Move,
        direction: FORWARD,
        granularity: Granularity::DocumentBoundary,
        line_x: None,
        upstream: false,
    };
    let moved = doc.modify_selection(at(p, 0), at(p, 0), request).unwrap();
    assert_eq!(moved.focus.offset, 1);
}

#[test]
fn empty_lines_between_breaks_are_line_stops() {
    let doc = laid_out("<p>a<br><br>b</p>");
    let (p, b) = (q(&doc, "p"), child(&doc, "p", 3));
    let steps = [
        moves(BACKWARD, Granularity::Line, 1),
        moves(FORWARD, Granularity::Line, 2),
    ]
    .concat();
    assert_eq!(
        carets(&doc, at(b, 0), &steps),
        [at(p, 2), at(b, 0), at(b, 1)]
    );
}

#[test]
fn text_controls_move_over_their_editor_layout() {
    use blitz_dom::form_selection::{ControlSelection, SelectionDirection};
    let mut doc = laid_out("<input value='abc def'><textarea>ab\ncd</textarea>");
    let (input, area) = (q(&doc, "input"), q(&doc, "textarea"));
    let caret = |offset| ControlSelection {
        start: offset,
        end: offset,
        direction: SelectionDirection::None,
    };
    let run = |doc: &mut BaseDocument, id, start, steps: &[(Alter, Direction, Granularity)]| {
        doc.mutate().set_control_selection(id, caret(start));
        let (mut line_x, mut upstream) = (None, false);
        let mut out = Vec::new();
        for &(alter, direction, granularity) in steps {
            doc.resolve(0.0);
            let request = ModifyRequest {
                alter,
                direction,
                granularity,
                line_x,
                upstream,
            };
            let moved = doc.mutate().modify_control_selection(id, request).unwrap();
            (line_x, upstream) = (moved.line_x, moved.upstream);
            let s = doc.control_selection(id).unwrap();
            out.push((s.start, s.end, s.direction));
        }
        out
    };
    use SelectionDirection as D;
    let steps = [
        (Alter::Move, FORWARD, Granularity::Character),
        (Alter::Extend, FORWARD, Granularity::Word),
        (Alter::Move, BACKWARD, Granularity::Character),
        (Alter::Move, FORWARD, Granularity::LineBoundary),
        (Alter::Move, BACKWARD, Granularity::DocumentBoundary),
    ];
    assert_eq!(
        run(&mut doc, input, 1, &steps),
        [
            (2, 2, D::None),
            (2, 3, D::Forward),
            (2, 2, D::None),
            (7, 7, D::None),
            (0, 0, D::None)
        ]
    );
    let steps = [
        (Alter::Move, FORWARD, Granularity::Line),
        (Alter::Extend, BACKWARD, Granularity::Character),
        (Alter::Move, FORWARD, Granularity::LineBoundary),
    ];
    assert_eq!(
        run(&mut doc, area, 1, &steps),
        [(4, 4, D::None), (3, 4, D::Backward), (5, 5, D::None)]
    );
}

#[test]
fn source_newlines_before_breaks_collapse_and_preserved_newlines_break() {
    let doc = laid_out("<p>ab\n<br>cd</p>");
    let (ab, cd) = (child(&doc, "p", 0), child(&doc, "p", 2));
    let steps = [
        (Alter::Move, FORWARD, Granularity::Character),
        (Alter::Move, FORWARD, Granularity::LineBoundary),
    ];
    assert_eq!(carets(&doc, at(ab, 2), &steps), [at(cd, 0), at(cd, 2)]);

    let doc = laid_out("<pre>ab\ncd\nef</pre>");
    let t = child(&doc, "pre", 0);
    let steps = [
        (Alter::Move, FORWARD, Granularity::LineBoundary),
        (Alter::Move, FORWARD, Granularity::LineBoundary),
        (Alter::Move, FORWARD, Granularity::Character),
        (Alter::Move, FORWARD, Granularity::ParagraphBoundary),
        (Alter::Move, BACKWARD, Granularity::Paragraph),
    ];
    let offsets: Vec<_> = carets(&doc, at(t, 1), &steps)
        .iter()
        .map(|b| b.offset)
        .collect();
    assert_eq!(offsets, [2, 2, 3, 5, 2]);
}

#[test]
fn transformed_generated_content_and_final_sigma_stay_aligned() {
    let doc = laid_out("<style>p{text-transform:uppercase}p::before{content:'x'}</style><p>ab</p>");
    let t = child(&doc, "p", 0);
    let steps = moves(FORWARD, Granularity::Character, 2);
    assert_eq!(carets(&doc, at(t, 0), &steps), [at(t, 1), at(t, 2)]);

    let doc = laid_out(
        "<p style='text-transform:lowercase'>\u{39F}\u{394}\u{39F}\u{3A3} \u{391}\u{392}</p>",
    );
    let t = child(&doc, "p", 0);
    let offsets: Vec<_> = carets(&doc, at(t, 0), &moves(FORWARD, Granularity::Word, 2))
        .iter()
        .map(|b| b.offset)
        .collect();
    assert_eq!(offsets, [4, 7]);
}

#[test]
fn embedded_boxes_are_navigated_in_tree_order() {
    let doc = laid_out("<p>ab<span style='display:inline-flex'>cd</span>ef</p>");
    let (ab, cd, ef) = (
        child(&doc, "p", 0),
        child(&doc, "span", 0),
        child(&doc, "p", 2),
    );
    assert_eq!(
        carets(&doc, at(ab, 2), &moves(FORWARD, Granularity::Character, 3)),
        [at(cd, 1), at(cd, 2), at(ef, 1)]
    );

    let doc = laid_out("<div>ab<span style='display:inline-block'><div>cd</div></span>ef</div>");
    let (ab, cd, ef) = (
        child(&doc, "div", 0),
        child(&doc, "span div", 0),
        child(&doc, "div", 2),
    );
    assert_eq!(
        carets(&doc, at(ab, 2), &moves(FORWARD, Granularity::Character, 4)),
        [at(cd, 0), at(cd, 1), at(cd, 2), at(ef, 0)]
    );

    let doc = laid_out("<p>ab<span style='position:absolute;left:300px'>XY</span>cd</p>");
    let (xy, cd) = (child(&doc, "span", 0), child(&doc, "p", 2));
    assert_eq!(
        carets(&doc, at(xy, 0), &moves(FORWARD, Granularity::Character, 3)),
        [at(xy, 1), at(xy, 2), at(cd, 0)]
    );

    let doc = laid_out(
        "<div style='display:flex'><p style='order:2'>ab</p><p style='order:1'>cd</p></div>",
    );
    let (ab, cd) = (child(&doc, "p", 0), child(&doc, "p + p", 0));
    assert_eq!(
        carets(&doc, at(ab, 1), &moves(FORWARD, Granularity::Character, 3)),
        [at(ab, 2), at(cd, 0), at(cd, 1)]
    );
}

#[test]
fn soft_wrapped_lines_end_before_their_hanging_space() {
    let doc = laid_out("<p style='width:1px'>abc def ghi</p>");
    let t = child(&doc, "p", 0);
    let steps = [
        (Alter::Move, FORWARD, Granularity::LineBoundary),
        (Alter::Move, FORWARD, Granularity::LineBoundary),
        (Alter::Move, FORWARD, Granularity::Character),
        (Alter::Move, FORWARD, Granularity::Line),
        (Alter::Move, BACKWARD, Granularity::LineBoundary),
        (Alter::Move, FORWARD, Granularity::Line),
    ];
    let offsets: Vec<_> = carets(&doc, at(t, 1), &steps)
        .iter()
        .map(|b| b.offset)
        .collect();
    assert_eq!(offsets, [3, 3, 4, 8, 8, 11]);
}

#[test]
fn a_combining_mark_after_an_atom_starts_its_own_unit() {
    let doc = laid_out("<p>a<img style='width:10px;height:10px'>\u{301}b</p>");
    let (p, mark) = (q(&doc, "p"), child(&doc, "p", 2));
    let steps = moves(FORWARD, Granularity::Character, 2);
    let moved = carets(&doc, at(p, 1), &steps);
    assert_eq!(moved[0], at(p, 2));
    assert_eq!(moved[1].node, mark);
}

/// A combining mark on collapsible white space once broke the entry order
/// and indexed past the stops.
#[test]
fn combining_marks_on_collapsed_spaces_do_not_break_the_flow() {
    for html in [
        "<p>a  \u{301}</p><p>x</p>",
        "<p>a <span>\u{301}</span></p><p>x</p>",
    ] {
        let doc = laid_out(html);
        let body = q(&doc, "body");
        let x = child(&doc, "p + p", 0);
        let steps = [
            (Alter::Move, FORWARD, Granularity::DocumentBoundary),
            (Alter::Move, BACKWARD, Granularity::Character),
        ];
        let moved = carets(&doc, at(body, 0), &steps);
        assert_eq!(moved[0], at(x, 1), "{html}");
    }
}

#[test]
fn line_ends_at_soft_wraps_keep_upstream_affinity() {
    let doc = laid_out("<p style='width:1px;word-break:break-all'>abc</p>");
    let t = child(&doc, "p", 0);
    let steps = [
        (Alter::Move, FORWARD, Granularity::LineBoundary),
        (Alter::Move, FORWARD, Granularity::LineBoundary),
        (Alter::Move, FORWARD, Granularity::Line),
        (Alter::Move, BACKWARD, Granularity::LineBoundary),
    ];
    let offsets: Vec<_> = carets(&doc, at(t, 0), &steps)
        .iter()
        .map(|b| b.offset)
        .collect();
    assert_eq!(offsets, [1, 1, 2, 1]);
}

#[test]
fn element_ends_inside_a_root_canonicalize_upstream() {
    let doc = laid_out("<div><p>ab</p>cd</div>");
    let (p, ab, cd) = (q(&doc, "p"), child(&doc, "p", 0), child(&doc, "div", 1));
    let steps = [
        (Alter::Move, FORWARD, Granularity::Character),
        (Alter::Move, BACKWARD, Granularity::Character),
    ];
    assert_eq!(carets(&doc, at(p, 1), &steps), [at(cd, 0), at(ab, 2)]);

    let doc = laid_out("<p>ab<span style='float:left'>XY</span>cd</p>");
    let (p, xy) = (q(&doc, "p"), child(&doc, "span", 0));
    assert_eq!(
        carets(&doc, at(p, 1), &moves(FORWARD, Granularity::Character, 1)),
        [at(xy, 0)]
    );
}

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
fn used_user_select_none_is_inherited_and_applies_to_inline_blocks() {
    let doc = laid_out("<div style='user-select:none'><p>ab</p></div><p>cd</p>");
    let cd = child(&doc, "div + p", 0);
    assert_eq!(
        carets(&doc, at(cd, 0), &moves(BACKWARD, Granularity::Character, 1)),
        [at(cd, 0)]
    );

    let doc = laid_out("<p>ab<span style='display:inline-block;user-select:none'>XY</span>cd</p>");
    let (ab, cd) = (child(&doc, "p", 0), child(&doc, "p", 2));
    assert_eq!(
        carets(&doc, at(ab, 2), &moves(FORWARD, Granularity::Character, 2)),
        [at(cd, 0), at(cd, 1)]
    );
}

#[test]
fn paragraphs_after_a_trailing_break_and_preserved_controls() {
    let doc = laid_out("<p>ab<br></p><p>cd</p>");
    let (ab, cd) = (child(&doc, "p", 0), child(&doc, "p + p", 0));
    let steps = [
        (Alter::Move, FORWARD, Granularity::Paragraph),
        (Alter::Move, BACKWARD, Granularity::Paragraph),
    ];
    assert_eq!(carets(&doc, at(ab, 1), &steps), [at(cd, 1), at(ab, 1)]);

    let doc = laid_out("<div style='white-space:pre-line'>a\r\nb c\u{c}d</div>");
    let t = child(&doc, "div", 0);
    let offsets: Vec<_> = carets(&doc, at(t, 0), &moves(FORWARD, Granularity::Character, 6))
        .iter()
        .map(|b| b.offset)
        .collect();
    assert_eq!(offsets, [1, 2, 3, 4, 5, 6]);
}

#[test]
fn a_space_before_generated_content_does_not_hang() {
    let doc = laid_out("<style>q::after{content:'!'}</style><p><q>ab </q><br>cd</p>");
    let ab = child(&doc, "q", 0);
    let moved = carets(
        &doc,
        at(ab, 0),
        &moves(FORWARD, Granularity::LineBoundary, 1),
    );
    assert_eq!(moved, [at(ab, 3)]);
}

#[test]
fn extension_stays_in_the_anchor_tree() {
    let doc = laid_out(
        "<div id=host><template shadowrootmode=open><span>shadow</span></template></div><p>tail</p>",
    );
    let host = q(&doc, "#host");
    let root = doc.shadow_root_of(host).expect("declarative shadow root");
    let span = doc.get_node(root).unwrap().children[0];
    let shadow = doc.get_node(span).unwrap().children[0];
    let tail = child(&doc, "p", 0);
    let forward = [(Alter::Extend, FORWARD, Granularity::DocumentBoundary)];
    assert_eq!(
        walk(&doc, (at(shadow, 1), at(shadow, 1)), &forward),
        [(at(shadow, 1), at(shadow, 6))]
    );
    let backward = [(Alter::Extend, BACKWARD, Granularity::DocumentBoundary)];
    assert_eq!(
        walk(&doc, (at(tail, 2), at(tail, 2)), &backward),
        [(at(tail, 2), at(tail, 0))]
    );
}

#[test]
fn flows_without_stops_neither_stall_line_ends_nor_sentences() {
    let doc = laid_out(
        "<div>ab<span style='display:inline-block'><div>cd</div></span><span style='user-select:none'>XY</span></div>",
    );
    let ab = child(&doc, "div", 0);
    let moved = carets(
        &doc,
        at(ab, 0),
        &moves(FORWARD, Granularity::LineBoundary, 1),
    );
    assert_eq!(moved, [at(ab, 2)]);

    let doc = laid_out("<p>Hi. <span style='user-select:none'>X</span></p><p>Next.</p>");
    let (hi, next) = (child(&doc, "p", 0), child(&doc, "p + p", 0));
    let moved = carets(&doc, at(hi, 0), &moves(FORWARD, Granularity::Sentence, 2));
    assert_eq!(moved[1].node, next);
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
fn stored_upstream_affinity_applies_only_at_a_soft_wrap() {
    let doc = laid_out("<p>ab<br>cd<br>ef</p>");
    let (cd, ef) = (child(&doc, "p", 2), child(&doc, "p", 4));
    let request = ModifyRequest {
        alter: Alter::Move,
        direction: FORWARD,
        granularity: Granularity::Line,
        line_x: None,
        upstream: true,
    };
    let moved = doc.modify_selection(at(cd, 0), at(cd, 0), request).unwrap();
    assert_eq!(moved.focus, at(ef, 0));
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
