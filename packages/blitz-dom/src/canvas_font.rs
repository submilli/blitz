//! Canvas font shorthand computation using the document's CSS engine.

use style_traits::ToCss;

use crate::{BaseDocument, NodeId, stylo_to_parley};

/// A computed canvas font snapshot, independent of subsequent element style changes.
#[derive(Clone)]
pub struct CanvasFont {
    pub size: f32,
    pub weight: parley::FontWeight,
    pub style: parley::FontStyle,
    pub width: parley::FontWidth,
    pub families: Vec<parley::FontFamilyName<'static>>,
    pub features: Vec<parley::FontFeature>,
    pub serialized: String,
}

impl Default for CanvasFont {
    fn default() -> Self {
        Self {
            size: 10.0,
            weight: parley::FontWeight::NORMAL,
            style: parley::FontStyle::Normal,
            width: parley::FontWidth::NORMAL,
            families: vec![parley::FontFamilyName::Generic(
                parley::GenericFamily::SansSerif,
            )],
            features: Vec::new(),
            serialized: "10px sans-serif".into(),
        }
    }
}

impl BaseDocument {
    /// Parse and compute an HTML canvas font. Call after flushing styles.
    /// Invalid, unresolved, or over-budget values leave the caller's font unchanged.
    pub fn canvas_font(&self, node: NodeId, text: &str) -> Option<CanvasFont> {
        let computed = self.canvas_computed_style(node, "font", text)?;
        let font = computed.get_font();
        let size = font.font_size.used_size.0.px();
        if !size.is_finite() || !(0.0..=4096.0).contains(&size) {
            return None;
        }
        let families = font.font_family.to_css_string();
        let mut parts = Vec::new();
        // Canvas serializes the default oblique angle as italic. Other oblique
        // angles remain in the shaping snapshot but are omitted by the getter.
        let style = stylo_to_parley::font_style(font.font_style);
        let serialized_style = match style {
            parley::FontStyle::Italic | parley::FontStyle::Oblique(None | Some(14.0)) => "italic",
            _ => "normal",
        };
        for component in [
            serialized_style.into(),
            font.font_variant_caps.to_css_string(),
            if font.font_weight.value() == 700.0 {
                "bold".into()
            } else {
                font.font_weight.to_css_string()
            },
        ] {
            if component != "normal" && component != "400" {
                parts.push(component);
            }
        }
        parts.push(format!("{size}px"));
        parts.push(families.clone());
        Some(CanvasFont {
            size,
            weight: stylo_to_parley::font_weight(font.font_weight),
            style,
            width: stylo_to_parley::font_width(font.font_stretch),
            families: stylo_to_parley::font_families(font),
            features: stylo_to_parley::font_features(font),
            serialized: parts.join(" "),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DocumentConfig, qual_name};

    fn canvas(document: &mut BaseDocument) -> NodeId {
        let root = document.root_node().id;
        let mut mutation = document.mutate();
        let html = mutation.create_element(qual_name!("html", html), vec![]);
        mutation.append_children(root, &[html]);
        let canvas = mutation.create_element(qual_name!("canvas", html), vec![]);
        mutation.append_children(html, &[canvas]);
        canvas
    }

    #[test]
    fn canvas_font_uses_css_computation_and_keeps_a_snapshot() {
        let mut document = BaseDocument::new(DocumentConfig::default());
        let canvas = canvas(&mut document);
        document
            .mutate()
            .set_attribute(canvas, qual_name!("style"), "font-size:20px");
        document.flush_style_and_layout(0.0);
        let snapshot = document.canvas_font(canvas, "120% serif").unwrap();
        assert_eq!(snapshot.size, 24.0);
        assert_eq!(snapshot.serialized, "24px serif");
        assert_eq!(
            document
                .canvas_font(canvas, "calc(10px + 2px) serif")
                .unwrap()
                .size,
            12.0
        );
        document
            .mutate()
            .set_attribute(canvas, qual_name!("style"), "display:none;font-size:30px");
        document.flush_style_and_layout(0.0);
        assert_eq!(
            document.canvas_font(canvas, "1em serif").unwrap().size,
            30.0
        );
        document.mutate().remove_node(canvas);
        let detached = document.canvas_font(canvas, "120% serif").unwrap();
        assert_eq!(detached.size, 12.0);
        assert_eq!(snapshot.size, 24.0);
    }

    #[test]
    fn canvas_font_preserves_decoded_family_names_and_bold_serialization() {
        let mut document = BaseDocument::new(DocumentConfig::default());
        let canvas = canvas(&mut document);
        let font = document
            .canvas_font(canvas, r#"italic bold 12pt "A\"B\\C", serif"#)
            .unwrap();
        assert_eq!(font.serialized, r#"italic bold 16px "A\"B\\C", serif"#);
        assert_eq!(
            font.families[0],
            parley::FontFamilyName::Named("A\"B\\C".into())
        );
        assert_eq!(
            font.families[1],
            parley::FontFamilyName::Generic(parley::GenericFamily::Serif)
        );
        assert_eq!(font.style, parley::FontStyle::Italic);
        assert_eq!(font.weight, parley::FontWeight::BOLD);
    }

    #[test]
    fn canvas_and_css_system_fonts_share_resolution_and_preserve_role_names() {
        let mut document = BaseDocument::new(DocumentConfig::default());
        let canvas = canvas(&mut document);
        for role in [
            "caption",
            "icon",
            "menu",
            "message-box",
            "small-caption",
            "status-bar",
        ] {
            let declaration = format!("font:{role}");
            assert_eq!(document.style_attr_get_property(&declaration, "font"), role);
            document
                .mutate()
                .set_attribute(canvas, qual_name!("style"), &declaration);
            document.flush_style_and_layout(0.0);
            assert_eq!(document.resolved_style_value(canvas, "font-size"), "16px");
            assert_eq!(
                document.resolved_style_value(canvas, "font-family"),
                "Arial"
            );
            let font = document.canvas_font(canvas, role).unwrap();
            assert_eq!(font.serialized, "16px Arial");
            assert_eq!(font.weight, parley::FontWeight::NORMAL);
            assert_eq!(font.style, parley::FontStyle::Normal);
        }
        for role in ["CAPTION", "/**/menu", r"\63 aption"] {
            assert_eq!(
                document.canvas_font(canvas, role).unwrap().serialized,
                "16px Arial"
            );
        }
        for invalid in ["caption serif", "menu 12px", "menu; color:red"] {
            assert!(document.canvas_font(canvas, invalid).is_none());
        }
    }

    #[test]
    fn system_font_serialization_rejects_longhand_overrides() {
        let document = BaseDocument::new(DocumentConfig::default());
        for property in [
            "font-kerning:none",
            "font-feature-settings:'liga' 0",
            "font-variant-caps:small-caps",
            "font-optical-sizing:none",
            "font-size:20px",
            "line-height:2",
            "font-weight:700",
        ] {
            let declaration = format!("font:menu;{property}");
            assert_eq!(
                document.style_attr_get_property(&declaration, "font"),
                "",
                "{property}"
            );
        }
    }

    #[test]
    fn system_font_computed_values_inherit_and_ordinary_shorthands_clear_roles() {
        let mut document = BaseDocument::new(DocumentConfig::default());
        let canvas = canvas(&mut document);
        let child = {
            let mut mutation = document.mutate();
            mutation.set_attribute(canvas, qual_name!("style"), "font:menu");
            let child = mutation.create_element(qual_name!("span", html), vec![]);
            mutation.set_attribute(child, qual_name!("style"), "font-size:200%");
            mutation.append_children(canvas, &[child]);
            child
        };
        document.flush_style_and_layout(0.0);
        assert_eq!(document.resolved_style_value(child, "font-size"), "32px");
        assert_eq!(document.resolved_style_value(child, "font-family"), "Arial");
        let declaration = "font:menu;font:italic 24px serif";
        document
            .mutate()
            .set_attribute(canvas, qual_name!("style"), declaration);
        document.flush_style_and_layout(0.0);
        assert_eq!(
            document.style_attr_get_property(declaration, "font"),
            "italic 24px serif"
        );
        assert_eq!(
            document.resolved_style_value(canvas, "font-family"),
            "serif"
        );
        assert_eq!(document.resolved_style_value(child, "font-size"), "48px");
    }

    #[test]
    fn canvas_font_rejects_invalid_unresolved_and_excessive_values() {
        let mut document = BaseDocument::new(DocumentConfig::default());
        let canvas = canvas(&mut document);
        for value in [
            "italic italic 12px serif",
            "12px serif; color:red",
            "inherit",
            "var(--font)",
            "5000px serif",
            "-2px serif",
        ] {
            assert!(document.canvas_font(canvas, value).is_none(), "{value}");
        }
        assert!(document.canvas_font(canvas, &" ".repeat(4097)).is_none());
        assert_eq!(document.canvas_font(canvas, "0px serif").unwrap().size, 0.0);
    }
}
