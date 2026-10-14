//! `Selection.modify` steps at `<select>` controls against Chrome 154
//! (macOS), in `fixtures/caret_select.tsv`: a line step forward from just
//! before a select enters it (a drop-down's label, reported before the
//! select, or a list box's first row), and a list box's rows are lines.

mod caret_support;
mod common;

use caret_support::recorded::run;

/// Word steps that still differ from Chrome: Chrome stops beside atomic
/// inlines (images and controls alike), which ours pass.
// submilli SUB-1676.
const KNOWN_DIFFERENCES: &[(&str, &str)] = &[
    ("dd.fw", "#aa:2 | #bb:2"),
    ("dd.fw2", "#aa:2 | #bb:2 | #bb:2 | #bb:2"),
    ("dd.bw2", "P:2 | #aa:0 | #aa:0 | #aa:0"),
    ("dd.fwsp", "#aa :2 | # bb:3 | # bb:3 | # bb:3"),
    ("lb.fw2", "#aa:2 | #bb:2 | #bb:2"),
    ("inp.fw2", "#aa:2 | #bb:2 | #bb:2"),
    ("img.fw2", "#aa:2 | #bb:2 | #bb:2"),
];

#[test]
fn steps_at_selects_match_chrome() {
    let cases = include_str!("fixtures/caret_select.tsv");
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
