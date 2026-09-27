//! Form submission: the entries a submitted form sends.

mod common;

use std::sync::{Arc, Mutex};

use blitz_dom::{BaseDocument, DocumentConfig};
use blitz_html::{DocumentHtmlParser, HtmlProvider};
use blitz_traits::navigation::{NavigationOptions, NavigationProvider};
use blitz_traits::net::{Body, EntryValue};
use common::q;

#[derive(Default)]
struct Capture(Mutex<Vec<NavigationOptions>>);

impl NavigationProvider for Capture {
    fn navigate_to(&self, options: NavigationOptions) {
        self.0.lock().unwrap().push(options);
    }
}

fn submit(html: &str, edit: impl FnOnce(&mut BaseDocument)) -> Vec<(String, String)> {
    let capture = Arc::new(Capture::default());
    let mut doc = BaseDocument::new(DocumentConfig {
        base_url: Some("https://example.test/".into()),
        html_parser_provider: Some(Arc::new(HtmlProvider)),
        navigation_provider: Some(capture.clone()),
        ..Default::default()
    });
    DocumentHtmlParser::parse_into_mutator(&mut doc.mutate(), html);
    edit(&mut doc);
    let form = q(&doc, "form");
    doc.submit_form(form, form);
    let options = capture.0.lock().unwrap().pop().expect("submitted");
    let Body::Form(data) = options.document_resource else {
        panic!("a POST form body")
    };
    data.0
        .into_iter()
        .map(|e| match e.value {
            EntryValue::String(v) => (e.name, v),
            other => (e.name, format!("{other:?}")),
        })
        .collect()
}

fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter()
        .map(|(n, v)| (n.to_string(), v.to_string()))
        .collect()
}

#[test]
fn selects_submit_their_selected_options() {
    let entries = submit(
        "<form method=post><select name=one><option>a<option selected value=B>b</select>\
         <select name=many multiple><option selected>x<optgroup><option selected>y<option>z</optgroup>\
         <option selected disabled>w</select><select name=first><option>only</select></form>",
        |_| {},
    );
    assert_eq!(
        entries,
        pairs(&[
            ("one", "B"),
            ("many", "x"),
            ("many", "y"),
            ("first", "only")
        ])
    );
}

#[test]
fn controls_submit_their_current_values() {
    let entries = submit(
        "<form method=post><input name=t value=default><input id=typed name=typed value=old>\
         <textarea name=area>text\\nbody</textarea><input type=hidden name=h value=hid></form>",
        |doc| {
            let typed = q(doc, "#typed");
            doc.mutate().set_form_value(typed, "new");
        },
    );
    assert_eq!(
        entries,
        pairs(&[
            ("t", "default"),
            ("typed", "new"),
            ("area", "text\\nbody"),
            ("h", "hid")
        ])
    );
}
