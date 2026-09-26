//! Relative URLs in documents without a usable base URL do not panic.

use blitz_dom::{BaseDocument, DocumentConfig};
use blitz_html::DocumentHtmlParser;

#[test]
fn relative_resource_urls_without_a_base_are_skipped() {
    // The default base URL is a `data:` URL, which cannot be a base.
    let mut doc = BaseDocument::new(DocumentConfig::default());
    DocumentHtmlParser::parse_into_mutator(
        &mut doc.mutate(),
        r#"<link rel=stylesheet href="style.css"><img src="a.png"><form action="submit"></form>"#,
    );
    let form = doc.query_selector("form").unwrap().unwrap();
    // Submitting a form whose action cannot be resolved is a no-op.
    doc.submit_form(form, form);
}
