//! Button layout against Chrome 154 (macOS): inline-blocks in the control
//! font around an anonymous content box, which is centered in the block axis
//! when it fits, and whose first line gives the button's baseline.

mod common;

use blitz_dom::BaseDocument;
use common::{laid_out, q};

/// Border boxes measured in Chrome, in `fixtures/button_layout.tsv`.
const BOXES: &str = include_str!("fixtures/button_layout.tsv");

/// Cases that still differ from Chrome, with our boxes.
const KNOWN_DIFFERENCES: &[(&str, &str)] = &[
    // Blitz snaps box edges to whole pixels and rounds shrink-to-fit inline
    // content up to one, where Chrome keeps 64ths of a pixel (submilli
    // SUB-1683).
    ("x", "23x21+0+0"),
    ("long", "133x21+0+0"),
    ("baseline", "17.8x17+0+1 23x21+17.8+0 16.91x17+40.8+1"),
    ("basespan", "23x21+0+0 6.67x15+8.17+3"),
    ("short", "23x6+0+0 6.67x15+8.17+3"),
    ("blocks", "31x36+0+0 15x15+8+3 15x15+8+18"),
    ("mixed", "31x51+0+0 15x15+8+18"),
    (
        "inline",
        "17.8x17+0+1 23x21+17.8+0 6.67x15+25.97+3 16.91x17+40.8+1",
    ),
    ("block", "23x21+0+0 6.67x15+8.17+3"),
    ("blocktall", "23x50+0+0 6.67x15+8.17+17.5"),
    ("flex", "32x21+0+0 8x15+8+3 8x15+16+3"),
    ("flexcol", "24x60+0+0 8x15+8+3 8x15+8+18"),
    ("grid", "24x36+0+0 8x15+8+3 8x15+8+18"),
    ("pad", "31x39+0+0 6.67x15+12.17+12"),
    ("input", "23x21+0+0"),
    ("submit", "23x21+0+0"),
    ("reset", "23x21+0+0"),
    ("inputbase", "17.8x17+0+1 23x21+17.8+0 16.91x17+40.8+1"),
    ("abspos", "16x60+0+0 7x15+8+30"),
    ("float", "32x21+0+0 8x15+8+3"),
    ("overflow", "23x20+0+0 7x50+8+3"),
    ("margin", "23x41+0+0 7x15+8+13"),
    ("pmargin", "23x47.67+0+-0.33 7x15+8+16"),
    ("blockmargin", "7x35+0+0 7x15+0+10"),
    ("floatline", "32x46+0+0 8x40+8+3 7.42x15+16.3+3"),
    // Parley also rounds a tall inline box's extents, so the line sits half
    // a pixel lower (submilli SUB-1677).
    ("tall", "23x60+0+0.5 6.67x15+8.17+23"),
    ("tallbase", "17.8x17+0+21 23x60+17.8+0.5 6.67x15+25.97+23"),
    ("alignend", "23x60+0+0.5 6.67x15+8.17+23"),
    ("inputtall", "23x60+0+0.5"),
    ("tallmargin", "23x60+0+0.5 7x15+8+23"),
    // A line holding only atomic inlines has no strut (submilli SUB-1677).
    ("empty", "16x6+0+0"),
    ("img", "34x13+0+0"),
    ("nested", "23x21+0+7"),
    // Text of a smaller font sits a pixel higher in its line (submilli
    // SUB-1677).
    ("contents", "0x0+-8+-8 6.67x15+0+1"),
    // A flex or grid button sits on the baseline instead of aligning its
    // first item's (submilli SUB-1677).
    ("flexbase", "17.8x17+0+46 23x60+17.8+0 7x54+25.8+3"),
    ("gridbase", "17.8x17+0+46 23x60+17.8+0 7x54+25.8+3"),
    // Vertical writing modes (submilli SUB-1678).
    ("vertical", "60x21+0+0 6.67x15+26.67+3"),
];

/// A border box's width, height, x and y.
type BorderBox = (f64, f64, f64, f64);

/// Text advances differ from Chrome's Arial by a few hundredths of a pixel.
const TOLERANCE: f64 = 0.05;

fn page(html: &str) -> BaseDocument {
    laid_out(&format!(
        "<!doctype html><html><head></head><body>{html}</body></html>"
    ))
}

/// Each measured element's border box from the first div's origin, as the
/// probe prints it.
fn boxes(doc: &BaseDocument) -> Vec<BorderBox> {
    let origin = doc
        .query_selector("div")
        .unwrap()
        .and_then(|div| doc.get_client_bounding_rect(div))
        .expect("each case has a laid-out div");
    let nodes = doc.query_selector_all("button, input, span, i").unwrap();
    nodes
        .iter()
        .map(|&node| {
            // A box-less element reports an empty rectangle at the origin.
            let Some(rect) = doc.get_client_bounding_rect(node) else {
                return (0.0, 0.0, -origin.x, -origin.y);
            };
            (
                rect.width,
                rect.height,
                rect.x - origin.x,
                rect.y - origin.y,
            )
        })
        .collect()
}

fn parse_box(text: &str) -> BorderBox {
    let (size, position) = text.split_once('+').expect("a box is WxH+X+Y");
    let (width, height) = size.split_once('x').expect("a size is WxH");
    // A negative offset prints as `+-8`.
    let (x, y) = position.split_once('+').expect("a position is X+Y");
    let number = |text: &str| text.parse::<f64>().expect("a number");
    (number(width), number(height), number(x), number(y))
}

fn format_boxes(boxes: &[BorderBox]) -> String {
    let round = |v: f64| (v * 100.0).round() / 100.0;
    boxes
        .iter()
        .map(|b| {
            format!(
                "{}x{}+{}+{}",
                round(b.0),
                round(b.1),
                round(b.2),
                round(b.3)
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn boxes_match_chrome() {
    let mut differing = Vec::new();
    let mut known = 0;
    for line in BOXES.lines().filter(|line| !line.starts_with('#')) {
        let mut fields = line.split('\t');
        let (Some(key), Some(html)) = (fields.next(), fields.next()) else {
            panic!("malformed line: {line}");
        };
        let chrome: Vec<_> = fields.map(parse_box).collect();
        let ours = boxes(&page(html));
        match KNOWN_DIFFERENCES.iter().find(|(known, _)| *known == key) {
            Some((_, pinned)) => {
                let pinned: Vec<_> = pinned.split(' ').map(parse_box).collect();
                assert!(
                    near(&ours, &pinned),
                    "{key} changed: {}",
                    format_boxes(&ours)
                );
                assert!(!near(&ours, &chrome), "{key} no longer differs");
                known += 1;
            }
            None if !near(&ours, &chrome) => differing.push(format!(
                "{key}: Chrome {}, ours {}",
                format_boxes(&chrome),
                format_boxes(&ours)
            )),
            None => {}
        }
    }
    assert!(differing.is_empty(), "\n{}", differing.join("\n"));
    assert_eq!(known, KNOWN_DIFFERENCES.len());
}

/// Whether two cases' boxes agree within [`TOLERANCE`].
fn near(a: &[BorderBox], b: &[BorderBox]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(a, b)| {
            [(a.0, b.0), (a.1, b.1), (a.2, b.2), (a.3, b.3)]
                .iter()
                .all(|(a, b)| (a - b).abs() <= TOLERANCE)
        })
}

#[test]
fn a_button_computes_chrome_styles() {
    let doc = page("<button>x</button><input type=submit value=x>");
    for selector in ["button", "input"] {
        let node = q(&doc, selector);
        let style = |property| doc.resolved_style_value(node, property);
        assert_eq!(style("display"), "inline-block", "{selector}");
        assert_eq!(style("font-size"), "13.3333px", "{selector}");
        assert_eq!(style("padding-top"), "1px", "{selector}");
        assert_eq!(style("padding-left"), "6px", "{selector}");
        assert_eq!(style("border-top-width"), "2px", "{selector}");
        assert_eq!(style("border-top-style"), "outset", "{selector}");
        assert_eq!(style("box-sizing"), "border-box", "{selector}");
        assert_eq!(style("text-align"), "center", "{selector}");
        assert_eq!(
            style("background-color"),
            "rgb(239, 239, 239)",
            "{selector}"
        );
    }
}

/// A box's top and height.
fn block_extent(doc: &BaseDocument, selector: &str) -> (f64, f64) {
    let rect = doc
        .get_client_bounding_rect(q(doc, selector))
        .expect("the element is laid out");
    (rect.y, rect.height)
}

#[test]
fn content_is_centered_when_it_fits() {
    // Chrome: the 15px line is 19.5px below the content box's top in a
    // 60px button, 17.5px in a 50px block-level one, whatever their
    // whole-pixel snapping.
    for (style, offset) in [("height:60px", 22.5), ("display:block;height:50px", 17.5)] {
        let doc = page(&format!("<button style='{style}'><span>x</span></button>"));
        let (button, span) = (block_extent(&doc, "button"), block_extent(&doc, "span"));
        assert_eq!(span.0 - button.0, offset, "{style}");
    }
    // Content taller than the button stays at its top.
    let doc = page(
        "<button style='height:20px'><span style='display:block;height:50px'>x</span></button>",
    );
    let (button, span) = (block_extent(&doc, "button"), block_extent(&doc, "span"));
    assert_eq!(span.0 - button.0, 3.0);
}

#[test]
fn content_keeps_its_margins_and_floats() {
    // The content box is a flow root: a block's 10px margins stay inside it
    // (Chrome: a 41px button, the block 13px down), and so does a float
    // taller than the line (a 46px button).
    let doc = page("<button><span style='display:block;margin:10px 0'>x</span></button>");
    let (button, span) = (block_extent(&doc, "button"), block_extent(&doc, "span"));
    assert_eq!((button.1, span.0 - button.0), (41.0, 13.0));
    let doc = page("<button><span style='float:left;display:block;height:40px'>a</span>b</button>");
    assert_eq!(block_extent(&doc, "button").1, 46.0);
}

#[test]
fn a_button_shares_its_contents_baseline_with_its_line() {
    // The button's text and the line's sit on one baseline: their tops are
    // their fonts' ascents (12px and 14px) above it.
    for style in ["", "height:60px", "padding:10px"] {
        let doc = page(&format!(
            "<div style='font-size:16px'><i>ab</i><button style='{style}'><span>x</span></button></div>"
        ));
        let (outer, inner) = (block_extent(&doc, "i"), block_extent(&doc, "span"));
        assert_eq!(inner.0 + 12.0, outer.0 + 14.0, "{style}");
    }
    // With no content, its baseline is its content box's bottom.
    let doc = page("<div style='font-size:16px'><i>ab</i><button></button></div>");
    let (outer, button) = (block_extent(&doc, "i"), block_extent(&doc, "button"));
    assert_eq!(button.0 + button.1 - 3.0, outer.0 + 14.0);
}

#[test]
fn a_block_level_button_fits_its_content() {
    for display in ["block", "flex", "grid"] {
        let doc = page(&format!("<button style='display:{display}'>x</button>"));
        let rect = doc.get_client_bounding_rect(q(&doc, "button")).unwrap();
        assert!(rect.width < 30.0, "{display}: {}", rect.width);
    }
}

#[test]
fn a_button_is_laid_out_again_when_its_text_changes() {
    let mut doc = page("<button>x</button>");
    let button = q(&doc, "button");
    let width = |doc: &BaseDocument| doc.get_client_bounding_rect(button).unwrap().width;
    let narrow = width(&doc);
    let text = doc.get_node(button).unwrap().children[0];
    doc.mutate().set_node_text(text, "a much longer label");
    doc.resolve(0.0);
    assert!(width(&doc) > narrow + 50.0);
}

#[test]
fn a_button_is_laid_out_again_when_a_block_is_appended() {
    // The block stacks its line below the text, as in Chrome.
    let mut doc = page("<button>x</button>");
    let button = q(&doc, "button");
    let mut mutation = doc.mutate();
    let block = mutation.create_element(
        blitz_dom::QualName::new(None, blitz_dom::ns!(html), "div".into()),
        vec![],
    );
    let label = mutation.create_text_node("y");
    mutation.append_children(block, &[label]);
    mutation.append_children(button, &[block]);
    drop(mutation);
    doc.resolve(0.0);
    let rect = doc.get_client_bounding_rect(button).unwrap();
    assert_eq!(rect.height, 36.0);
}
