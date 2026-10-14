//! Inside list markers against Chrome 155 (macOS), in
//! `fixtures/list_markers.tsv`: where the content after the marker starts.
//! Chrome sizes a symbol marker (`disc`, `circle`, `square`) from the font's
//! ascent, with a 1em end margin; other markers are text.

mod common;

use blitz_dom::BaseDocument;
use common::{laid_out_at_scale, q};

/// Cases that still differ from Chrome, with our positions.
const KNOWN_DIFFERENCES: &[(&str, &str)] = &[
    // Chrome lays out a disclosure marker as text in a fallback font of the
    // system, as Arial has no triangles; Blitz uses its bundled bullet font,
    // whose taller ascent also lowers the line (submilli SUB-1706).
    ("disclosure-open@7", "6.17,0"),
    ("disclosure-closed@7", "6.17,0"),
    ("disclosure-open@10", "8.81,0"),
    ("disclosure-closed@10", "8.81,0"),
    ("disclosure-open@13", "11.45,0"),
    ("disclosure-closed@13", "11.45,0"),
    ("disclosure-open@13.3", "11.72,0"),
    ("disclosure-closed@13.3", "11.72,0"),
    ("disclosure-open@16", "14.09,1"),
    ("disclosure-closed@16", "14.09,1"),
    ("disclosure-open@20", "17.63,1"),
    ("disclosure-closed@20", "17.63,1"),
    ("disclosure-open@32", "28.19,2"),
    ("disclosure-closed@32", "28.19,2"),
    ("disclosure-open@50", "44.05,3"),
    ("disclosure-closed@50", "44.05,3"),
    // Blitz has no upper-roman counter style and lays out a square in the
    // bullet font (submilli SUB-1706).
    ("upper-roman@7", "7.03,0"),
    ("upper-roman@10", "10.05,0"),
    ("upper-roman@13", "13.06,0"),
    ("upper-roman@13.3", "13.36,0"),
    ("upper-roman@16", "16.08,1"),
    ("upper-roman@20", "20.09,1"),
    ("upper-roman@32", "32.14,2"),
    ("upper-roman@50", "50.22,3"),
    // Blitz lays out no inside marker before a block child (submilli
    // SUB-1707).
    ("block", "0,0"),
];

#[test]
fn content_after_inside_markers_starts_where_chrome_puts_it() {
    let cases = include_str!("fixtures/list_markers.tsv");
    let mut differing = Vec::new();
    let mut known = 0;
    for line in cases.lines().filter(|line| !line.starts_with('#')) {
        let [key, html, chrome] = line.split('\t').collect::<Vec<_>>()[..] else {
            panic!("malformed line: {line}");
        };
        let ours = start_of_content(html);
        match KNOWN_DIFFERENCES.iter().find(|(known, _)| *known == key) {
            Some((_, pinned)) => {
                assert_eq!(ours, *pinned, "{key} changed");
                assert!(!same_position(&ours, chrome), "{key} no longer differs");
                known += 1;
            }
            None if !same_position(&ours, chrome) => {
                differing.push(format!("{key}: Chrome {chrome}, ours {ours}"));
            }
            None => {}
        }
    }
    assert!(differing.is_empty(), "{}", differing.join("\n"));
    assert_eq!(known, KNOWN_DIFFERENCES.len());
}

#[test]
fn symbol_markers_keep_their_width_at_any_device_scale() {
    // Chrome 155 (macOS) starts the text 18, 22 and 43px in at 13, 16 and
    // 32px, at device scale factors 1.5, 2 and 3 as at 1.
    for scale in [1.5, 2.0, 3.0] {
        for (size, x) in [(13, 18.0), (16, 22.0), (32, 43.0)] {
            let html = format!(
                "<ul style='font:{size}px Arial;list-style-position:inside'><li id=t><span id=s>abc</span></li></ul>"
            );
            let doc = scaled_page(&html, scale);
            let item = doc.get_client_bounding_rect(q(&doc, "#t")).unwrap();
            let span = doc.node_client_rects(q(&doc, "#s"))[0];
            let start = span.x - item.x;
            assert!(
                (start - x).abs() <= 1.0 / 32.0,
                "{size}px at {scale}: {start}"
            );
        }
    }
}

#[test]
fn symbol_markers_of_huge_fonts_lay_out() {
    for size in ["1e9px", "3e38px", "calc(infinity * 1px)"] {
        let html = format!(
            "<ul style='font:{size} Arial;list-style-position:inside'><li id=t><span id=s>a</span></li></ul>"
        );
        let doc = page(&html);
        let rect = doc.node_client_rects(q(&doc, "#s"))[0];
        assert!(rect.x.is_finite() && rect.y.is_finite(), "{size}: {rect:?}");
    }
}

fn page(html: &str) -> BaseDocument {
    scaled_page(html, 1.0)
}

fn scaled_page(html: &str, scale: f32) -> BaseDocument {
    laid_out_at_scale(
        &format!("<!doctype html><html><head></head><body>{html}</body></html>"),
        scale,
    )
}

/// The first client rect of `#s` relative to the border box of `#t`.
fn start_of_content(html: &str) -> String {
    let doc = page(html);
    let item = doc.get_client_bounding_rect(q(&doc, "#t")).unwrap();
    let span = doc.node_client_rects(q(&doc, "#s"))[0];
    let round = |v: f64| (v * 100.0).round() / 100.0;
    format!("{},{}", round(span.x - item.x), round(span.y - item.y))
}

/// Positions within 1/32px: Parley and Chrome round the advances of a text
/// marker's glyphs apart.
fn same_position(ours: &str, chrome: &str) -> bool {
    let parse = |s: &str| -> Vec<f64> { s.split(',').map(|v| v.parse().unwrap()).collect() };
    let (ours, chrome) = (parse(ours), parse(chrome));
    ours.len() == chrome.len()
        && ours
            .iter()
            .zip(&chrome)
            .all(|(a, b)| (a - b).abs() <= 1.0 / 32.0)
}
