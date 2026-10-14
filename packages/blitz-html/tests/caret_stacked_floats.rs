//! The five stacked-float caret cases from SUB-1554, recorded in Chrome 155.

mod caret_support;
mod common;

use caret_support::recorded::run_with_browser_fonts;

#[test]
fn steps_beside_stacked_floats_match_chrome() {
    let cases = include_str!("fixtures/caret_stacked_floats.tsv");
    let mut differing = Vec::new();
    let mut count = 0;
    for line in cases.lines().filter(|line| !line.starts_with('#')) {
        let [key, html, path, offset, steps, chrome] = line.split('\t').collect::<Vec<_>>()[..]
        else {
            panic!("malformed line: {line}");
        };
        let ours = run_with_browser_fonts(html, path, offset.parse().unwrap(), steps);
        if ours != chrome {
            differing.push(format!("{key}: Chrome {chrome}, ours {ours}"));
        }
        count += 1;
    }
    assert!(differing.is_empty(), "{}", differing.join("\n"));
    // Every recorded case ran: the fixture was not cut short.
    assert_eq!(count, 5);
}
