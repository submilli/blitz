//! Client rectangles of inline content (CSSOM View §6), against Chrome 154.

mod common;

use common::{laid_out, q};

/// The `(x, width)` of each of the client rects of the element `selector`.
fn rects(html: &str, selector: &str) -> Vec<(f64, f64)> {
    let doc = laid_out(&format!(
        "<!doctype html><html><head></head><body style='margin:0'>{html}</body></html>"
    ));
    doc.node_client_rects(q(&doc, selector))
        .iter()
        .map(|rect| {
            (
                (rect.x * 100.0).round() / 100.0,
                (rect.width * 100.0).round() / 100.0,
            )
        })
        .collect()
}

#[test]
fn a_wrapped_line_excludes_its_hanging_space_at_the_paragraphs_end_edge() {
    // The space ending the first line hangs at the line's end edge: the
    // right in a left-to-right paragraph, the left in a right-to-left one,
    // where it takes the paragraph's direction (UAX #9 L1) though the words
    // are left-to-right. Chrome's first rect is 117.42px wide in both.
    let text = "<span>aaa bbb ccc ddd eee fff</span>";
    for (dir, x) in [("ltr", 0.0), ("rtl", 2.58)] {
        let html = format!(
            "<div dir={dir} style='width:120px;font-family:Arial;font-size:16px'>{text}</div>"
        );
        let first = rects(&html, "span")[0];
        assert!((first.0 - x).abs() < 0.01, "{dir}: {first:?}");
        assert!((first.1 - 117.42).abs() < 0.01, "{dir}: {first:?}");
    }
}

#[test]
fn a_wrapped_right_to_left_word_starts_at_a_left_to_right_lines_start() {
    // The space after the first Hebrew word ends its line and hangs at the
    // right, so the word's rect starts at the line's left edge, as in
    // Chrome 154.
    let html =
        "<div style='width:40px;font-family:Arial;font-size:16px'><span>אבג אבג</span></div>";
    let rects = rects(html, "span");
    assert_eq!(rects.len(), 2, "{rects:?}");
    assert!(rects.iter().all(|rect| rect.0 == 0.0), "{rects:?}");
}
