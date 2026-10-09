//! Live DOM filter snapshots. SVG parsing and unit resolution stay in the engine.

use crate::canvas_svg_attributes::{Attribute, chrome_attribute};
use crate::{BaseDocument, CanvasFilter, CanvasFilters, NodeId};
use std::borrow::Cow;
use std::fmt::Write;
use std::sync::Arc;
use style_traits::ToCss;

const MAX_VISITS: usize = 4096;
const MAX_NODES: usize = 64;
const MAX_MARKUP: usize = 16 * 1024;
const SNAPSHOT_ID: &str = "canvas-filter";

/// A draw-time snapshot contains no DOM handles and cannot retain removed nodes.
#[derive(Clone, Debug)]
pub struct CanvasSvgFilter {
    markup: String,
    /// Per filter primitive, in document order: whether its own paint depends on
    /// currentColor. Chrome taints a canvas only when such paint reaches the
    /// filter output, and passes displacement through when its map is tainted.
    primitive_taints: Vec<bool>,
    /// `primitiveUnits="objectBoundingBox"` on the filter element.
    bounding_box_units: bool,
}

/// Primitive coordinates that the engine's parser leaves in primitive units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanvasSvgUnits {
    /// The filtered bounds `[x, y, width, height]` for bounding-box units.
    bounding_box: Option<[f32; 4]>,
}

impl CanvasSvgUnits {
    /// Light positions as user-space coordinates. Bounding-box units scale x and y
    /// by the box and z by its normalized diagonal, as Chrome resolves them.
    /// <https://drafts.fxtf.org/filter-effects/#element-attrdef-fepointlight-z>
    pub fn light(&self, light: usvg::filter::LightSource) -> usvg::filter::LightSource {
        use usvg::filter::LightSource;
        let Some([x, y, width, height]) = self.bounding_box else {
            return light;
        };
        let depth = ((width * width + height * height) / 2.0).sqrt();
        let point = |px: f32, py: f32, pz: f32| (x + px * width, y + py * height, pz * depth);
        match light {
            LightSource::DistantLight(light) => LightSource::DistantLight(light),
            LightSource::PointLight(mut light) => {
                (light.x, light.y, light.z) = point(light.x, light.y, light.z);
                LightSource::PointLight(light)
            }
            LightSource::SpotLight(mut light) => {
                (light.x, light.y, light.z) = point(light.x, light.y, light.z);
                (light.points_at_x, light.points_at_y, light.points_at_z) =
                    point(light.points_at_x, light.points_at_y, light.points_at_z);
                LightSource::SpotLight(light)
            }
        }
    }

    /// The parser scales a bounding-box displacement by the mean box side; Chrome
    /// uses the box width.
    pub fn displacement_scale(&self, parsed: f32) -> f32 {
        match self.bounding_box {
            Some([_, _, width, height]) if width + height > 0.0 => {
                parsed / ((width + height) / 2.0) * width
            }
            _ => parsed,
        }
    }
}

/// Page-controlled graph or serialization limits are checked before retaining data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CanvasSvgLimit;

impl CanvasSvgFilter {
    /// Units of primitive coordinates for `bounds` (`[x, y, right, bottom]`), the
    /// same bounds passed to [`Self::resolve`].
    #[expect(
        clippy::cast_possible_truncation,
        reason = "Resolved bounds are finite and at most 1e6 in magnitude."
    )]
    pub fn units(&self, bounds: [f64; 4]) -> CanvasSvgUnits {
        let [x, y, right, bottom] = bounds.map(|v| v as f32);
        CanvasSvgUnits {
            bounding_box: self
                .bounding_box_units
                .then_some([x, y, right - x, bottom - y]),
        }
    }

    /// Whether each primitive's own paint depends on currentColor, in document order.
    pub fn primitive_taints(&self) -> &[bool] {
        &self.primitive_taints
    }

    /// Resolve filterUnits/primitiveUnits against the geometric source bounds.
    /// The parser performs no I/O: images and resource references are disabled.
    pub fn resolve(
        &self,
        bounds: [f64; 4],
        viewport: [u16; 2],
    ) -> Option<Arc<usvg::filter::Filter>> {
        if !bounds.into_iter().all(|n| n.is_finite() && n.abs() <= 1e6)
            || bounds[2] <= bounds[0]
            || bounds[3] <= bounds[1]
        {
            return None;
        }
        let [x, y, right, bottom] = bounds;
        let mut svg = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\"><defs>{}</defs><rect x=\"{x}\" y=\"{y}\" width=\"{}\" height=\"{}\" filter=\"url(#",
            viewport[0],
            viewport[1],
            self.markup,
            right - x,
            bottom - y
        );
        svg.push_str(SNAPSHOT_ID);
        svg.push_str(")\"/></svg>");
        let options = usvg::Options {
            image_href_resolver: usvg::ImageHrefResolver {
                resolve_data: Box::new(|_, _, _| None),
                resolve_string: Box::new(|_, _| None),
            },
            ..usvg::Options::default()
        };
        let tree = usvg::Tree::from_str(&svg, &options).ok()?;
        tree.filters()
            .iter()
            .find(|f| f.id() == SNAPSHOT_ID)
            .cloned()
    }
}

impl BaseDocument {
    /// Refresh every URL immediately before a draw, including saved/restored lists.
    /// Unresolved/external URLs remain unapplied and never trigger ambient fetching.
    pub fn resolve_canvas_svg_filters(
        &self,
        owner: NodeId,
        filters: &mut CanvasFilters,
    ) -> Result<(), CanvasSvgLimit> {
        let mut snapshots = Vec::with_capacity(filters.operations.len());
        for operation in &filters.operations {
            snapshots.push(match operation {
                CanvasFilter::Reference(url) => self.canvas_svg_filter(owner, url)?,
                _ => None,
            });
        }
        filters.svg = snapshots;
        Ok(())
    }

    fn canvas_svg_filter(
        &self,
        owner: NodeId,
        reference: &str,
    ) -> Result<Option<CanvasSvgFilter>, CanvasSvgLimit> {
        let Ok(url) = url::Url::parse(reference) else {
            return Ok(None);
        };
        let document_url = self.document_url(owner);
        if url[..url::Position::AfterQuery] != document_url[..url::Position::AfterQuery] {
            return Ok(None);
        }
        let Some(fragment) = url.fragment() else {
            return Ok(None);
        };
        if fragment.len() > 4096 {
            return Err(CanvasSvgLimit);
        }
        let id = percent_encoding::percent_decode_str(fragment).decode_utf8_lossy();
        let Some(filter) = self.find_canvas_filter(owner, &id)? else {
            return Ok(None);
        };
        self.snapshot_canvas_filter(filter)
    }

    fn find_canvas_filter(
        &self,
        owner: NodeId,
        id: &str,
    ) -> Result<Option<NodeId>, CanvasSvgLimit> {
        let mut visits = 0;
        let mut tree = owner;
        while let Some(parent) = self.nodes[tree].parent {
            visits += 1;
            if visits > MAX_VISITS {
                return Err(CanvasSvgLimit);
            }
            tree = parent;
        }
        let root = if self.nodes[tree].is_shadow_root() {
            tree
        } else {
            self.node_document(owner)
        };
        let mut pending = vec![root];
        while let Some(node_id) = pending.pop() {
            visits += 1;
            if visits > MAX_VISITS {
                return Err(CanvasSvgLimit);
            }
            let node = &self.nodes[node_id];
            if let Some(element) = node.element_data() {
                for attr in element.attrs() {
                    visits += 1;
                    if visits > MAX_VISITS {
                        return Err(CanvasSvgLimit);
                    }
                    if attr.name.ns.as_ref().is_empty()
                        && attr.name.local.as_ref() == "id"
                        && attr.value.utf16_len_bound() <= 4096
                        && attr.value.as_str_lossy() == id
                    {
                        return Ok(Some(node_id));
                    }
                }
            }
            if pending.len() + node.children.len() > MAX_VISITS {
                return Err(CanvasSvgLimit);
            }
            pending.extend(node.children.iter().rev().copied());
        }
        Ok(None)
    }

    fn snapshot_canvas_filter(
        &self,
        filter: NodeId,
    ) -> Result<Option<CanvasSvgFilter>, CanvasSvgLimit> {
        let Some(element) = self.nodes[filter].element_data() else {
            return Ok(None);
        };
        if element.name.local.as_ref() != "filter"
            || element.name.ns.as_ref() != "http://www.w3.org/2000/svg"
        {
            return Ok(None);
        }
        let mut snapshot = CanvasSvgFilter {
            markup: String::new(),
            primitive_taints: Vec::new(),
            bounding_box_units: false,
        };
        let mut pending = vec![(filter, false)];
        let mut nodes = 0;
        let mut attributes = 0;
        while let Some((node_id, closing)) = pending.pop() {
            let node = &self.nodes[node_id];
            let Some(element) = node.element_data() else {
                continue;
            };
            let tag = element.name.local.as_ref();
            if element.name.ns.as_ref() != "http://www.w3.org/2000/svg"
                || matches!(tag, "title" | "desc" | "metadata")
            {
                continue;
            }
            if closing {
                write!(&mut snapshot.markup, "</{tag}>").map_err(|_| CanvasSvgLimit)?;
                if snapshot.markup.len() > MAX_MARKUP {
                    return Err(CanvasSvgLimit);
                }
                continue;
            }
            nodes += 1;
            if nodes > MAX_NODES || pending.len() + node.children.len() > MAX_NODES * 2 {
                return Err(CanvasSvgLimit);
            }
            // No geometry, scripts, nested documents or resource-bearing primitives
            // can enter the synthetic parser document.
            if !allowed_tag(tag) {
                return Ok(None);
            }
            let primitive = node.parent == Some(filter) && is_primitive(tag);
            if primitive {
                snapshot.primitive_taints.push(false);
            }
            write!(&mut snapshot.markup, "<{tag}").map_err(|_| CanvasSvgLimit)?;
            if node_id == filter {
                write!(&mut snapshot.markup, " id=\"{SNAPSHOT_ID}\"")
                    .map_err(|_| CanvasSvgLimit)?;
            }
            for attr in element.attrs() {
                attributes += 1;
                if attributes > MAX_VISITS {
                    return Err(CanvasSvgLimit);
                }
                let name = attr.name.local.as_ref();
                if node_id == filter && name == "id" {
                    continue;
                }
                if !attr.name.ns.as_ref().is_empty() || name == "xmlns" || name.starts_with("on") {
                    continue;
                }
                if matches!(
                    name,
                    "color"
                        | "style"
                        | "flood-color"
                        | "flood-opacity"
                        | "lighting-color"
                        | "color-interpolation-filters"
                ) {
                    continue;
                }
                if name == "href" {
                    return Ok(None);
                }
                if attr.value.utf16_len_bound() > 4096 {
                    return Err(CanvasSvgLimit);
                }
                let value = attr.value.as_str_lossy();
                if value.len() > 4096 {
                    return Err(CanvasSvgLimit);
                }
                if node_id == filter && name == "primitiveUnits" {
                    snapshot.bounding_box_units = value == "objectBoundingBox";
                }
                let value: Cow<'_, str> = match chrome_attribute(tag, name, value) {
                    Attribute::Keep(value) => Cow::Borrowed(value),
                    Attribute::Replace(value) => Cow::Owned(value),
                    Attribute::Omit => continue,
                };
                if snapshot.markup.len() + value.len() * 6 + name.len() + 4 > MAX_MARKUP {
                    return Err(CanvasSvgLimit);
                }
                write!(&mut snapshot.markup, " {name}=\"").map_err(|_| CanvasSvgLimit)?;
                escape(&mut snapshot.markup, &value);
                snapshot.markup.push('"');
            }
            if let Some(style) = node
                .primary_styles()
                .map(|s| s.clone())
                .or_else(|| self.resolve_undisplayed_style(node_id))
            {
                let color = style.clone_color();
                let flood = style.clone_flood_color();
                let lighting = style.clone_lighting_color();
                let tainted = match tag {
                    "feFlood" | "feDropShadow" => !flood.is_absolute(),
                    "feDiffuseLighting" | "feSpecularLighting" => !lighting.is_absolute(),
                    _ => false,
                };
                if primitive && let Some(last) = snapshot.primitive_taints.last_mut() {
                    *last = tainted;
                }
                let flood = svg_color(&flood.resolve_to_absolute(&color));
                let lighting = svg_color(&lighting.resolve_to_absolute(&color));
                let interpolation = style.clone_color_interpolation_filters().to_css_string();
                let interpolation = if interpolation.eq_ignore_ascii_case("srgb") {
                    "sRGB"
                } else {
                    "linearRGB"
                };
                snapshot.markup.push_str(" style=\"color:");
                escape(&mut snapshot.markup, &svg_color(&color));
                snapshot.markup.push_str(";flood-color:");
                escape(&mut snapshot.markup, &flood);
                snapshot.markup.push_str(";lighting-color:");
                escape(&mut snapshot.markup, &lighting);
                write!(
                    &mut snapshot.markup,
                    ";flood-opacity:{};color-interpolation-filters:{interpolation}\"",
                    style.clone_flood_opacity()
                )
                .map_err(|_| CanvasSvgLimit)?;
            }
            snapshot.markup.push('>');
            if snapshot.markup.len() > MAX_MARKUP {
                return Err(CanvasSvgLimit);
            }
            pending.push((node_id, true));
            pending.extend(node.children.iter().rev().map(|child| (*child, false)));
        }
        Ok(Some(snapshot))
    }
}

fn svg_color(color: &style::color::AbsoluteColor) -> String {
    use crate::util::ToColorColor;
    let color = color.as_color_color();
    let [r, g, b, a] = color.components;
    format!(
        "rgba({},{},{},{})",
        (r.clamp(0.0, 1.0) * 255.0).round(),
        (g.clamp(0.0, 1.0) * 255.0).round(),
        (b.clamp(0.0, 1.0) * 255.0).round(),
        a.clamp(0.0, 1.0)
    )
}

fn allowed_tag(tag: &str) -> bool {
    matches!(
        tag,
        "filter"
            | "feColorMatrix"
            | "feComponentTransfer"
            | "feFuncR"
            | "feFuncG"
            | "feFuncB"
            | "feFuncA"
            | "feGaussianBlur"
            | "feOffset"
            | "feFlood"
            | "feComposite"
            | "feBlend"
            | "feMerge"
            | "feMergeNode"
            | "feDropShadow"
            | "feMorphology"
            | "feConvolveMatrix"
            | "feDisplacementMap"
            | "feTurbulence"
            | "feTile"
            | "feDiffuseLighting"
            | "feSpecularLighting"
            | "feDistantLight"
            | "fePointLight"
            | "feSpotLight"
    )
}

/// Filter children that the parser turns into graph primitives, in document order.
fn is_primitive(tag: &str) -> bool {
    tag.starts_with("fe")
        && !matches!(
            tag,
            "feFuncR"
                | "feFuncG"
                | "feFuncB"
                | "feFuncA"
                | "feMergeNode"
                | "feDistantLight"
                | "fePointLight"
                | "feSpotLight"
        )
}

fn escape(output: &mut String, text: &str) {
    for ch in text.chars() {
        output.push_str(match ch {
            '&' => "&amp;",
            '<' => "&lt;",
            '>' => "&gt;",
            '"' => "&quot;",
            _ => {
                output.push(ch);
                continue;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CanvasFont, DocumentConfig, QualName, qual_name};

    fn document() -> (BaseDocument, NodeId, NodeId, NodeId) {
        let mut document = BaseDocument::new(DocumentConfig {
            base_url: Some("https://example.test/page".into()),
            ..Default::default()
        });
        let root = document.root_node_id;
        let mut mutation = document.mutate();
        let html = mutation.create_element(qual_name!("html", html), vec![]);
        mutation.append_children(root, &[html]);
        let canvas = mutation.create_element(qual_name!("canvas", html), vec![]);
        let svg_name = |name: &str| QualName {
            prefix: None,
            ns: crate::Namespace::from("http://www.w3.org/2000/svg"),
            local: name.into(),
        };
        let filter = mutation.create_element(svg_name("filter"), vec![]);
        let matrix = mutation.create_element(svg_name("feColorMatrix"), vec![]);
        mutation.set_attribute(filter, qual_name!("id"), "effect");
        mutation.set_attribute(
            matrix,
            qual_name!("values"),
            "0.5 0 0 0 0 0 1 0 0 0 0 0 1 0 0 0 0 0 1 0",
        );
        mutation.append_children(html, &[filter]);
        mutation.append_children(filter, &[matrix]);
        drop(mutation);
        document.flush_style_and_layout(0.0);
        (document, canvas, filter, matrix)
    }

    #[test]
    fn references_refresh_detached_canvas_graphs_and_resolve_object_bounds() {
        let (mut document, canvas, filter, matrix) = self::document();
        assert_eq!(
            document.query_selector("feColorMatrix").unwrap(),
            Some(matrix)
        );
        assert_eq!(document.query_selector("fecolormatrix").unwrap(), None);
        let mut filters = document
            .canvas_filters(canvas, "url(#effect) opacity(.5)", &CanvasFont::default())
            .unwrap();
        document
            .resolve_canvas_svg_filters(canvas, &mut filters)
            .unwrap();
        let saved = filters.clone();
        let parsed = filters.svg[0]
            .as_ref()
            .unwrap()
            .resolve([10.0, 20.0, 30.0, 40.0], [100, 100])
            .unwrap();
        assert_eq!(
            (
                parsed.rect().x(),
                parsed.rect().y(),
                parsed.rect().width(),
                parsed.rect().height()
            ),
            (8.0, 18.0, 24.0, 24.0)
        );
        document.mutate().set_attribute(
            matrix,
            qual_name!("values"),
            "1 0 0 0 0 0 1 0 0 0 0 0 1 0 0 0 0 0 1 0",
        );
        document.flush_style_and_layout(0.0);
        document
            .resolve_canvas_svg_filters(canvas, &mut filters)
            .unwrap();
        assert_ne!(
            filters.svg[0].as_ref().unwrap().markup,
            saved.svg[0].as_ref().unwrap().markup
        );
        document.mutate().remove_node(filter);
        document
            .resolve_canvas_svg_filters(canvas, &mut filters)
            .unwrap();
        assert!(filters.svg[0].is_none());
    }

    fn svg(
        mutation: &mut crate::DocumentMutator<'_>,
        name: &str,
        attrs: &[(&str, &str)],
    ) -> NodeId {
        let node = mutation.create_element(
            QualName {
                prefix: None,
                ns: crate::Namespace::from("http://www.w3.org/2000/svg"),
                local: name.into(),
            },
            vec![],
        );
        for (attr, value) in attrs {
            mutation.set_attribute(
                node,
                QualName {
                    prefix: None,
                    ns: crate::Namespace::from(""),
                    local: (*attr).into(),
                },
                *value,
            );
        }
        node
    }

    #[test]
    fn primitive_families_resolve_with_lighting_color_and_aligned_taints() {
        let (mut document, canvas, filter, matrix) = self::document();
        let mut mutation = document.mutate();
        mutation.remove_node(matrix);
        let morphology = svg(&mut mutation, "feMorphology", &[("radius", "0 2")]);
        let lighting = svg(
            &mut mutation,
            "feDiffuseLighting",
            &[("lighting-color", "currentColor")],
        );
        let light = svg(&mut mutation, "feDistantLight", &[("elevation", "45")]);
        let flood = svg(&mut mutation, "feFlood", &[("flood-color", "red")]);
        let turbulence = svg(&mut mutation, "feTurbulence", &[("baseFrequency", "0.1")]);
        let specular = svg(
            &mut mutation,
            "feSpecularLighting",
            &[
                ("lighting-color", "rgb(0, 255, 0)"),
                ("specularExponent", "500"),
            ],
        );
        let point = svg(&mut mutation, "fePointLight", &[("z", "3")]);
        mutation.set_attribute(filter, qual_name!("color"), "blue");
        mutation.append_children(lighting, &[light]);
        mutation.append_children(specular, &[point]);
        mutation.append_children(filter, &[morphology, lighting, flood, turbulence, specular]);
        drop(mutation);
        document.flush_style_and_layout(0.0);
        let mut filters = document
            .canvas_filters(canvas, "url(#effect)", &CanvasFont::default())
            .unwrap();
        document
            .resolve_canvas_svg_filters(canvas, &mut filters)
            .unwrap();
        let snapshot = filters.svg[0].as_ref().unwrap();
        // Light sources are not primitives, so taints stay aligned with the parser.
        assert_eq!(
            snapshot.primitive_taints(),
            [false, true, false, false, false]
        );
        let parsed = snapshot.resolve([0.0, 0.0, 8.0, 8.0], [8, 8]).unwrap();
        let primitives = parsed.primitives();
        assert_eq!(primitives.len(), 5);
        let usvg::filter::Kind::Morphology(morphology) = primitives[0].kind() else {
            panic!("morphology")
        };
        // A zero axis rounds to zero instead of the parser's default of one.
        assert!(morphology.radius_x().get() < 0.5);
        let usvg::filter::Kind::DiffuseLighting(diffuse) = primitives[1].kind() else {
            panic!("diffuse lighting")
        };
        assert_eq!(diffuse.lighting_color(), usvg::Color::new_rgb(0, 0, 255));
        let usvg::filter::Kind::SpecularLighting(specular) = primitives[4].kind() else {
            panic!("specular lighting: out-of-range exponents clamp instead of dropping")
        };
        assert_eq!(specular.lighting_color(), usvg::Color::new_rgb(0, 255, 0));
        assert_eq!(specular.specular_exponent(), 128.0);
    }

    #[test]
    fn image_inputs_remain_unresolved() {
        let (mut document, canvas, filter, _) = self::document();
        let mut mutation = document.mutate();
        let image = svg(
            &mut mutation,
            "feImage",
            &[("href", "data:image/png;base64,AA==")],
        );
        mutation.append_children(filter, &[image]);
        drop(mutation);
        let mut filters = document
            .canvas_filters(canvas, "url(#effect)", &CanvasFont::default())
            .unwrap();
        document
            .resolve_canvas_svg_filters(canvas, &mut filters)
            .unwrap();
        assert!(filters.svg[0].is_none());
    }

    #[test]
    fn external_and_missing_references_do_not_acquire_local_graphs() {
        let (document, canvas, _, _) = self::document();
        for text in [
            "url(#missing)",
            "url(https://other.test/page#effect)",
            "url(https://example.test/other#effect)",
        ] {
            let mut filters = document
                .canvas_filters(canvas, text, &CanvasFont::default())
                .unwrap();
            document
                .resolve_canvas_svg_filters(canvas, &mut filters)
                .unwrap();
            assert!(filters.svg[0].is_none(), "{text}");
        }
    }

    #[test]
    fn graph_snapshots_bound_attribute_bytes_and_child_count() {
        let (mut document, canvas, filter, matrix) = self::document();
        let mut filters = document
            .canvas_filters(canvas, "url(#effect)", &CanvasFont::default())
            .unwrap();
        document
            .mutate()
            .set_attribute(matrix, qual_name!("values"), "0 ".repeat(8192));
        assert_eq!(
            document.resolve_canvas_svg_filters(canvas, &mut filters),
            Err(CanvasSvgLimit)
        );
        document
            .mutate()
            .set_attribute(matrix, qual_name!("values"), "");
        for _ in 0..65 {
            let mut mutation = document.mutate();
            let child = mutation.create_element(
                QualName {
                    prefix: None,
                    ns: crate::Namespace::from("http://www.w3.org/2000/svg"),
                    local: "feOffset".into(),
                },
                vec![],
            );
            mutation.append_children(filter, &[child]);
        }
        assert_eq!(
            document.resolve_canvas_svg_filters(canvas, &mut filters),
            Err(CanvasSvgLimit)
        );
    }
    #[test]
    fn graph_lookup_charges_owner_ancestry_and_inspected_attributes() {
        let (mut document, canvas, _, _) = self::document();
        let mut filters = document
            .canvas_filters(canvas, "url(#effect)", &CanvasFont::default())
            .unwrap();
        let mut parent = canvas;
        for _ in 0..MAX_VISITS + 1 {
            let mut mutation = document.mutate();
            let ancestor = mutation.create_element(qual_name!("div", html), vec![]);
            mutation.append_children(ancestor, &[parent]);
            parent = ancestor;
        }
        assert_eq!(
            document.resolve_canvas_svg_filters(canvas, &mut filters),
            Err(CanvasSvgLimit)
        );
        let (mut document, canvas, _, _) = self::document();
        let html = document.root_node().children[0];
        for index in 0..MAX_VISITS + 1 {
            document.mutate().set_attribute(
                html,
                QualName {
                    prefix: None,
                    ns: crate::Namespace::from(""),
                    local: format!("data-{index}").into(),
                },
                "",
            );
        }
        let mut filters = document
            .canvas_filters(canvas, "url(#effect)", &CanvasFont::default())
            .unwrap();
        assert_eq!(
            document.resolve_canvas_svg_filters(canvas, &mut filters),
            Err(CanvasSvgLimit)
        );
    }
}
