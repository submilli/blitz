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
            parsed.filter.rect().x(),
            parsed.filter.rect().y(),
            parsed.filter.rect().width(),
            parsed.filter.rect().height()
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

fn svg(mutation: &mut crate::DocumentMutator<'_>, name: &str, attrs: &[(&str, &str)]) -> NodeId {
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
    let resolved = snapshot.resolve([0.0, 0.0, 8.0, 8.0], [8, 8]).unwrap();
    let primitives = resolved.filter.primitives();
    assert_eq!(primitives.len(), 5);
    // Light sources are not primitives, so taints stay aligned with the parser;
    // a primitive without a flag fails closed.
    let taints: Vec<bool> = (0..6).map(|index| resolved.tainted(index)).collect();
    assert_eq!(taints, [false, true, false, false, false, true]);
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

/// The attribute rewrites rely on these parser outcomes.
#[test]
fn rewritten_attributes_produce_chrome_results_through_the_parser() {
    let (mut document, canvas, filter, matrix) = self::document();
    let mut mutation = document.mutate();
    mutation.remove_node(matrix);
    let zero = svg(&mut mutation, "feMorphology", &[("radius", "0 1")]);
    let invalid = svg(
        &mut mutation,
        "feConvolveMatrix",
        &[("order", "-3"), ("kernelMatrix", "0 0 0 0 1 0 0 0 0")],
    );
    let target = svg(
        &mut mutation,
        "feConvolveMatrix",
        &[("kernelMatrix", "0 0 0 0 1 0 0 0 0"), ("targetX", "1.5")],
    );
    let noise = svg(
        &mut mutation,
        "feTurbulence",
        &[("numOctaves", "2.5"), ("baseFrequency", "1e39")],
    );
    let huge_order = svg(
        &mut mutation,
        "feConvolveMatrix",
        &[("order", "2147483647 2147483647 0"), ("kernelMatrix", "1")],
    );
    mutation.append_children(filter, &[zero, invalid, target, noise, huge_order]);
    drop(mutation);
    document.flush_style_and_layout(0.0);
    let mut filters = document
        .canvas_filters(canvas, "url(#effect)", &CanvasFont::default())
        .unwrap();
    document
        .resolve_canvas_svg_filters(canvas, &mut filters)
        .unwrap();
    let resolved = filters.svg[0]
        .as_ref()
        .unwrap()
        .resolve([0.0, 0.0, 8.0, 8.0], [8, 8])
        .unwrap();
    let kinds: Vec<_> = resolved
        .filter
        .primitives()
        .iter()
        .map(|p| p.kind().clone())
        .collect();
    let usvg::filter::Kind::Morphology(morphology) = &kinds[0] else {
        panic!("morphology")
    };
    assert!(morphology.radius_x().get() < 0.5 && morphology.radius_y().get() == 1.0);
    // An invalid kernel becomes the parser's transparent stand-in.
    assert!(matches!(&kinds[1], usvg::filter::Kind::Flood(flood) if flood.opacity().get() == 0.0));
    let usvg::filter::Kind::ConvolveMatrix(convolve) = &kinds[2] else {
        panic!("convolve")
    };
    assert_eq!(convolve.matrix().target_x(), 0);
    let usvg::filter::Kind::Turbulence(turbulence) = &kinds[3] else {
        panic!("turbulence")
    };
    assert_eq!(turbulence.num_octaves(), 1);
    assert_eq!(turbulence.base_frequency_x().get(), 0.0);
    // A malformed order never reaches the parser's unchecked product.
    assert!(matches!(&kinds[4], usvg::filter::Kind::Flood(flood) if flood.opacity().get() == 0.0));
}

/// Bounding-box scaling of a huge radius must stay finite inside the parser.
#[test]
fn bounding_box_radii_cannot_overflow_the_parser() {
    let (mut document, canvas, filter, matrix) = self::document();
    let mut mutation = document.mutate();
    mutation.remove_node(matrix);
    mutation.set_attribute(filter, qual_name!("primitiveUnits"), "objectBoundingBox");
    let morphology = svg(&mut mutation, "feMorphology", &[("radius", "3e38")]);
    mutation.append_children(filter, &[morphology]);
    drop(mutation);
    document.flush_style_and_layout(0.0);
    let mut filters = document
        .canvas_filters(canvas, "url(#effect)", &CanvasFont::default())
        .unwrap();
    document
        .resolve_canvas_svg_filters(canvas, &mut filters)
        .unwrap();
    let snapshot = filters.svg[0].as_ref().unwrap();
    let resolved = snapshot.resolve([0.0, 0.0, 1e6, 1e6], [8, 8]).unwrap();
    let usvg::filter::Kind::Morphology(morphology) = resolved.filter.primitives()[0].kind() else {
        panic!("morphology")
    };
    assert!(morphology.radius_x().get().is_finite());
}

/// Snapshot `filter`'s graph for `canvas` and resolve it against a 4×4 bitmap.
fn resolved(document: &mut BaseDocument, canvas: NodeId) -> ResolvedCanvasSvgFilter {
    document.flush_style_and_layout(0.0);
    let mut filters = document
        .canvas_filters(canvas, "url(#effect)", &CanvasFont::default())
        .unwrap();
    document
        .resolve_canvas_svg_filters(canvas, &mut filters)
        .unwrap();
    filters.svg[0]
        .as_ref()
        .expect("feImage graphs resolve")
        .resolve([0.0, 0.0, 4.0, 4.0], [4, 4])
        .unwrap()
}

#[test]
fn image_inputs_capture_their_source_aspect_and_taint() {
    use crate::canvas_svg_image::CanvasSvgImageSource as Source;
    let (mut document, canvas, filter, matrix) = self::document();
    let mut mutation = document.mutate();
    mutation.remove_node(matrix);
    // A 1×1 PNG, padded past the 4096-unit bound on serialized attributes.
    let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";
    let href = format!("data:image/png;base64,{png}{}", " ".repeat(5000));
    let image = svg(
        &mut mutation,
        "feImage",
        &[
            ("href", &href),
            ("x", "1"),
            ("width", "2"),
            ("preserveAspectRatio", "xMinYMax slice"),
            ("result", "i"),
        ],
    );
    let element = svg(&mut mutation, "feImage", &[("href", "#shape")]);
    let missing = svg(&mut mutation, "feImage", &[]);
    let pending = svg(
        &mut mutation,
        "feImage",
        &[("href", "https://cdn.test/a.png")],
    );
    mutation.append_children(filter, &[image, element, missing, pending]);
    let shape = svg(
        &mut mutation,
        "rect",
        &[
            ("id", "shape"),
            ("width", "2"),
            ("height", "2"),
            ("fill", "lime"),
        ],
    );
    let parent = mutation.doc.nodes[filter].parent.unwrap();
    mutation.append_children(parent, &[shape]);
    drop(mutation);

    let resolved = resolved(&mut document, canvas);
    let primitives = resolved.filter.primitives();
    assert_eq!(primitives.len(), 4);
    // The parser sees each `feImage` as a transparent flood over its subregion.
    let usvg::filter::Kind::Flood(flood) = primitives[0].kind() else {
        panic!("feImage without its href");
    };
    assert_eq!(flood.opacity().get(), 0.0);
    let rect = primitives[0].rect();
    assert_eq!((rect.x(), rect.width()), (1.0, 2.0));
    assert_eq!(primitives[0].result(), "i");

    let image = resolved.image(0).unwrap();
    assert!(matches!(&image.source, Source::Raster(raster) if raster.width == 1));
    assert_eq!(
        (image.aspect.align, image.aspect.slice),
        (svgtypes::Align::XMinYMax, true)
    );
    assert!(!resolved.tainted(0), "data: images are readable");
    let Source::Element(element) = &resolved.image(1).unwrap().source else {
        panic!("element references capture the element");
    };
    assert!(element.tree.root().has_children());
    assert_eq!((element.bounds.x(), element.bounds.width()), (0.0, 2.0));
    assert!(resolved.tainted(1), "element references taint");
    assert!(matches!(resolved.image(2).unwrap().source, Source::None));
    assert!(resolved.tainted(2), "an feImage without an image taints");
    assert!(matches!(resolved.image(3).unwrap().source, Source::None));
    assert!(
        !resolved.tainted(3),
        "a pending fetch draws nothing readable"
    );
}

#[test]
fn element_references_bound_their_serialized_subtree() {
    let (mut document, canvas, filter, matrix) = self::document();
    let mut mutation = document.mutate();
    mutation.remove_node(matrix);
    let image = svg(&mut mutation, "feImage", &[("href", "#group")]);
    mutation.append_children(filter, &[image]);
    let parent = mutation.doc.nodes[filter].parent.unwrap();
    let group = svg(&mut mutation, "g", &[("id", "group")]);
    mutation.append_children(parent, &[group]);
    for _ in 0..2000 {
        let child = svg(&mut mutation, "rect", &[]);
        mutation.append_children(group, &[child]);
    }
    drop(mutation);
    document.flush_style_and_layout(0.0);
    let mut filters = document
        .canvas_filters(canvas, "url(#effect)", &CanvasFont::default())
        .unwrap();
    document
        .resolve_canvas_svg_filters(canvas, &mut filters)
        .unwrap();
    let resolved = filters.svg[0]
        .as_ref()
        .unwrap()
        .resolve([0.0, 0.0, 4.0, 4.0], [4, 4])
        .unwrap();
    // Past the bounds the reference draws nothing, and still taints.
    assert!(matches!(
        resolved.image(0).unwrap().source,
        crate::canvas_svg_image::CanvasSvgImageSource::None
    ));
    assert!(resolved.tainted(0));
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

/// Bounding-box units multiply region lengths in the parser, which unwraps the
/// product; huge finite lengths must not reach it.
#[test]
fn huge_regions_cannot_overflow_bounding_box_transforms() {
    for (filter_attrs, primitive_attrs) in [
        (&[("width", "1e37")][..], &[][..]),
        (
            &[("primitiveUnits", "objectBoundingBox")][..],
            &[("width", "3e38")][..],
        ),
        // Em lengths resolve against the font size inside the parser.
        (&[("font-size", "1e37"), ("width", "1em")][..], &[][..]),
        (
            &[("primitiveUnits", "objectBoundingBox")][..],
            &[("font-size", "1e37"), ("width", "1e6em")][..],
        ),
    ] {
        let (mut document, canvas, filter, matrix) = self::document();
        let mut mutation = document.mutate();
        mutation.remove_node(matrix);
        for (name, value) in filter_attrs {
            mutation.set_attribute(
                filter,
                QualName {
                    prefix: None,
                    ns: crate::Namespace::from(""),
                    local: (*name).into(),
                },
                *value,
            );
        }
        let blur = svg(&mut mutation, "feGaussianBlur", primitive_attrs);
        mutation.append_children(filter, &[blur]);
        drop(mutation);
        document.flush_style_and_layout(0.0);
        let mut filters = document
            .canvas_filters(canvas, "url(#effect)", &CanvasFont::default())
            .unwrap();
        document
            .resolve_canvas_svg_filters(canvas, &mut filters)
            .unwrap();
        let snapshot = filters.svg[0].as_ref().unwrap();
        assert!(
            snapshot
                .resolve([0.0, 0.0, 300.0, 150.0], [300, 150])
                .is_some()
        );
    }
}

/// A primitive without area renders nothing; a filter without area is ignored.
#[test]
fn empty_sizes_empty_primitives_and_ignore_filters() {
    let (mut document, canvas, filter, matrix) = self::document();
    let mut mutation = document.mutate();
    mutation.remove_node(matrix);
    let flood = svg(&mut mutation, "feFlood", &[("width", "0")]);
    let offset = svg(&mut mutation, "feOffset", &[]);
    mutation.append_children(filter, &[flood, offset]);
    drop(mutation);
    document.flush_style_and_layout(0.0);
    let mut filters = document
        .canvas_filters(canvas, "url(#effect)", &CanvasFont::default())
        .unwrap();
    document
        .resolve_canvas_svg_filters(canvas, &mut filters)
        .unwrap();
    let resolved = filters.svg[0]
        .as_ref()
        .unwrap()
        .resolve([0.0, 0.0, 8.0, 8.0], [8, 8])
        .unwrap();
    assert_eq!(resolved.filter.primitives().len(), 2);
    assert!(resolved.empty_region(0) && !resolved.empty_region(1));
    document
        .mutate()
        .set_attribute(filter, qual_name!("height"), "-1");
    document
        .resolve_canvas_svg_filters(canvas, &mut filters)
        .unwrap();
    assert!(filters.svg[0].is_none());
}

#[test]
fn element_references_leave_out_use_and_non_xml_names() {
    use crate::canvas_svg_image::CanvasSvgImageSource as Source;
    let (mut document, canvas, filter, matrix) = self::document();
    let mut mutation = document.mutate();
    mutation.remove_node(matrix);
    let image = svg(&mut mutation, "feImage", &[("href", "#group")]);
    mutation.append_children(filter, &[image]);
    let parent = mutation.doc.nodes[filter].parent.unwrap();
    let group = svg(&mut mutation, "g", &[("id", "group")]);
    let rect = svg(
        &mut mutation,
        "rect",
        &[("width", "2"), ("height", "2"), ("a\"b", "1"), ("p:q", "1")],
    );
    let reuse = svg(&mut mutation, "use", &[("href", "#group")]);
    mutation.append_children(parent, &[group]);
    mutation.append_children(group, &[rect, reuse]);
    drop(mutation);
    let resolved = resolved(&mut document, canvas);
    let Source::Element(element) = &resolved.image(0).unwrap().source else {
        panic!("the element still parses");
    };
    let group = match element.tree.root().children() {
        [usvg::Node::Group(group)] => group,
        children => panic!("{children:?}"),
    };
    assert!(matches!(group.children(), [usvg::Node::Path(_)]));
}

/// `url(#id)` names the document itself whatever its base URL, as CSS local
/// URLs do; other URLs still resolve against the base.
#[test]
fn fragment_references_ignore_the_base_url() {
    let (mut document, canvas, _, _) = self::document();
    let mut mutation = document.mutate();
    let html = mutation.doc.root_element().id;
    let base = mutation.create_element(qual_name!("base", html), vec![]);
    mutation.set_attribute(base, qual_name!("href"), "https://cdn.test/assets/");
    mutation.append_children(html, &[base]);
    drop(mutation);
    document.flush_style_and_layout(0.0);
    let mut filters = document
        .canvas_filters(
            canvas,
            "url(#effect) url(other#effect)",
            &CanvasFont::default(),
        )
        .unwrap();
    let references: Vec<_> = filters
        .operations
        .iter()
        .map(|operation| match operation {
            crate::CanvasFilter::Reference(url) => url.as_str(),
            _ => panic!("a reference"),
        })
        .collect();
    assert_eq!(
        references,
        [
            "https://example.test/page#effect",
            "https://cdn.test/assets/other#effect"
        ]
    );
    document
        .resolve_canvas_svg_filters(canvas, &mut filters)
        .unwrap();
    assert!(filters.svg[0].is_some());
    assert!(filters.svg[1].is_none());
}
