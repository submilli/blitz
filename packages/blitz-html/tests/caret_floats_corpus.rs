//! `Selection.modify` steps around floats against Chrome 154 (macOS): the
//! 2,961 steps of the corpus in `fixtures/caret_floats_corpus.tsv`, each
//! started from a collapsed selection like `Selection.collapse`.

mod caret_support;
mod common;

use caret_support::recorded::run;
use std::collections::HashMap;

#[test]
fn steps_around_floats_match_chrome() {
    let corpus = include_str!("fixtures/caret_floats_corpus.tsv");
    let mut pages = HashMap::new();
    let mut differing = Vec::new();
    for line in corpus.lines().filter(|l| !l.starts_with('#')) {
        let fields: Vec<&str> = line.split('\t').collect();
        match fields.as_slice() {
            ["H", id, html] => {
                pages.insert(*id, html.replace("\\n", "\n"));
            }
            ["C", key, page, path, offset, steps, chrome] => {
                let ours = run(&pages[page], path, offset.parse().unwrap(), steps);
                if ours != *chrome {
                    differing.push(format!("{key}: Chrome {chrome}, ours {ours}"));
                }
            }
            _ => panic!("malformed corpus line: {line}"),
        }
    }
    assert!(differing.is_empty(), "{}", differing.join("\n"));
}
