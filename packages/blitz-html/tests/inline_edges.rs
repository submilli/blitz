//! Horizontal edges of ordinary inline boxes participate in line breaking.

mod common;

use blitz_dom::parley;
use common::q;

#[test]
fn inline_edges_count_when_breaking_lines_beside_floats() {
    // Chrome 154 P1–P4, captured by SUB-1554's independent review.
    for (name, float, edge, tail, expected) in [
        (
            "P1",
            false,
            "padding-right:30px",
            "two three four",
            vec!["one two", "three four"],
        ),
        (
            "P2",
            true,
            "padding-right:30px",
            "two three four five six",
            vec!["one two", "three four five", "six"],
        ),
        (
            "P3",
            true,
            "margin-left:40px",
            "two three four five six",
            vec!["one two", "three four five", "six"],
        ),
        (
            "P4",
            true,
            "border-left:40px solid",
            "two three four five six",
            vec!["one two", "three four five", "six"],
        ),
    ] {
        let float_html = if float {
            "<i style='float:left;width:50px;height:60px'></i>"
        } else {
            ""
        };
        let width = if float { 150 } else { 100 };
        let doc = common::laid_out(&format!(
            "<!doctype html><body style='margin:0;font:16px Arial'><div style='width:{width}px'>{float_html}one<span style='{edge}'> </span>{tail}</div>"
        ));
        let node = doc.get_node(q(&doc, "div")).unwrap();
        let layout = node
            .element_data()
            .unwrap()
            .inline_layout_data
            .as_ref()
            .unwrap();
        let lines: Vec<_> = layout
            .layout
            .lines()
            .map(|line| layout.text[line.text_range()].trim().to_owned())
            .collect();
        assert_eq!(lines, expected, "{name}");
        if float {
            assert_eq!(
                layout
                    .layout
                    .lines()
                    .next()
                    .unwrap()
                    .metrics()
                    .block_min_coord,
                0.0,
                "{name}"
            );
        }
    }
}

#[test]
fn nested_empty_negative_and_fragmented_edges_match_chrome() {
    for (html, expected) in [
        (
            "one <span style='white-space:nowrap;padding:0 20px'>two three</span> four",
            vec!["one", "two three", "four"],
        ),
        (
            "one <span style='padding-left:10px'><span style='padding-right:20px'>two</span> three</span> four",
            vec!["one two", "three four"],
        ),
        (
            "one<span style='padding:0 20px'></span> two three four",
            vec!["one two", "three four"],
        ),
        (
            "one <span style='margin-right:-30px'>two three four</span> five",
            vec!["one two three", "four five"],
        ),
        (
            "one<span style='padding-right:30%'> </span>two three four",
            vec!["one two", "three four"],
        ),
        (
            "<span style='padding:0 20px'>one two three four five six</span>",
            vec!["one two", "three four five", "six"],
        ),
    ] {
        let doc = common::laid_out(&format!(
            "<!doctype html><body style='margin:0;font:16px Arial'><div style='width:100px'>{html}</div>"
        ));
        let node = doc.get_node(q(&doc, "div")).unwrap();
        let layout = node
            .element_data()
            .unwrap()
            .inline_layout_data
            .as_ref()
            .unwrap();
        let lines: Vec<_> = layout
            .layout
            .lines()
            .map(|line| layout.text[line.text_range()].trim())
            .collect();
        assert_eq!(lines, expected, "{html}");
    }
}

#[test]
fn extreme_percentage_edges_keep_layout_finite() {
    for html in [
        "<span style='margin-left:-3e38%;padding-left:3e38%'>abc</span>",
        "<span style='margin-left:3e38%'><span style='padding-left:3e38%'>abc</span></span>",
    ] {
        let doc = common::laid_out(&format!(
            "<!doctype html><body><div style='width:1000px'>{html}</div>"
        ));
        let node = doc.get_node(q(&doc, "div")).unwrap();
        let layout = &node
            .element_data()
            .unwrap()
            .inline_layout_data
            .as_ref()
            .unwrap()
            .layout;
        assert!(layout.lines().count() > 0);
        assert!(
            layout
                .inline_boxes()
                .iter()
                .all(|item| item.width.is_finite())
        );
        for line in layout.lines() {
            assert!(line.metrics().advance.is_finite());
            for item in line.items() {
                if let parley::PositionedLayoutItem::InlineBox(item) = item {
                    assert!(item.x.is_finite() && item.y.is_finite());
                }
            }
        }
    }
}

#[test]
fn span_rectangles_exclude_margins_and_retain_an_empty_font_box() {
    for (html, x, width) in [
        ("a<span style='margin:0 40px'>b</span>", 48.90625, 8.90625),
        ("a<span style='padding:0 20px'></span>", 8.90625, 40.0),
    ] {
        let doc = common::laid_out(&format!(
            "<!doctype html><body style='margin:0;font:16px Arial'><div style='width:200px'>{html}</div>"
        ));
        let rects = doc.inline_fragment_rects(q(&doc, "span")).unwrap();
        assert_eq!(rects.len(), 1);
        assert!((rects[0].x - x).abs() < 0.02, "{html}: {rects:?}");
        assert!((rects[0].width - width).abs() < 0.02, "{html}: {rects:?}");
        assert_eq!(
            (rects[0].y, rects[0].height),
            (0.0, 17.0),
            "{html}: {rects:?}"
        );
    }
}

#[test]
fn atomic_children_wrap_with_their_enclosing_edges() {
    for edge in ["padding-left:40px", "padding-right:30px"] {
        let doc = common::laid_out(&format!(
            "<!doctype html><body style='margin:0;font:16px Arial'><div style='width:100px'>one <span style='{edge}'><i style='display:inline-block;width:40px;height:10px'></i></span></div>"
        ));
        let node = doc.get_node(q(&doc, "div")).unwrap();
        let layout = &node
            .element_data()
            .unwrap()
            .inline_layout_data
            .as_ref()
            .unwrap()
            .layout;
        assert_eq!(layout.lines().count(), 2, "{edge}");
        let rects = doc.inline_fragment_rects(q(&doc, "span")).unwrap();
        assert_eq!(rects.len(), 1, "{edge}: {rects:?}");
        assert_eq!((rects[0].x, rects[0].y), (0.0, 18.0), "{edge}: {rects:?}");
        assert_eq!(
            rects[0].width,
            if edge.contains("left") { 80.0 } else { 70.0 }
        );
    }
}

#[test]
fn min_content_width_keeps_the_opening_edge_with_its_word() {
    let doc = common::laid_out(
        "<!doctype html><body style='margin:0;font:16px Arial'><div style='width:min-content'>a <span style='padding-left:40px'>longword</span></div>",
    );
    let node = doc.get_node(q(&doc, "div")).unwrap();
    let layout = &node
        .element_data()
        .unwrap()
        .inline_layout_data
        .as_ref()
        .unwrap()
        .layout;
    let minimum = layout.calculate_content_widths().min;
    assert!((minimum - 104.9375).abs() < 0.02, "{minimum}");
    // Final intrinsic sizing still rounds up to whole pixels (SUB-1683).
    // The border box must contain the complete decorated min-content segment.
    let width = node.unrounded_layout().size.width;
    assert!(
        width >= minimum && width - minimum < 1.0,
        "{width} / {minimum}"
    );
}
