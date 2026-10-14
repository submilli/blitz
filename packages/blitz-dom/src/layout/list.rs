use blitz_traits::node_id::NodeId;
use markup5ever::local_name;
use parley::{FontContext, FontFamily, LayoutContext, TextStyle, WhiteSpaceCollapse};
use style::computed_values::list_style_type::T as ListStyleType;
use style::properties::ComputedValues;
use style::{
    computed_values::list_style_position::T as ListStylePosition, counter_style::CounterStyle,
};
use stylo_taffy::convert::layout_length;

use crate::{
    BaseDocument,
    font_metrics::first_available_or_fallback_font_metrics,
    node::{ListItemLayout, ListItemLayoutPosition, Marker, TextBrush},
    stylo_to_parley,
};

pub(super) const BULLET_FONT_FAMILY: &str = "Bullet, monospace, sans-serif";

pub(super) fn collect_list_item_children(
    doc: &mut BaseDocument,
    index: &mut usize,
    reversed: bool,
    node_id: NodeId,
) {
    let mut children = doc.nodes[node_id].children.to_vec();
    if reversed {
        children.reverse();
    }
    for child in children.into_iter() {
        if let Some(layout) = node_list_item_child(doc, child, *index) {
            let node = &mut doc.nodes[child];
            node.element_data_mut().unwrap().list_item_data = Some(Box::new(layout));
            *index += 1;
            collect_list_item_children(doc, index, reversed, child);
        } else {
            // Unset marker in case it was previously set
            let node = &mut doc.nodes[child];
            if let Some(element_data) = node.element_data_mut() {
                element_data.list_item_data = None;
            }
        }
    }
}

// Return a child node which is of display: list-item
fn node_list_item_child(
    doc: &mut BaseDocument,
    child_id: NodeId,
    index: usize,
) -> Option<ListItemLayout> {
    let node = &doc.nodes[child_id];

    // We only care about elements with display: list-item (li's have this automatically)
    if !node
        .primary_styles()
        .is_some_and(|style| style.get_box().display.is_list_item())
    {
        return None;
    }

    //Break on container elements when already in a list
    if node
        .element_data()
        .map(|element_data| {
            matches!(
                element_data.name.local,
                local_name!("ol") | local_name!("ul"),
            )
        })
        .unwrap_or(false)
    {
        return None;
    };

    let styles = node.primary_styles().unwrap();
    let list_style_type = styles.clone_list_style_type();
    let list_style_position = styles.clone_list_style_position();
    let marker = marker_for_style(list_style_type.clone(), index)?;

    let position = match list_style_position {
        ListStylePosition::Inside => ListItemLayoutPosition::Inside,
        ListStylePosition::Outside => {
            let mut parley_style = stylo_to_parley::style(child_id, &styles);

            if let Some(font_family) = font_for_bullet_style(list_style_type) {
                parley_style.font_family = font_family;
            }

            // Create a parley tree builder
            let mut font_ctx = doc.font_ctx.lock().unwrap();
            let mut builder = doc.layout_ctx.tree_builder(
                &mut font_ctx,
                doc.viewport.scale(),
                true,
                &parley_style,
            );

            match &marker {
                Marker::Char(char) => {
                    let mut buf = [0u8; 4];
                    builder.push_text(char.encode_utf8(&mut buf));
                }
                Marker::String(str) => builder.push_text(str),
            };

            let mut layout = builder.build().0;
            let width = layout.calculate_content_widths().max;
            layout.break_all_lines(Some(width));

            ListItemLayoutPosition::Outside(Box::new(layout))
        }
    };

    Some(ListItemLayout { marker, position })
}

/// The text style of an inside marker that is a single character followed
/// by a space, given the item's style and its text style (in CSS px).
///
/// Bullet glyphs live in the bundled bullet font. Without it a marker like
/// disclosure-closed (U+25B8) falls back to the item's own font and renders
/// as a missing glyph.
///
/// A symbol marker (`disc`, `circle` or `square`) also takes Chrome's width.
/// Chrome sizes it from the item's first available font rather than from a
/// glyph (Blink's `ListMarker::WidthOfSymbol` and `InlineMarginsForInside`):
/// the symbol box is `(ascent * 2 / 3 + 1) / 2 + 2` whole device pixels, with
/// `ascent` rounded, or none at font size 0, with a -1px start margin and a
/// 1em end margin. Letter and word spacing do not change it, so the marker
/// takes none, except for the word spacing that widens its space to that
/// width.
pub(super) fn inside_char_marker_style(
    layout_ctx: &mut LayoutContext<TextBrush>,
    font_ctx: &mut FontContext,
    item_style: Option<&ComputedValues>,
    text_style: &TextStyle<'static, 'static, TextBrush>,
    symbol: char,
    scale: f32,
) -> TextStyle<'static, 'static, TextBrush> {
    let mut marker_style = text_style.clone();
    marker_style.font_family = BULLET_FONT_FAMILY.into();
    let Some(item_style) = item_style.filter(|style| is_symbol_marker(style)) else {
        return marker_style;
    };
    marker_style.letter_spacing = 0.0;
    marker_style.word_spacing = 0.0;

    let font = item_style.get_font();
    let font_size = font.font_size.used_size.0.px();
    let ascent = first_available_or_fallback_font_metrics(font_ctx, font, font_size, scale)
        .map_or(0.0, |metrics| metrics.ascent * scale);
    let width = symbol_marker_width(f64::from(ascent), f64::from(font_size * scale)) / scale;

    // Collapsible white space ending a paragraph is removed: preserve it.
    let mut measured_style = marker_style.clone();
    measured_style.white_space_collapse = WhiteSpaceCollapse::Preserve;
    let mut builder = layout_ctx.tree_builder(font_ctx, scale, true, &measured_style);
    let mut buf = [0u8; 4];
    builder.push_text(symbol.encode_utf8(&mut buf));
    builder.push_text(" ");
    let mut natural = builder.build().0;
    natural.break_all_lines(None);
    let natural = natural.full_width() / scale;
    // Saturated, so that a huge font cannot make it infinite or NaN.
    marker_style.word_spacing = layout_length(width - natural);
    marker_style
}

/// Whether an item's marker is one of Chrome's symbol markers.
pub(crate) fn is_symbol_marker(style: &ComputedValues) -> bool {
    match &style.clone_list_style_type().0 {
        CounterStyle::Name(name) => matches!(&*name.0, "disc" | "circle" | "square"),
        _ => false,
    }
}

/// Chrome's inside symbol marker width, margins included, in device pixels.
/// Blink's integer arithmetic, in floats so that no font size overflows it.
fn symbol_marker_width(ascent: f64, font_size: f64) -> f32 {
    let symbol = if font_size == 0.0 {
        0.0
    } else {
        (((ascent.round() * 2.0 / 3.0).floor() + 1.0) / 2.0).floor() + 2.0
    };
    // The end margin is a LayoutUnit: whole 1/64ths of a pixel.
    let end_margin = (font_size * 64.0).trunc() / 64.0;
    (symbol - 1.0 + end_margin) as f32
}

// Determine the marker to render for a given list style type
fn marker_for_style(list_style_type: ListStyleType, index: usize) -> Option<Marker> {
    Some(match list_style_type.0 {
        CounterStyle::None => return None,
        CounterStyle::Name(name) => match &*name.0 {
            "lower-alpha" => {
                let mut marker = String::new();
                build_alpha_marker(index, &mut marker);
                Marker::String(format!("{marker}. "))
            }
            "upper-alpha" => {
                let mut marker = String::new();
                build_alpha_marker(index, &mut marker);
                Marker::String(format!("{}. ", marker.to_ascii_uppercase()))
            }
            "decimal" => Marker::String(format!("{}. ", index + 1)),
            "disc" => Marker::Char('•'),
            "circle" => Marker::Char('◦'),
            "square" => Marker::Char('▪'),
            "disclosure-open" => Marker::Char('▾'),
            "disclosure-closed" => Marker::Char('▸'),
            _ => Marker::Char('□'),
        },
        CounterStyle::String(atom_string) => Marker::String(atom_string.as_ref().to_string()),
        CounterStyle::Symbols { symbols, .. } => {
            // Use symbols from counter-style definition
            let syms = &symbols.0;
            let len = syms.len();
            if len == 0 {
                Marker::Char('•')
            } else {
                // Cycle through symbols based on index: 1st, 2nd, 1st, 2nd, etc.
                let idx = if len > 1 { index % len } else { 0 };
                // Symbol is String or Ident variant, extract the string value
                let sym = &syms[idx];
                let marker_str = match sym {
                    style::counter_style::Symbol::String(s) => s.to_string(),
                    style::counter_style::Symbol::Ident(id) => id.0.to_string(),
                };
                Marker::String(marker_str)
            }
        }
    })
}

// Override the font to our specific bullet font when rendering bullets
fn font_for_bullet_style(list_style_type: ListStyleType) -> Option<FontFamily<'static>> {
    if list_style_type.0.is_bullet() {
        return Some(BULLET_FONT_FAMILY.into());
    }

    None
}

const ALPHABET: [char; 26] = [
    'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k', 'l', 'm', 'n', 'o', 'p', 'q', 'r', 's',
    't', 'u', 'v', 'w', 'x', 'y', 'z',
];

// Construct alphanumeric marker from index, appending characters when index exceeds powers of 26
fn build_alpha_marker(index: usize, str: &mut String) {
    let rem = index % 26;
    let sym = ALPHABET[rem];
    str.insert(0, sym);
    let rest = (index - rem) as i64 / 26 - 1;
    if rest >= 0 {
        build_alpha_marker(rest as usize, str);
    }
}

#[cfg(test)]
mod tests {
    use crate::node::Marker;
    use style::{
        Atom,
        counter_style::CounterStyle,
        values::{CustomIdent, computed::ListStyleType},
    };

    use super::marker_for_style;

    fn list_style(s: &str) -> ListStyleType {
        ListStyleType(CounterStyle::Name(CustomIdent(Atom::from(s))))
    }

    #[test]
    fn test_marker_for_disc() {
        let result = marker_for_style(ListStyleType::disc(), 0);
        assert_eq!(result, Some(Marker::Char('•')));
    }

    #[test]
    fn test_marker_for_decimal() {
        let result_1 = marker_for_style(list_style("decimal"), 0);
        let result_2 = marker_for_style(list_style("decimal"), 1);
        assert_eq!(result_1, Some(Marker::String("1. ".to_string())));
        assert_eq!(result_2, Some(Marker::String("2. ".to_string())));
    }

    #[test]
    fn test_marker_for_lower_alpha() {
        let result_1 = marker_for_style(list_style("lower-alpha"), 0);
        let result_2 = marker_for_style(list_style("lower-alpha"), 1);
        let result_extended_1 = marker_for_style(list_style("lower-alpha"), 26);
        let result_extended_2 = marker_for_style(list_style("lower-alpha"), 27);
        assert_eq!(result_1, Some(Marker::String("a. ".to_string())));
        assert_eq!(result_2, Some(Marker::String("b. ".to_string())));
        assert_eq!(result_extended_1, Some(Marker::String("aa. ".to_string())));
        assert_eq!(result_extended_2, Some(Marker::String("ab. ".to_string())));
    }

    #[test]
    fn test_marker_for_upper_alpha() {
        let result_1 = marker_for_style(list_style("upper-alpha"), 0);
        let result_2 = marker_for_style(list_style("upper-alpha"), 1);
        let result_extended_1 = marker_for_style(list_style("upper-alpha"), 26);
        let result_extended_2 = marker_for_style(list_style("upper-alpha"), 27);
        assert_eq!(result_1, Some(Marker::String("A. ".to_string())));
        assert_eq!(result_2, Some(Marker::String("B. ".to_string())));
        assert_eq!(result_extended_1, Some(Marker::String("AA. ".to_string())));
        assert_eq!(result_extended_2, Some(Marker::String("AB. ".to_string())));
    }
}
