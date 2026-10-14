//! Inside list markers against Chrome 155 (macOS), in
//! `fixtures/list_markers.tsv`: where the content after the marker starts.
//! Chrome sizes a symbol marker (`disc`, `circle`, `square`) from the font's
//! ascent, with a 1em end margin; other markers are text.

mod common;

use blitz_dom::BaseDocument;
use common::{laid_out, q};

/// Cases that still differ from Chrome, with our positions.
const KNOWN_DIFFERENCES: &[(&str, &str)] = &[
    // Chrome lays out a disclosure marker as text in a fallback font of the
    // system, as Arial has no triangles; Blitz uses its bundled bullet font.
    ("disclosure-open@7", "7.19,0"),
    ("disclosure-closed@7", "7.19,0"),
    ("disclosure-open@10", "10.28,0"),
    ("disclosure-closed@10", "10.28,0"),
    ("disclosure-open@13", "13.36,0"),
    ("disclosure-closed@13", "13.36,0"),
    ("disclosure-open@13.3", "13.67,0"),
    ("disclosure-closed@13.3", "13.67,0"),
    ("disclosure-open@16", "16.45,0"),
    ("disclosure-closed@16", "16.45,0"),
    ("disclosure-open@20", "20.56,0"),
    ("disclosure-closed@20", "20.56,0"),
    ("disclosure-open@32", "32.89,0"),
    ("disclosure-closed@32", "32.89,0"),
    ("disclosure-open@50", "51.39,1"),
    ("disclosure-closed@50", "51.39,1"),
    // Blitz has no upper-roman counter style and lays out a square
    // (submilli SUB-1706).
    ("upper-roman@7", "6.17,0"),
    ("upper-roman@10", "8.81,0"),
    ("upper-roman@13", "11.47,0"),
    ("upper-roman@13.3", "11.73,0"),
    ("upper-roman@16", "14.11,0"),
    ("upper-roman@20", "17.64,0"),
    ("upper-roman@32", "28.22,0"),
    ("upper-roman@50", "44.09,1"),
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
    laid_out(&format!(
        "<!doctype html><html><head></head><body>{html}</body></html>"
    ))
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
