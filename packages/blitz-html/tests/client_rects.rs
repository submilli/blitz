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

#[test]
fn a_line_beside_a_float_keeps_its_last_inlines_width() {
    // The line starts past the float, and the inline ending it is as wide
    // as its text: 8.91px beside a 20px float in Chrome 154, on either side.
    for (dir, side) in [("ltr", "left"), ("rtl", "right")] {
        let html = format!(
            "<div dir={dir} style='font-family:Arial;font-size:16px'><p style='margin:0'>\
             <span style='float:{side};display:block;width:20px;height:40px'></span><i>b</i></p></div>"
        );
        let float = rects(&html, "span")[0];
        let text = rects(&html, "i");
        assert_eq!(text.len(), 1, "{dir}: {text:?}");
        let (x, width) = text[0];
        assert!((width - 8.91).abs() < 0.01, "{dir}: {text:?}");
        // Liberation Sans advances differ from Arial's by hundredths.
        let expected = if dir == "ltr" {
            float.0 + 20.0
        } else {
            float.0 - 8.91
        };
        assert!(
            (x - expected).abs() < 0.05,
            "{dir}: {text:?}, float {float:?}"
        );
    }
}

#[test]
fn a_justified_lines_last_inline_reaches_its_end() {
    // Justification widens the line's spaces, so "jj" ends the second line
    // at the 100px edge: 7.11px wide at 92.89 in Chrome 154, in either
    // direction.
    for dir in ["ltr", "rtl"] {
        let html = format!(
            "<div dir={dir} style='width:100px;text-align:justify;font-family:Arial;font-size:16px'>\
             aa bb cc dd ee ff gg hh ii <span>jj</span> kk ll mm nn oo</div>"
        );
        let rects = rects(&html, "span");
        assert_eq!(rects.len(), 1, "{dir}: {rects:?}");
        let (x, width) = rects[0];
        assert!((width - 7.11).abs() < 0.05, "{dir}: {rects:?}");
        assert!((x - 92.89).abs() < 0.05, "{dir}: {rects:?}");
    }
}

#[test]
fn an_inline_returns_to_its_line_when_its_parent_stops_being_flex_or_grid() {
    // As a flex or grid item the span had a box of its own; back in its
    // parent's line it reports its text's rects, as before the change.
    for (tag, display) in [("p", "flex"), ("div", "grid"), ("button", "flex")] {
        let html = format!("<{tag} id=t>aa <span>bb</span> cc</{tag}>");
        let expected = rects(&html, "span");
        let mut doc = laid_out(&format!(
            "<!doctype html><html><head></head><body style='margin:0'>{html}</body></html>"
        ));
        let target = q(&doc, "#t");
        for style in [format!("display:{display}"), String::new()] {
            let style_attr = blitz_dom::QualName::new(None, blitz_dom::ns!(), "style".into());
            doc.mutate().set_attribute(target, style_attr, &style);
            doc.resolve(0.0);
        }
        let ours: Vec<_> = doc
            .node_client_rects(q(&doc, "span"))
            .iter()
            .map(|rect| {
                (
                    (rect.x * 100.0).round() / 100.0,
                    (rect.width * 100.0).round() / 100.0,
                )
            })
            .collect();
        assert_eq!(ours, expected, "{tag} {display}");
    }
}
