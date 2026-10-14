//! `Selection.modify` steps at buttons against Chrome 154 (macOS), in
//! `fixtures/caret_button.tsv`, for each `display` a button lays out with.

mod caret_support;
mod common;

use caret_support::recorded::run;

#[test]
fn steps_at_buttons_match_chrome() {
    let cases = include_str!("fixtures/caret_button.tsv");
    let mut differing = Vec::new();
    for line in cases.lines().filter(|line| !line.starts_with('#')) {
        let [key, html, path, offset, steps, chrome] = line.split('\t').collect::<Vec<_>>()[..]
        else {
            panic!("malformed line: {line}");
        };
        let ours = run(html, path, offset.parse().unwrap(), steps);
        if ours != chrome {
            differing.push(format!("{key}: Chrome {chrome}, ours {ours}"));
        }
    }
    assert!(differing.is_empty(), "{}", differing.join("\n"));
}
