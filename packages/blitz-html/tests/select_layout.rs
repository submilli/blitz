//! `<select>` layout against Chrome 154 (macOS): drop-downs as wide as
//! their longest option label and their button part, list boxes as tall as
//! `size` rows, and a drop-down's label on its line's baseline.

mod common;

use blitz_dom::BaseDocument;
use blitz_dom::node::SelectContent;
use common::{laid_out, q};

/// Border boxes measured in Chrome, in `fixtures/select_sizes.tsv`.
const SIZES: &str = include_str!("fixtures/select_sizes.tsv");

/// Cases whose fonts the tests do not have: they lay out in Liberation Sans,
/// which has Arial's metrics, but not Times New Roman's, the system font's
/// or bold.
const OTHER_FONTS: &[&str] = &["dd_ff", "dd_sys", "dd_bold"];

/// Cases that still differ from Chrome, with our size.
// `appearance`, block-level and `multiple size=1` selects and vertical
// writing modes: submilli SUB-1678.
const KNOWN_DIFFERENCES: &[(&str, (f64, f64))] = &[
    // `appearance: none` drops the button part and the label's padding.
    ("none", (44.0, 19.0)),
    ("none16", (48.0, 21.0)),
    // A block-level select keeps its intrinsic width in Chrome.
    ("block", (784.0, 19.0)),
    // Chrome sizes a one-row multiple select unlike other list boxes.
    ("size1m", (42.5, 19.0)),
    // A vertical select swaps its axes.
    ("dd_vertical", (44.0, 19.0)),
];

/// Text advances differ from Chrome's Arial by a few hundredths of a pixel.
const TOLERANCE: f64 = 0.05;

fn page(html: &str) -> BaseDocument {
    laid_out(&format!(
        "<!doctype html><html><head></head><body>{html}</body></html>"
    ))
}

fn border_box(doc: &BaseDocument, selector: &str) -> blitz_dom::BoundingRect {
    doc.get_client_bounding_rect(q(doc, selector))
        .expect("the element is laid out")
}

#[test]
fn sizes_match_chrome() {
    let mut differing = Vec::new();
    let mut known = 0;
    for line in SIZES.lines().filter(|line| !line.starts_with('#')) {
        let [key, html, size] = line.split('\t').collect::<Vec<_>>()[..] else {
            panic!("malformed line: {line}");
        };
        if OTHER_FONTS.contains(&key) {
            continue;
        }
        let (width, height) = size.split_once('x').expect("a size is WxH");
        let chrome: (f64, f64) = (width.parse().unwrap(), height.parse().unwrap());
        let rect = border_box(&page(html), "select");
        let ours = (rect.width, rect.height);
        let near = |a: (f64, f64), b: (f64, f64)| {
            (a.0 - b.0).abs() <= TOLERANCE && (a.1 - b.1).abs() <= TOLERANCE
        };
        match KNOWN_DIFFERENCES.iter().find(|(known, _)| *known == key) {
            Some((_, pinned)) => {
                assert!(near(ours, *pinned), "{key} changed: {ours:?}");
                assert!(!near(ours, chrome), "{key} no longer differs");
                known += 1;
            }
            None if !near(ours, chrome) => {
                differing.push(format!("{key}: Chrome {chrome:?}, ours {ours:?}"));
            }
            None => {}
        }
    }
    assert!(differing.is_empty(), "{}", differing.join("\n"));
    assert_eq!(known, KNOWN_DIFFERENCES.len());
}

#[test]
fn a_drop_down_sits_on_the_baseline_of_its_label() {
    // Chrome: the 30px select's label shares the 16px text's baseline, so
    // the select starts 15px above the text.
    let doc = page(
        "<div style='font-family:Arial;font-size:16px'><span>x</span>\
         <select style='font-size:30px'><option>abc</option></select></div>",
    );
    let (span, select) = (border_box(&doc, "span"), border_box(&doc, "select"));
    assert_eq!(span.y - select.y, 15.0);
    // Padding moves the label, and the box with it.
    let doc = page(
        "<div style='font-family:Arial;font-size:16px'><span>x</span>\
         <select style='padding:10px'><option>abc</option></select></div>",
    );
    let (span, select) = (border_box(&doc, "span"), border_box(&doc, "select"));
    assert_eq!(span.y - select.y, 10.0);
}

#[test]
fn a_list_box_of_many_rows_is_as_tall_as_chrome_lays_out() {
    // Chrome's largest layout length bounds the box.
    for size in ["10000000", "4294967295"] {
        let doc = page(&format!(
            "<select size={size} style='font-family:Arial'><option>abc</option></select>"
        ));
        assert_eq!(border_box(&doc, "select").height, 33_554_432.0, "{size}");
    }
}

#[test]
fn a_drop_down_shows_its_selection_and_sizes_by_every_option() {
    let mut doc = page(
        "<select style='font-family:Arial'><option>o</option>\
         <option>a much longer label</option></select>",
    );
    let select = q(&doc, "select");
    let label_width = |doc: &BaseDocument| {
        let element = doc.get_node(select).unwrap().element_data().unwrap();
        let layout = element
            .select_layout
            .as_deref()
            .expect("a select is laid out");
        let SelectContent::DropDown(drop_down) = &layout.content else {
            panic!("a drop-down");
        };
        let label = drop_down.label.as_deref().expect("an option is selected");
        f64::from(label.full_width())
    };
    let width = border_box(&doc, "select").width;
    let short = label_width(&doc);
    doc.mutate().set_select_index(select, 1);
    doc.resolve(0.0);
    assert!(label_width(&doc) > short + 50.0);
    assert_eq!(border_box(&doc, "select").width, width);

    // An option's text sizes the control as it changes.
    let option = doc.get_node(select).unwrap().children[1];
    let text = doc.get_node(option).unwrap().children[0];
    doc.mutate().set_node_text(text, "x");
    doc.resolve(0.0);
    assert_eq!(border_box(&doc, "select").width, 30.0);
}

#[test]
fn huge_author_lengths_keep_a_select_finite() {
    // Infinite baselines or widths would leave line breaking unable to
    // place the box, and paint with nothing to draw.
    for style in [
        "padding-block:3e38px",
        "margin-top:3e38px;padding-top:3e38px",
        "letter-spacing:3e38px",
        "font-size:3e38px",
    ] {
        let doc = page(&format!(
            "<div>a<select style='{style}'><option>abc</option></select><br>b</div>"
        ));
        let rect = border_box(&doc, "select");
        assert!(
            rect.width.is_finite() && rect.height.is_finite(),
            "{style}: {rect:?}"
        );
    }
}

/// The drop-down label's width in device pixels and its scale.
fn label_metrics(doc: &BaseDocument) -> (f32, f32) {
    let element = doc
        .get_node(q(doc, "select"))
        .unwrap()
        .element_data()
        .unwrap();
    let layout = element
        .select_layout
        .as_deref()
        .expect("a select is laid out");
    let SelectContent::DropDown(drop_down) = &layout.content else {
        panic!("a drop-down");
    };
    let label = drop_down.label.as_deref().expect("an option is selected");
    (label.full_width(), label.scale())
}

#[test]
fn a_select_is_laid_out_again_at_a_new_scale() {
    let mut doc = page("<select style='font-family:Arial'><option>abc</option></select>");
    let (width, scale) = label_metrics(&doc);
    let mut viewport = doc.viewport().clone();
    viewport.hidpi_scale = 2.0;
    doc.set_viewport(viewport);
    doc.resolve(0.0);
    let (new_width, new_scale) = label_metrics(&doc);
    assert_eq!((scale, new_scale), (1.0, 2.0));
    assert!((new_width - 2.0 * width).abs() < 0.5, "{width} {new_width}");
}

#[test]
fn changes_inside_options_lay_a_select_out_again() {
    // Text nested in an option, which has no style of its own.
    let mut doc = page("<select style='font-family:Arial'><option>a<b>bc</b></option></select>");
    assert_eq!(border_box(&doc, "select").width, 44.0);
    let b = q(&doc, "b");
    let text = doc.get_node(b).unwrap().children[0];
    doc.mutate().set_node_text(text, "bcdefghij");
    doc.resolve(0.0);
    assert!(border_box(&doc, "select").width > 80.0);

    // An option's padding, a change that only relays out.
    let mut doc = page(
        "<style>.pad option{padding:5px}</style>\
         <select size=3 style='font-family:Arial'><option>abc</option></select>",
    );
    let select = q(&doc, "select");
    let class = blitz_dom::QualName::new(None, blitz_dom::ns!(), "class".into());
    doc.mutate().set_attribute(select, class, "pad");
    doc.resolve(0.0);
    // Chrome: `lb_optpad`.
    let rect = border_box(&doc, "select");
    assert_eq!((rect.width, rect.height), (48.5, 80.0));
}
