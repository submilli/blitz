//! Observable pixels of the shared SVG image renderer.
use super::*;
use anyrender_vello_cpu::VelloCpuScenePainter;

fn draw(body: &str) -> vello_cpu::Pixmap {
    let svg = format!("<svg xmlns='http://www.w3.org/2000/svg' width='8' height='8'>{body}</svg>");
    let tree = usvg::Tree::from_str(&svg, &usvg::Options::default()).unwrap();
    let mut scene = VelloCpuScenePainter::new(8, 8);
    paint_svg_tree(
        &mut scene,
        &tree,
        &SvgNestedRasters::default(),
        Affine::IDENTITY,
    );
    scene.finish()
}

#[test]
fn flood_filter_replaces_source_paint() {
    let pixels = draw(
        "<filter id='f' x='0' y='0' width='1' height='1'><feFlood flood-color='lime'/></filter><rect width='8' height='8' filter='url(#f)'/>",
    );
    assert!(
        pixels
            .data()
            .iter()
            .all(|p| (p.r, p.g, p.b, p.a) == (0, 255, 0, 255))
    );
}

#[test]
fn filter_source_alpha_and_offset_use_original_source() {
    let pixels = draw(
        "<filter id='f' x='0' y='0' width='8' height='8' filterUnits='userSpaceOnUse'><feOffset in='SourceAlpha' dx='2'/></filter><rect width='2' height='8' fill='red' filter='url(#f)'/>",
    );
    assert_eq!(pixels.data()[0].a, 0);
    assert_eq!(pixels.data()[2].a, 255);
    assert_eq!(pixels.data()[2].r, 0);
}

#[test]
fn offset_reads_source_outside_the_filter_crop() {
    let pixels = draw(
        "<filter id='f' x='4' y='0' width='4' height='8' filterUnits='userSpaceOnUse'><feOffset dx='4'/></filter><rect width='2' height='8' fill='red' filter='url(#f)'/>",
    );
    assert_eq!(pixels.data()[0].a, 0);
    assert_eq!(pixels.data()[4].r, 255);
    assert_eq!(pixels.data()[6].a, 0);
}

#[test]
fn empty_merge_is_transparent_and_later_siblings_paint() {
    let pixels = draw(
        "<filter id='f'><feMerge/></filter><rect width='8' height='8' fill='red' filter='url(#f)'/><rect x='4' width='4' height='8' fill='lime'/>",
    );
    assert_eq!(pixels.data()[0].a, 0);
    assert_eq!(pixels.data()[4].g, 255);
}

#[test]
fn refused_filtered_group_does_not_hide_later_siblings() {
    let pixels = draw(
        "<filter id='f' x='0' y='0' width='20000' height='20000' filterUnits='userSpaceOnUse'><feFlood/></filter><rect width='8' height='8' filter='url(#f)'/><rect x='4' width='4' height='8' fill='lime'/>",
    );
    assert_eq!(pixels.data()[0].a, 0);
    assert_eq!(pixels.data()[4].g, 255);
}

#[test]
fn admitted_graph_work_is_cumulative() {
    use vello_common::filter::frames::RasterCost;
    let mut budget = Budget::default();
    budget
        .charge_graph(RasterCost {
            pixels: 1,
            work: MAX_WORK / 2,
        })
        .unwrap();
    budget
        .charge_graph(RasterCost {
            pixels: 1,
            work: MAX_WORK / 2,
        })
        .unwrap();
    assert_eq!(
        budget.charge_graph(RasterCost { pixels: 1, work: 1 }),
        Err(GraphError::Work)
    );
}

#[test]
fn scaled_filters_keep_vector_edges_at_device_resolution() {
    let svg = "<svg xmlns='http://www.w3.org/2000/svg' width='1' height='1'><filter id='f' x='0' y='0' width='1' height='1'><feColorMatrix/></filter><rect width='0.5' height='1' fill='lime' filter='url(#f)'/></svg>";
    let tree = usvg::Tree::from_str(svg, &usvg::Options::default()).unwrap();
    let mut scene = VelloCpuScenePainter::new(8, 8);
    paint_svg_tree(
        &mut scene,
        &tree,
        &SvgNestedRasters::default(),
        Affine::scale(8.0),
    );
    let pixels = scene.finish();
    assert_eq!(pixels.data()[3].g, 255);
    assert_eq!(pixels.data()[4].a, 0);
}

#[test]
fn positioned_image_uses_its_whole_subregion_before_clipping() {
    let source = "data:image/svg+xml,%3Csvg%20xmlns='http://www.w3.org/2000/svg'%20width='4'%20height='4'%3E%3Crect%20width='4'%20height='4'%20fill='lime'/%3E%3C/svg%3E";
    let pixels = draw(&format!(
        "<filter id='f' x='5' y='0' width='3' height='8' filterUnits='userSpaceOnUse'><feImage x='4' y='2' width='4' height='4' href=\"{source}\"/></filter><rect width='8' height='8' filter='url(#f)'/>"
    ));
    assert_eq!(pixels.data()[2 * 8 + 4].a, 0);
    assert_eq!(pixels.data()[2 * 8 + 5].g, 255);
    assert_eq!(pixels.data()[6 * 8 + 5].a, 0);
}

#[test]
fn parent_source_clips_child_filter_overflow_like_chrome() {
    let pixels = draw(
        "<filter id='outer' filterUnits='userSpaceOnUse' x='4' y='0' width='4' height='8'><feOffset dx='4'/></filter><filter id='inner' filterUnits='userSpaceOnUse' x='0' y='0' width='2' height='8'><feFlood flood-color='red'/></filter><g filter='url(#outer)'><rect x='4' width='2' height='8' filter='url(#inner)'/></g>",
    );
    assert!(pixels.data().iter().all(|p| p.a == 0));
}

#[test]
fn temporary_painters_charge_each_whole_raster_conversion() {
    use std::sync::Arc;
    let raster = blitz_dom::node::RasterImageData::new(4096, 2048, Arc::new(Vec::new()));
    let mut budget = Budget::default();
    let mut first = HashSet::new();
    budget.raster(&raster, &mut first).unwrap();
    budget.raster(&raster, &mut first).unwrap();
    assert_eq!(budget.pixels, MAX_INTERMEDIATE_PIXELS / 2);
    let mut second = HashSet::new();
    budget.raster(&raster, &mut second).unwrap();
    let mut refused = HashSet::new();
    assert_eq!(
        budget.raster(&raster, &mut refused),
        Err(GraphError::Memory)
    );
    assert!(
        refused.is_empty(),
        "refusal must not mark an uncharged conversion cached"
    );
    assert_eq!(
        budget.raster(&raster, &mut refused),
        Err(GraphError::Memory)
    );
}

#[test]
fn image_sampling_work_is_charged_before_execution() {
    let source = "data:image/svg+xml,%3Csvg%20xmlns='http://www.w3.org/2000/svg'%20width='4'%20height='4'%3E%3Crect%20width='4'%20height='4'%20fill='lime'/%3E%3C/svg%3E";
    let svg = format!(
        "<svg xmlns='http://www.w3.org/2000/svg' width='4' height='4'><filter id='f' x='0' y='0' width='1' height='1'><feImage href=\"{source}\"/></filter><rect width='4' height='4' filter='url(#f)'/></svg>"
    );
    let tree = usvg::Tree::from_str(&svg, &usvg::Options::default()).unwrap();
    let mut groups = vec![tree.root()];
    let filtered = loop {
        let group = groups.pop().unwrap();
        if !group.filters().is_empty() {
            break group;
        }
        groups.extend(group.children().iter().filter_map(|node| match node {
            usvg::Node::Group(group) => Some(group.as_ref()),
            _ => None,
        }));
    };
    let mut scene = VelloCpuScenePainter::new(4, 4);
    let mut budget = Budget {
        work: 128,
        ..Budget::default()
    };
    assert_eq!(
        filters::paint(
            &mut scene,
            filtered,
            &SvgNestedRasters::default(),
            Affine::IDENTITY,
            &mut budget,
            0
        ),
        Err(GraphError::Work)
    );
    assert!(scene.finish().data().iter().all(|p| p.a == 0));
}

#[test]
fn translated_view_box_does_not_expand_source_bounds_to_the_origin() {
    let svg = "<svg xmlns='http://www.w3.org/2000/svg' width='8' height='8' viewBox='100000 0 8 8'><filter id='f'><feColorMatrix/></filter><rect x='100000' width='8' height='8' fill='lime' filter='url(#f)'/></svg>";
    let tree = usvg::Tree::from_str(svg, &usvg::Options::default()).unwrap();
    let mut scene = VelloCpuScenePainter::new(8, 8);
    paint_svg_tree(
        &mut scene,
        &tree,
        &SvgNestedRasters::default(),
        Affine::IDENTITY,
    );
    assert!(
        scene
            .finish()
            .data()
            .iter()
            .all(|p| p.g == 255 && p.a == 255)
    );
}

#[test]
fn empty_filtered_groups_draw_nothing_like_chrome() {
    let svg = "<svg xmlns='http://www.w3.org/2000/svg' width='8' height='8' viewBox='100000 0 8 8'><filter id='f' filterUnits='userSpaceOnUse' x='100000' y='0' width='8' height='8'><feFlood flood-color='lime'/></filter><g filter='url(#f)'/></svg>";
    let tree = usvg::Tree::from_str(svg, &usvg::Options::default()).unwrap();
    let mut scene = VelloCpuScenePainter::new(8, 8);
    paint_svg_tree(
        &mut scene,
        &tree,
        &SvgNestedRasters::default(),
        Affine::IDENTITY,
    );
    assert!(scene.finish().data().iter().all(|p| p.a == 0));
}

#[test]
fn filter_owned_vectors_are_bounded_before_flattening() {
    let pixels = draw(
        "<defs><path id='huge' d='M0 0 C 1e20 1e20 -1e20 1e20 1 0 Z'/></defs><filter id='f'><feImage href='#huge'/></filter><rect width='8' height='8' filter='url(#f)'/>",
    );
    assert!(pixels.data().iter().all(|p| p.a == 0));
}

#[test]
fn intermediate_filter_clips_are_bounded_at_their_raster_transform() {
    let pixels = draw(
        "<defs><clipPath id='clip'><ellipse rx='1' ry='1e8'/></clipPath><filter id='f'><feColorMatrix/></filter></defs><g transform='scale(1,1e-4)'><g filter='url(#f)'><g clip-path='url(#clip)'><rect width='8' height='8' fill='lime'/></g></g></g>",
    );
    assert!(pixels.data().iter().all(|p| p.a == 0));
}

#[test]
fn source_rect_preserves_nonuniform_stroke_coverage() {
    fn render(shape: &str, filter: bool) -> vello_cpu::Pixmap {
        let group = if filter { " filter='url(#f)'" } else { "" };
        let svg = format!(
            "<svg xmlns='http://www.w3.org/2000/svg' width='16' height='16'>\
             <filter id='f' x='0' y='0' width='16' height='16' filterUnits='userSpaceOnUse'><feOffset/></filter>\
             <g{group}><g transform='scale({} {})' fill='none' stroke='red' stroke-width='23.717'>{shape}</g></g></svg>",
            8.0 / 300.0,
            8.0 / 150.0,
        );
        let tree = usvg::Tree::from_str(&svg, &usvg::Options::default()).unwrap();
        let mut scene = VelloCpuScenePainter::new(16, 16);
        paint_svg_tree(
            &mut scene,
            &tree,
            &SvgNestedRasters::default(),
            Affine::IDENTITY,
        );
        scene.finish()
    }
    for filter in [false, true] {
        let rect = render("<rect x='60' y='30' width='150' height='75'/>", filter);
        let path = render("<path d='M60 30H210V105H60Z'/>", filter);
        assert!(rect.data()[17].a.abs_diff(255) <= 1, "filter={filter}");
        assert!(path.data()[17].a.abs_diff(184) <= 1, "filter={filter}");
    }
}

#[test]
fn pattern_units_viewbox_and_nested_paints_repeat_their_tiles() {
    for (attrs, content) in [
        (
            "patternUnits='userSpaceOnUse' width='4' height='4'",
            "<rect width='2' height='2' fill='lime'/>",
        ),
        (
            "width='.5' height='.5' patternContentUnits='objectBoundingBox'",
            "<rect width='.25' height='.25' fill='lime'/>",
        ),
        (
            "width='.5' height='.5' viewBox='0 0 8 8'",
            "<rect width='4' height='4' fill='lime'/>",
        ),
        (
            "patternUnits='userSpaceOnUse' width='4' height='4'",
            "<rect width='2' height='2' fill='url(#q)'/>",
        ),
    ] {
        let pixels = draw(&format!(
            "<pattern id='q' patternUnits='userSpaceOnUse' width='1' height='1'><rect width='1' height='1' fill='lime'/></pattern><pattern id='p' {attrs}>{content}</pattern><rect width='8' height='8' fill='url(#p)'/>"
        ));
        for y in 0..8 {
            for x in 0..8 {
                assert_eq!(
                    pixels.data()[y * 8 + x].g,
                    if x % 4 < 2 && y % 4 < 2 { 255 } else { 0 },
                    "{attrs}: {x},{y}"
                );
            }
        }
    }
}

#[test]
fn pattern_origin_moves_the_tile_without_moving_its_content_coordinates() {
    let pixels = draw(
        "<pattern id='p' patternUnits='userSpaceOnUse' x='1' y='2' width='4' height='4'><rect x='1' y='2' width='2' height='2' fill='lime'/></pattern><rect width='8' height='8' fill='url(#p)'/>",
    );
    for y in 0..8 {
        for x in 0..8 {
            assert_eq!(
                pixels.data()[y * 8 + x].g,
                if x % 4 >= 2 && y % 4 < 2 { 255 } else { 0 }
            );
        }
    }
}

#[test]
fn pattern_refusal_is_bounded_and_preserves_later_siblings() {
    let pixels = draw(
        "<pattern id='p' patternUnits='userSpaceOnUse' width='20000' height='20000'><rect width='20000' height='20000' fill='red'/></pattern><rect width='8' height='8' fill='url(#p)'/><rect x='4' width='4' height='8' fill='lime'/>",
    );
    assert_eq!(pixels.data()[0].a, 0);
    assert_eq!(pixels.data()[4].g, 255);
}

#[test]
fn pattern_tiles_share_cumulative_work_and_depth_admission() {
    let tree = usvg::Tree::from_str("<svg xmlns='http://www.w3.org/2000/svg' width='8' height='8'><pattern id='p' patternUnits='userSpaceOnUse' width='4' height='4'><rect width='2' height='2' fill='lime'/></pattern><rect width='8' height='8' fill='url(#p)'/></svg>",&usvg::Options::default()).unwrap();
    let pattern = &tree.patterns()[0];
    let rasters = SvgNestedRasters::default();
    let mut budget = Budget {
        work: 512,
        ..Default::default()
    };
    pattern::brush(
        pattern,
        usvg::Opacity::ONE,
        Affine::IDENTITY,
        &rasters,
        &mut budget,
        0,
    )
    .unwrap();
    assert_eq!(
        pattern::brush(
            pattern,
            usvg::Opacity::ONE,
            Affine::IDENTITY,
            &rasters,
            &mut budget,
            0
        )
        .unwrap_err(),
        GraphError::Work
    );
    assert_eq!(
        pattern::brush(
            pattern,
            usvg::Opacity::ONE,
            Affine::IDENTITY,
            &rasters,
            &mut Budget::default(),
            129
        )
        .unwrap_err(),
        GraphError::Size
    );
}

#[test]
fn refused_pattern_paint_keeps_the_other_paint_on_the_same_path() {
    let pattern = "<pattern id='p' patternUnits='userSpaceOnUse' width='20000' height='20000'><rect width='1' height='1'/></pattern>";
    for order in ["fill stroke", "stroke fill"] {
        let fill = draw(&format!(
            "{pattern}<rect x='1' y='1' width='6' height='6' fill='lime' stroke='url(#p)' stroke-width='2' paint-order='{order}'/>"
        ));
        assert_eq!(fill.data()[3 * 8 + 3].g, 255);
        let stroke = draw(&format!(
            "{pattern}<rect x='1' y='1' width='6' height='6' fill='url(#p)' stroke='lime' stroke-width='2' paint-order='{order}'/>"
        ));
        assert_eq!(stroke.data()[9].g, 255);
    }
}

#[test]
fn pattern_layer_work_is_admitted_before_emission() {
    let svg = "<svg xmlns='http://www.w3.org/2000/svg' width='8' height='8'><pattern id='p' patternUnits='userSpaceOnUse' width='4' height='4'><g opacity='.5'><g opacity='.5'><rect width='4' height='4' fill='lime'/></g></g></pattern><rect width='8' height='8' fill='url(#p)'/></svg>";
    let tree = usvg::Tree::from_str(svg, &usvg::Options::default()).unwrap();
    let mut budget = Budget {
        work: 512,
        ..Default::default()
    };
    let result = pattern::brush(
        &tree.patterns()[0],
        usvg::Opacity::ONE,
        Affine::IDENTITY,
        &SvgNestedRasters::default(),
        &mut budget,
        0,
    );
    assert_eq!(result.unwrap_err(), GraphError::Work);
    assert_eq!(budget.surface_area, 0, "target restored even on refusal");
}

#[test]
fn overlapping_pattern_fills_share_the_surface_work_budget() {
    let svg = "<svg xmlns='http://www.w3.org/2000/svg' width='8' height='8'><pattern id='p' patternUnits='userSpaceOnUse' width='4' height='4'><rect width='4' height='4'/><rect width='4' height='4'/></pattern><rect width='8' height='8' fill='url(#p)'/></svg>";
    let tree = usvg::Tree::from_str(svg, &usvg::Options::default()).unwrap();
    let mut budget = Budget {
        work: 512,
        ..Default::default()
    };
    let result = pattern::brush(
        &tree.patterns()[0],
        usvg::Opacity::ONE,
        Affine::IDENTITY,
        &SvgNestedRasters::default(),
        &mut budget,
        0,
    );
    assert_eq!(result.unwrap_err(), GraphError::Work);
    assert_eq!(budget.surface_area, 0);
}

#[test]
fn refused_nested_pattern_does_not_commit_an_outer_tile() {
    let svg = "<svg xmlns='http://www.w3.org/2000/svg' width='8' height='8'><pattern id='q' patternUnits='userSpaceOnUse' width='20000' height='20000'><rect width='1' height='1'/></pattern><pattern id='p' patternUnits='userSpaceOnUse' width='4' height='4'><rect width='4' height='4' fill='url(#q)'/></pattern><rect width='8' height='8' fill='url(#p)'/></svg>";
    let tree = usvg::Tree::from_str(svg, &usvg::Options::default()).unwrap();
    let pattern = tree
        .patterns()
        .iter()
        .find(|pattern| pattern.id() == "p")
        .unwrap();
    assert_eq!(
        pattern::brush(
            pattern,
            usvg::Opacity::ONE,
            Affine::IDENTITY,
            &SvgNestedRasters::default(),
            &mut Budget::default(),
            0
        )
        .unwrap_err(),
        GraphError::Memory
    );
}
