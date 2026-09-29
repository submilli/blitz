//! On-demand style contexts must drain while Stylo is in its layout state.
mod common;
use common::{parse, q};

#[test]
fn resolves_style_below_display_none_and_releases_layout_state() {
    let mut doc = parse(
        "<div style='display:none'><span id=hidden style='color:rgb(1, 2, 3)'>text</span></div>",
    );
    let hidden = q(&doc, "#hidden");
    doc.resolve(0.0);
    for _ in 0..2 {
        assert!(doc.resolve_undisplayed_style(hidden).is_some());
    }
    doc.resolve(0.0);
}
