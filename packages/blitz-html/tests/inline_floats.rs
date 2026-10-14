//! Floats inside inline content, placed as CSS 2.1 §9.5.1 and Chrome do.

mod common;

use blitz_dom::BaseDocument;
use common::{FLOAT, q};

fn laid_out(html: &str) -> BaseDocument {
    common::laid_out(&format!(
        "<!doctype html><html><head></head><body style='margin:0;font-size:16px'>{html}</body></html>"
    ))
}

fn top(doc: &BaseDocument, selector: &str) -> f32 {
    let node = doc.get_node(q(doc, selector)).unwrap();
    node.unrounded_absolute_position(0.0, 0.0).y
}

/// The tops and bottoms of a block's line boxes, in the document.
fn line_boxes(doc: &BaseDocument, selector: &str) -> Vec<(f32, f32)> {
    let node = doc.get_node(q(doc, selector)).unwrap();
    let origin = node.unrounded_absolute_position(0.0, 0.0).y;
    let layout = node
        .element_data()
        .unwrap()
        .inline_layout_data
        .as_ref()
        .unwrap();
    let scale = layout.layout.scale();
    layout
        .layout
        .lines()
        .map(|line| {
            let metrics = line.metrics();
            (
                origin + metrics.block_min_coord / scale,
                origin + metrics.block_max_coord / scale,
            )
        })
        .collect()
}

#[test]
fn a_float_that_does_not_fit_beside_its_line_goes_below_it() {
    // "aaaaa" overflows the 30px block, leaving no room beside it.
    let doc = laid_out(&format!(
        "<p style='width:30px;margin:0'>aaaaa {FLOAT}b</p>"
    ));
    let lines = line_boxes(&doc, "p");
    assert!(
        top(&doc, "img") >= lines[1].0 - 0.01,
        "{} {:?}",
        top(&doc, "img"),
        lines
    );
}

#[test]
fn a_float_that_fits_beside_its_line_stays_on_it() {
    let doc = laid_out(&format!("<p style='width:200px;margin:0'>aa{FLOAT}b</p>"));
    assert_eq!(top(&doc, "img"), top(&doc, "p"));
}

#[test]
fn a_line_too_narrow_beside_a_float_moves_below_it() {
    // Beside the float only 2px remain, too narrow for "bb".
    let doc = laid_out(&format!("<p style='width:12px;margin:0'>{FLOAT}bb</p>"));
    let lines = line_boxes(&doc, "p");
    assert!(lines[0].0 >= 10.0, "{lines:?}");
}

#[test]
fn shrink_to_fit_widths_count_floats() {
    let doc = laid_out(&format!(
        "<p style='margin:0'><span style='display:inline-block'>aa{FLOAT}</span></p>"
    ));
    let span = doc.get_node(q(&doc, "span")).unwrap();
    let alone = laid_out("<p style='margin:0'><span style='display:inline-block'>aa</span></p>");
    let text = alone
        .get_node(q(&alone, "span"))
        .unwrap()
        .final_layout()
        .size
        .width;
    assert_eq!(span.final_layout().size.width, text + 10.0);
}

#[test]
fn lines_moved_below_a_float_grow_their_block() {
    // "bbbbbbbbbbb" does not fit beside the 60px float, so both lines move
    // below it (Chrome: y=30 and 49), and the next block follows them.
    let doc = laid_out(
        "<div id=d style='width:100px'><img style='float:right;width:60px;height:30px'>bbbbbbbbbbb cc</div>\
         <div id=after>after</div>",
    );
    let lines = line_boxes(&doc, "#d");
    assert!(lines[0].0 >= 30.0, "{lines:?}");
    let last = lines.last().unwrap().1;
    assert!(
        top(&doc, "#after") >= last - 0.01,
        "{} {lines:?}",
        top(&doc, "#after")
    );
}

#[test]
fn wrapped_lines_stay_beside_a_float_past_which_their_spaces_hang() {
    // Each wrapped line's trailing space hangs past the 100px beside the
    // float, but its words fit: Chrome keeps all four lines beside it.
    let doc = laid_out(
        "<div style='width:150px'><span style='float:left;width:50px;height:60px'></span>\
         one two three four five six seven eight nine</div>",
    );
    let lines = line_boxes(&doc, "div");
    assert_eq!(lines.len(), 4, "{lines:?}");
    assert_eq!(lines[0].0, 0.0, "{lines:?}");
    assert!(lines.iter().all(|line| line.0 < 60.0), "{lines:?}");
}

#[test]
fn a_float_placed_below_a_line_leaves_the_line_free_to_move() {
    // The left float does not fit beside "bbbbbbbbbbb", which does not fit
    // beside the right float either: Chrome moves the line below the right
    // float, then places the left float below the line.
    let doc = laid_out(
        "<div style='width:100px'><img style='float:right;width:60px;height:30px'>\
         bbbbbbbbbbb<img id=left style='float:left;width:50px;height:10px'></div><div id=after>after</div>",
    );
    let lines = line_boxes(&doc, "div");
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(lines[0].0, 30.0, "{lines:?}");
    assert!(top(&doc, "#left") >= lines[0].1 - 0.01, "{lines:?}");
    assert_eq!(top(&doc, "#after"), top(&doc, "#left"));
}

#[test]
fn a_lone_word_whose_space_hangs_past_a_float_stays_beside_it() {
    // "wwwwwwwwii" fits the 100px beside the float with its space hanging
    // past it, so Chrome 154 keeps it there: lines at 0 and 60.
    let doc = laid_out(
        "<div style='width:150px;font-family:Arial'><span style='float:left;width:50px;height:60px'></span>\
         wwwwwwwwii nextwordthatdoesnotfit</div>",
    );
    let lines = line_boxes(&doc, "div");
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert_eq!(lines[0].0, 0.0, "{lines:?}");
    assert_eq!(lines[1].0, 60.0, "{lines:?}");
}

#[test]
fn an_unbreakable_run_whose_space_hangs_past_a_float_stays_beside_it() {
    // A line of one break segment longer than a word: the no-wrap run fits
    // beside the float with the space after it hanging, so Chrome 154 keeps
    // it there and "five" follows on the next line beside the float (tops 0
    // and 18). In 145px the run does not fit, and both move below the float.
    for (width, beside) in [(150, true), (160, true), (145, false)] {
        let doc = laid_out(&format!(
            "<div style='width:{width}px;font-family:Arial'>\
             <span style='float:left;width:50px;height:60px'></span>\
             <span style='white-space:nowrap'>two three four</span> five</div>"
        ));
        let lines = line_boxes(&doc, "div");
        if beside {
            assert_eq!(lines.len(), 2, "{width}: {lines:?}");
            assert_eq!(lines[0].0, 0.0, "{width}: {lines:?}");
            assert_eq!(lines[1].0, lines[0].1, "{width}: {lines:?}");
        } else {
            assert_eq!(lines.len(), 1, "{width}: {lines:?}");
            assert_eq!(lines[0].0, 60.0, "{width}: {lines:?}");
        }
    }
}

#[test]
fn a_line_whose_preserved_spaces_overflow_beside_a_float_stays_beside_it() {
    // Preserved spaces before the end of the paragraph hang only as far as
    // they overflow (CSS Text 4 §4.3.2), so the line's content measures the
    // 100px beside the float give or take rounding: Chrome 154 keeps it there.
    let doc = laid_out(
        "<div style='width:150px;font-family:Arial;white-space:pre-wrap'>\
         <span style='float:left;width:50px;height:60px'></span>wwwwwwwwii      </div>",
    );
    let lines = line_boxes(&doc, "div");
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(lines[0].0, 0.0, "{lines:?}");
}

#[test]
fn a_taller_adjacent_float_keeps_all_overlapping_lines_beside_it() {
    let doc = laid_out(
        "<div style='width:200px;font-family:Arial'><span style='float:left;width:50px;height:20px'></span>\
         <span style='float:left;width:50px;height:60px'></span>\
         one two three four five six seven eight nine</div>",
    );
    let lines = line_boxes(&doc, "div");
    assert_eq!(lines.len(), 4, "{lines:?}");
    assert_eq!(
        lines.iter().map(|line| line.0).collect::<Vec<_>>(),
        vec![0.0, 18.0, 36.0, 54.0]
    );
}
