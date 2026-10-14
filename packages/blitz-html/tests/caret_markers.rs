//! `Selection.modify` steps at inside list markers against Chrome 155
//! (macOS), in `fixtures/caret_markers.tsv`. A line step keeps a horizontal
//! position; over a symbol marker it reaches generated content, and over the
//! marker's end margin the content after it.

mod caret_support;
mod common;

use caret_support::recorded::run;

#[test]
fn steps_at_inside_markers_match_chrome() {
    let cases = include_str!("fixtures/caret_markers.tsv");
    let mut differing = Vec::new();
    let mut count = 0;
    for line in cases.lines().filter(|line| !line.starts_with('#')) {
        let [key, html, path, offset, steps, chrome] = line.split('\t').collect::<Vec<_>>()[..]
        else {
            panic!("malformed line: {line}");
        };
        let ours = run(html, path, offset.parse().unwrap(), steps);
        if ours != chrome {
            differing.push(format!("{key}: Chrome {chrome}, ours {ours}"));
        }
        count += 1;
    }
    assert!(differing.is_empty(), "{}", differing.join("\n"));
    // Every recorded case ran: the fixture was not cut short.
    assert_eq!(count, 218);
}
