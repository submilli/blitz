//! Lengths past `f32` range saturate at 2^25 px, as Chrome's `LayoutUnit`s
//! do, so layout never hands line breaking a NaN or infinite width (which
//! panicked Parley's line breaker). Boxes measured in Chrome 155.

mod common;

use blitz_dom::BaseDocument;
use common::{laid_out, q};

/// Each page's `#t` border box in Chrome 155 (800x600, standards mode), as
/// `x,y WxH`. These pages and `KNOWN_DIFFERENCES`' are measured by
/// submilli-browser's `docs/reconstruction/probes/SUB-1578/geometry.js`;
/// keep the two lists alike.
const CHROME: &[(&str, &str, &str)] = &[
    (
        "pad",
        "<p id=t style='padding-left:3e38px;padding-right:3e38px'>aa</p>",
        "8,16 33554432x18",
    ),
    (
        "margins",
        "<div style='margin-left:2e38px'><p id=t style='margin-left:2e38px'>aa</p></div>",
        "33554432,16 0x18",
    ),
    (
        "w3e7",
        "<p id=t style='width:3e7px'>aa</p>",
        "8,16 30000000x18",
    ),
    (
        "ml",
        "<p id=t style='margin-left:1e9px'>aa</p>",
        "33554432,16 0x18",
    ),
    (
        "pad1e8",
        "<p id=t style='padding-left:1e8px;padding-right:1e8px'>aa bb</p>",
        "8,16 33554432x36",
    ),
    (
        "nest",
        "<div style='margin-left:3e7px'><div style='margin-left:3e7px'><p id=t style='margin-left:3e7px'>aa</p></div></div>",
        "33554432,16 0x18",
    ),
    (
        "pctcalc",
        "<div style='width:calc(1e30% + 1px)'><div style='width:calc(1e30% + 1px)'><p id=t>aa</p></div></div>",
        "8,16 33554432x18",
    ),
    (
        "pctpad",
        "<div style='padding-left:1e30%'><div style='padding-left:1e30%'><p id=t>aa</p></div></div>",
        "33554432,16 0x18",
    ),
    (
        "floatpad",
        "<div style='padding:0 3e38px'><img style='float:left;width:10px;height:10px'><p id=t>aa bb cc dd</p></div>",
        "33554432,16 0x72",
    ),
    (
        "floatbig",
        "<p id=t>aa <img style='float:left;width:3e38px;height:10px'> bb cc</p>",
        "8,16 784x18",
    ),
    (
        "floatneg",
        "<p id=t>aa <img style='float:left;width:10px;height:10px;margin-right:-3e38px'> bb cc</p>",
        "8,16 784x18",
    ),
    (
        "indent",
        "<p id=t style='text-indent:-3e38px'>aa bb</p>",
        "8,16 784x18",
    ),
    (
        "indentpct",
        "<p id=t style='text-indent:1e30%'>aa bb</p>",
        "8,16 784x36",
    ),
    (
        "flex",
        "<div style='display:flex'><p id=t style='padding:0 3e38px;flex:1e30'>aa</p><p style='flex-basis:3e38px'>b</p></div>",
        "8,24 33554432x18",
    ),
    (
        "grid",
        "<div style='display:grid;grid-template-columns:1e30fr 3e38px'><p id=t style='padding:0 3e38px'>a</p><p>b</p></div>",
        "8,24 33554432x18",
    ),
    (
        "abspos",
        "<div style='position:relative'><p id=t style='position:absolute;left:3e38px;right:3e38px'>aa bb</p></div>",
        "33554432,24 0x36",
    ),
    (
        "minmax",
        "<p id=t style='min-width:3e38px;max-width:-1px;padding:3e38px'>aa</p>",
        "8,16 33554432x33554432",
    ),
    (
        "letterspacing",
        "<p id=t style='letter-spacing:-3e38px'>aa bb</p>",
        "8,16 784x18",
    ),
    (
        "wordspacing",
        "<p id=t style='word-spacing:-3e38px'>aa bb</p>",
        "8,16 784x18",
    ),
    (
        "list",
        "<ul style='padding-left:3e38px'><li id=t style='list-style-position:inside;padding:0 3e38px'>aa</li></ul>",
        "33554432,16 33554432x36",
    ),
    (
        "ibpct",
        "<p>t<span id=t style='display:inline-block;padding:0 1e30%'>x</span>u</p>",
        "8,34 33554432x18",
    ),
    (
        "pctchain",
        "<div style='width:1e30%'><div style='width:1e30%'><div style='width:1e30%'><div style='width:1e30%'><div style='width:1e30%'><div style='width:1e30%'><p id=t>aa</p></div></div></div></div></div></div>",
        "8,16 33554432x18",
    ),
];

/// Pages where Chrome's saturating `LayoutUnit` arithmetic still differs
/// from ours, with Chrome's box and ours (submilli SUB-1702): sums at the
/// limit, inline offsets past it, floats beside saturated lines, huge
/// `border-spacing`, and transforms (Chrome saturates a translation, and
/// rounds a huge scale differently).
const KNOWN_DIFFERENCES: &[(&str, &str, &str, &str)] = &[
    (
        "w",
        "<p id=t style='width:3e38px'>aa</p>",
        "8,16 33554428x18",
        "8,16 33554432x18",
    ),
    (
        "ibinf",
        "<p>t<span id=t style='display:inline-block;padding:calc(infinity*1px)'>x</span>u</p>",
        "8,34 33554432x33554430",
        "8,32 33554432x33554432",
    ),
    (
        "btninf",
        "<p>t<button id=t style='padding:calc(infinity*1px)'>x</button>u</p>",
        "8,34 33554432x33554430",
        "8,32 33554432x33554432",
    ),
    (
        "td",
        "<table><tr><td id=t style='padding:3e38px'>a</td></tr></table>",
        "10,10 33554430x33554430",
        "10,10 33554432x33554432",
    ),
    (
        "mlneg",
        "<p id=t style='margin-left:-3e38px'>aa</p>",
        "-33554422,16 33554432x18",
        "-33554424,16 33554432x18",
    ),
    (
        "floatpad2",
        "<div style='padding-left:3e7px;padding-right:3e7px'><img style='float:left;width:10px;height:10px'><img style='float:right;width:3e7px;height:10px'><p id=t>aa bb cc dd</p></div>",
        "30000008,16 0x82",
        "30000008,16 0x76",
    ),
    (
        "floatin",
        "<p id=t style='padding:0 3e38px'>aa <img style='float:left;width:10px;height:10px'> bb <img style='float:right;width:3e38px;height:10px'> cc</p>",
        "8,16 33554432x54",
        "8,16 33554432x74",
    ),
    (
        "abspos2",
        "<p id=t style='position:absolute;left:-3e38px;width:3e38px;padding:3e38px'>aa bb</p>",
        "-33554430,24 33554432x33554432",
        "-33554432,24 33554432x33554432",
    ),
    (
        "ibnest",
        "<p><span style='display:inline-block;padding:0 3e38px'><span style='display:inline-block;padding:0 3e38px'><span style='display:inline-block;padding:0 3e38px'><span style='display:inline-block;padding:0 3e38px'><span style='display:inline-block;padding:0 3e38px'><span style='display:inline-block;padding:0 3e38px'><span id=t>x</span></span></span></span></span></span></span></p>",
        "201326592,16 0x18",
        "67108864,16 8x17",
    ),
    (
        "translate",
        "<p id=t style='transform:translateX(3e38px)'>aa</p>",
        "33554432,16 784x18",
        "300000000549775580000000000000000000000,16 0x18",
    ),
    (
        "scale",
        "<p id=t style='transform:scale(1e30)'>bb</p>",
        "-3.4028232737618185e+32,-8.999999984311469e+30 6.805646547523637e+32x1.7999999968622937e+31",
        "-392000005898606760000000000000000,-9000000135427196000000000000000 784000011797213500000000000000000x18000000270854392000000000000000",
    ),
    (
        "table",
        "<table style='border-spacing:3e38px'><tr><td id=t style='width:3e38px'>a</td><td style='width:3e38px'>b</td></tr></table>",
        "8,8 773.984375x20",
        "33554432,33554432 33554432x20",
    ),
];

fn page(html: &str) -> BaseDocument {
    laid_out(&format!(
        "<!doctype html><html><head></head><body>{html}</body></html>"
    ))
}

/// `#t`'s border box as `x,y WxH`.
fn border_box(doc: &BaseDocument) -> String {
    let rect = doc
        .get_client_bounding_rect(q(doc, "#t"))
        .expect("#t has a box");
    format!("{},{} {}x{}", rect.x, rect.y, rect.width, rect.height)
}

#[test]
fn lengths_past_float_range_saturate_like_chrome() {
    let differing: Vec<String> = CHROME
        .iter()
        .filter_map(|(name, html, chrome)| {
            let ours = border_box(&page(html));
            (ours != *chrome).then(|| format!("{name}: Chrome {chrome}, ours {ours}"))
        })
        .collect();
    assert!(differing.is_empty(), "{}", differing.join("\n"));
}

#[test]
fn saturated_pages_that_still_differ_from_chrome_lay_out() {
    for (name, html, chrome, ours) in KNOWN_DIFFERENCES {
        let actual = border_box(&page(html));
        assert_eq!(actual, *ours, "{name} (Chrome {chrome})");
    }
}

/// The issue's pages, and buttons and inline-blocks around their content:
/// each panicked at Parley's `line_break.rs` assertion.
#[test]
fn pages_whose_widths_overflowed_to_nan_lay_out() {
    for html in [
        "<p style='padding-left:3e38px;padding-right:3e38px'>aa</p>",
        "<div style='margin-left:2e38px'><p style='margin-left:2e38px'>aa</p></div>",
        "<p>t<button style='padding:calc(infinity*1px)'>x</button>u</p>",
        "<p>t<button style='padding-left:3e38px;padding-right:3e38px'>x</button>u</p>",
        "<button style='display:block;padding:calc(infinity*1px)'>x</button>",
        "<div style='display:flex'><button style='padding:calc(infinity*1px)'><div>x</div></button></div>",
        "<span style='display:inline-block;padding:calc(infinity*1px)'>x</span>",
        "<table><tr><td style='padding:3e38px'>a</td></tr></table>",
        "<div style='padding:0 3e38px'><img style='float:left;width:3e38px;height:10px'><p>aa bb</p></div>",
    ] {
        let doc = page(html);
        let body = doc.get_client_bounding_rect(q(&doc, "body")).unwrap();
        assert!(body.width.is_finite() && body.height.is_finite(), "{html}");
    }
}
