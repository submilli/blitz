//! `Selection.modify` steps at buttons against Chrome 154 (macOS), in
//! `fixtures/caret_button.tsv`, for each `display` a button lays out with.

mod caret_support;
mod common;

use caret_support::recorded::run;

/// Steps that still differ from Chrome, with ours: character steps pass a
/// button, where Chrome's enter its content (submilli SUB-1676).
const KNOWN_DIFFERENCES: &[(&str, &str)] = &[
    ("button.fc", "P:2 | #bb:1 | #bb:2"),
    ("flex.fc", "P:2 | #bb:1 | #bb:2"),
    ("inline.fc", "P:2 | #bb:1 | #bb:2"),
];

#[test]
fn steps_at_buttons_match_chrome() {
    let cases = include_str!("fixtures/caret_button.tsv");
    let mut differing = Vec::new();
    let mut known = 0;
    for line in cases.lines().filter(|line| !line.starts_with('#')) {
        let [key, html, path, offset, steps, chrome] = line.split('\t').collect::<Vec<_>>()[..]
        else {
            panic!("malformed line: {line}");
        };
        let ours = run(html, path, offset.parse().unwrap(), steps);
        match KNOWN_DIFFERENCES.iter().find(|(known, _)| *known == key) {
            Some((_, pinned)) => {
                assert_eq!(ours, *pinned, "{key} changed");
                assert_ne!(ours, chrome, "{key} no longer differs");
                known += 1;
            }
            None if ours != chrome => {
                differing.push(format!("{key}: Chrome {chrome}, ours {ours}"));
            }
            None => {}
        }
    }
    assert!(differing.is_empty(), "{}", differing.join("\n"));
    assert_eq!(known, KNOWN_DIFFERENCES.len());
}
