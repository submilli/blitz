//! `@font-face` source selection: the first source in a supported format
//! is fetched; unsupported formats are skipped whether the format hint is a
//! keyword or a quoted string, and URL queries do not hide the extension.

use std::sync::{Arc, Mutex};

use blitz_dom::{BaseDocument, DocumentConfig};
use blitz_html::DocumentHtmlParser;
use blitz_traits::net::{NetHandler, NetProvider, Request};

#[derive(Default)]
struct Recorder(Mutex<Vec<String>>);

impl NetProvider for Recorder {
    fn fetch(&self, _: usize, request: Request, _: Box<dyn NetHandler>) {
        self.0.lock().unwrap().push(request.url.to_string());
    }
}

fn requested_fonts(css: &str) -> Vec<String> {
    let net = Arc::new(Recorder::default());
    let config = DocumentConfig {
        base_url: Some("https://example.test/css/".into()),
        net_provider: Some(net.clone()),
        ..Default::default()
    };
    let mut doc = BaseDocument::new(config);
    DocumentHtmlParser::parse_into_mutator(&mut doc.mutate(), &format!("<style>{css}</style>"));
    doc.resolve(0.0);
    net.0.lock().unwrap().clone()
}

#[test]
fn quoted_unsupported_formats_are_skipped() {
    // The Bootstrap pattern: IE's embedded-opentype first, quoted.
    let urls = requested_fonts(
        "@font-face { font-family: icons; src: url('a.eot?#iefix') format('embedded-opentype'), \
         url('a.svg#x') format('svg'), url('a.ttf') format('truetype'); }",
    );
    assert_eq!(urls, vec!["https://example.test/css/a.ttf"]);
}

#[test]
fn extension_guess_ignores_queries() {
    let urls = requested_fonts(
        "@font-face { font-family: icons; src: url('b.eot?v=1'), url('b.ttf?v=1'); }",
    );
    assert_eq!(urls, vec!["https://example.test/css/b.ttf?v=1"]);
}

#[test]
fn unknown_quoted_formats_are_skipped() {
    let urls = requested_fonts(
        "@font-face { font-family: f; src: url('c.xyz') format('made-up'), url('c.otf') format('opentype'); }",
    );
    assert_eq!(urls, vec!["https://example.test/css/c.otf"]);
}
