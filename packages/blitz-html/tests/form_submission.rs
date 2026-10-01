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

#[test]
fn submission_uses_current_owners_and_only_submittable_controls() {
    let entries = submit(
        "<input form=f name=before value=a><form id=f method=post><input name=inner value=b><input name=other value=x form=g><output name=result>42</output><fieldset name=group></fieldset></form><form id=g></form><input id=after form=f name=after value=c><output form=f name=external>43</output>",
        |doc| {
            let after = q(doc, "#after");
            doc.mutate()
                .set_attribute_by_name(after, "form", "g")
                .unwrap();
        },
    );
    assert_eq!(entries, pairs(&[("before", "a"), ("inner", "b")]));
}

#[test]
fn activated_submitters_supply_entries_and_navigation_overrides() {
    use blitz_dom::{EventDriver, NoopEventHandler};
    use blitz_traits::events::DomEvent;
    for control in [
        "<button id=b name=action value=save>",
        "<button type=INVALID id=b name=action value=save>",
        "<button type=SuBmIt id=b name=action value=save>",
        "<input type=submit id=b name=action value=save>",
        "<input type=SuBmIt id=b name=action value=save>",
    ] {
        let capture = Arc::new(Capture::default());
        let mut doc = BaseDocument::new(DocumentConfig {
            base_url: Some("https://example.test/".into()),
            html_parser_provider: Some(Arc::new(HtmlProvider)),
            navigation_provider: Some(capture.clone()),
            ..Default::default()
        });
        DocumentHtmlParser::parse_into_mutator(
            &mut doc.mutate(),
            &format!("<form id=f action=/wrong method=get><input name=x value=y>{control}</form>"),
        );
        let button = q(&doc, "#b");
        for (name, value) in [
            ("formaction", "/save"),
            ("formmethod", "POST"),
            ("formenctype", "text/plain"),
        ] {
            doc.mutate()
                .set_attribute_by_name(button, name, value)
                .unwrap();
        }
        let event = doc
            .get_node(button)
            .unwrap()
            .synthetic_click_event(keyboard_types::Modifiers::empty());
        EventDriver::new(&mut doc, NoopEventHandler).handle_dom_event(DomEvent::new(button, event));
        let options = capture.0.lock().unwrap().pop().expect("activation submits");
        assert_eq!(options.url.as_str(), "https://example.test/save");
        assert_eq!(options.method.as_str(), "POST");
        assert_eq!(options.content_type.as_deref(), Some("text/plain"));
        let Body::Form(entries) = options.document_resource else {
            panic!("POST has a form body")
        };
        let values: Vec<_> = entries
            .0
            .iter()
            .map(|e| (e.name.clone(), e.value.as_ref().to_owned()))
            .collect();
        assert_eq!(values, pairs(&[("x", "y"), ("action", "save")]));
    }
}

#[test]
fn image_submission_preserves_pointer_coordinates_and_programmatic_origin() {
    use blitz_dom::{EventDriver, NoopEventHandler};
    use blitz_traits::events::{DomEvent, DomEventData};
    for named in [false, true] {
        for physical in [false, true] {
            let capture = Arc::new(Capture::default());
            let mut doc = BaseDocument::new(DocumentConfig {
                base_url: Some("https://example.test/".into()),
                html_parser_provider: Some(Arc::new(HtmlProvider)),
                navigation_provider: Some(capture.clone()),
                ..Default::default()
            });
            let name = if named { "name=point" } else { "" };
            DocumentHtmlParser::parse_into_mutator(
                &mut doc.mutate(),
                &format!(
                    "<form method=get><input id=b type=image {name} style='width:100px;height:100px;border:5px solid;padding:7px'></form>"
                ),
            );
            doc.resolve(0.0);
            let button = q(&doc, "#b");
            let rect = doc.get_client_bounding_rect(button).unwrap();
            let mut pointer = doc
                .get_node(button)
                .unwrap()
                .synthetic_click_event_data(keyboard_types::Modifiers::empty());
            pointer.coords.client_x = rect.x as f32 + 30.75;
            pointer.coords.client_y = rect.y as f32 + 40.5;
            let event = if physical {
                DomEventData::PointerUp(pointer)
            } else {
                DomEventData::Click(pointer)
            };
            EventDriver::new(&mut doc, NoopEventHandler)
                .handle_dom_event(DomEvent::new(button, event));
            let options = capture
                .0
                .lock()
                .unwrap()
                .pop()
                .expect("image activation submits");
            let prefix = if named { "point." } else { "" };
            let (x, y) = if physical { (26, 36) } else { (0, 0) };
            assert_eq!(
                options.url.query(),
                Some(format!("{prefix}x={x}&{prefix}y={y}").as_str())
            );
            let form = q(&doc, "form");
            let entries = doc.form_entry_list(form, Some(button)).unwrap();
            assert_eq!(entries.0.len(), 2);
            assert_eq!(entries.0[0].value.as_ref(), x.to_string());
            assert_eq!(entries.0[1].value.as_ref(), y.to_string());
        }
    }
}

#[test]
fn host_prepared_files_have_bounded_contents_and_selection_lifecycle() {
    use blitz_traits::net::FormFile;
    let mut doc = BaseDocument::new(DocumentConfig::default());
    DocumentHtmlParser::parse_into_mutator(
        &mut doc.mutate(),
        "<form><input type=file name=f></form>",
    );
    let input = q(&doc, "input");
    let form = q(&doc, "form");
    let file = FormFile {
        name: "sample.txt".into(),
        content_type: "text/plain".into(),
        bytes: Arc::from(&b"a\0b"[..]),
    };
    doc.mutate()
        .set_form_files(input, vec![file.clone()])
        .unwrap();
    assert_eq!(doc.form_value(input).unwrap(), "C:\\fakepath\\sample.txt");
    assert_eq!(
        doc.form_entry_list(form, None).unwrap()[0].value,
        EntryValue::FileContents(file.clone())
    );
    let mut invalid = file.clone();
    invalid.name = "/private/path.txt".into();
    assert_eq!(
        doc.mutate().set_form_files(input, vec![invalid]),
        Err(blitz_dom::FormFileError::InvalidMetadata)
    );
    assert_eq!(
        doc.form_entry_list(form, None).unwrap()[0].value,
        EntryValue::FileContents(file.clone())
    );
    let mut oversized = file.clone();
    oversized.bytes = Arc::from(vec![0; 16 * 1024 * 1024]);
    assert_eq!(
        doc.mutate().set_form_files(input, vec![oversized]),
        Err(blitz_dom::FormFileError::DataLimit)
    );
    doc.mutate().reset_form_controls(form).unwrap();
    assert_eq!(
        doc.form_entry_list(form, None).unwrap()[0].value,
        EntryValue::EmptyFile
    );
    doc.mutate().set_form_files(input, vec![file]).unwrap();
    doc.mutate().set_form_value(input, "");
    assert_eq!(
        doc.form_entry_list(form, None).unwrap()[0].value,
        EntryValue::EmptyFile
    );
}

#[test]
fn shared_file_selections_fail_entry_admission_before_metadata_amplification() {
    use blitz_traits::net::{FormDataLimit, FormFile};
    let mut doc = BaseDocument::new(DocumentConfig::default());
    DocumentHtmlParser::parse_into_mutator(
        &mut doc.mutate(),
        "<form><input type=file name=f></form>",
    );
    let input = q(&doc, "input");
    let form = q(&doc, "form");
    let file = FormFile {
        name: "n".repeat(4096),
        content_type: String::new(),
        bytes: Arc::from(&b""[..]),
    };
    doc.mutate().set_form_files(input, vec![file; 256]).unwrap();
    let shared = doc
        .get_node(input)
        .unwrap()
        .element_data()
        .unwrap()
        .form_state
        .files
        .clone();
    for _ in 0..16 {
        let copy = doc.mutate().clone_node(input, false);
        doc.mutate().append_children(form, &[copy]);
        assert!(Arc::ptr_eq(
            &shared,
            &doc.get_node(copy)
                .unwrap()
                .element_data()
                .unwrap()
                .form_state
                .files
        ));
    }
    assert_eq!(doc.form_entry_list(form, None), Err(FormDataLimit));
}

#[test]
fn prepared_selection_validity_type_change_and_clone_lifecycle() {
    use blitz_traits::net::FormFile;
    let mut doc = BaseDocument::new(DocumentConfig::default());
    DocumentHtmlParser::parse_into_mutator(
        &mut doc.mutate(),
        "<form><input type=file name=f required></form>",
    );
    let input = q(&doc, "input");
    let form = q(&doc, "form");
    assert!(doc.control_validity(input).unwrap().value_missing);
    let file = FormFile {
        name: "f".into(),
        content_type: String::new(),
        bytes: Arc::from(&b""[..]),
    };
    doc.mutate().set_form_files(input, vec![file]).unwrap();
    assert!(!doc.control_validity(input).unwrap().value_missing);
    doc.mutate()
        .set_attribute_by_name(input, "type", "FILE")
        .unwrap();
    assert_eq!(doc.form_value(input).unwrap(), "C:\\fakepath\\f");
    let copy = doc.mutate().clone_node(input, false);
    assert_eq!(doc.form_value(copy).unwrap(), "C:\\fakepath\\f");
    doc.mutate()
        .set_attribute_by_name(input, "type", "text")
        .unwrap();
    doc.mutate()
        .set_attribute_by_name(input, "type", "file")
        .unwrap();
    assert_eq!(
        doc.form_entry_list(form, None).unwrap()[0].value,
        EntryValue::EmptyFile
    );
}
