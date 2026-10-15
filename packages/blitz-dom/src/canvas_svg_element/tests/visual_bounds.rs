//! Blink stroke approximation, separate from renderer stroke outlines.

use super::*;

fn shape_bounds(
    tag: &str,
    geometry: &[(&str, &str)],
    cap: &str,
    join: &str,
    miter: &str,
) -> usvg::NonZeroRect {
    let (mut doc, svg) = page("");
    let mut m = doc.mutate();
    let mut attrs = geometry.to_vec();
    attrs.extend([
        ("stroke", "red"),
        ("stroke-width", "2"),
        ("stroke-linecap", cap),
        ("stroke-linejoin", join),
        ("stroke-miterlimit", miter),
    ]);
    let target = add(&mut m, svg, tag, &attrs);
    drop(m);
    capture(&mut doc, target).unwrap().bounds
}

#[test]
fn stroke_approximation_distinguishes_lines_paths_and_simple_shapes() {
    for cap in ["butt", "round", "square"] {
        for join in ["miter", "round", "bevel"] {
            for miter in ["1", "1.2", "4", "10"] {
                let square = cap == "square";
                let factor = if join == "miter" {
                    miter.parse::<f32>().unwrap().max(if square {
                        std::f32::consts::SQRT_2
                    } else {
                        1.0
                    })
                } else if square {
                    std::f32::consts::SQRT_2
                } else {
                    1.0
                };
                let path = shape_bounds("path", &[("d", "M2 3L6 3L6 7")], cap, join, miter);
                assert!((path.x() - (2.0 - factor)).abs() < 1e-5);
                assert!((path.width() - (4.0 + 2.0 * factor)).abs() < 1e-5);
                let line = shape_bounds(
                    "line",
                    &[("x1", "2"), ("y1", "3"), ("x2", "6"), ("y2", "3")],
                    cap,
                    join,
                    miter,
                );
                let delta = if square {
                    std::f32::consts::SQRT_2
                } else {
                    1.0
                };
                assert!((line.x() - (2.0 - delta)).abs() < 1e-5);
                assert!((line.height() - 2.0 * delta).abs() < 1e-5);
                for (tag, attrs) in [
                    (
                        "rect",
                        vec![("x", "2"), ("y", "3"), ("width", "4"), ("height", "4")],
                    ),
                    (
                        "rect",
                        vec![
                            ("x", "2"),
                            ("y", "3"),
                            ("width", "4"),
                            ("height", "4"),
                            ("rx", "1"),
                        ],
                    ),
                    (
                        "ellipse",
                        vec![("cx", "4"), ("cy", "5"), ("rx", "2"), ("ry", "2")],
                    ),
                    ("circle", vec![("cx", "4"), ("cy", "5"), ("r", "2")]),
                ] {
                    let bounds = shape_bounds(tag, &attrs, cap, join, miter);
                    assert_eq!(
                        (bounds.x(), bounds.y(), bounds.width(), bounds.height()),
                        (1.0, 2.0, 6.0, 6.0),
                        "{tag}/{cap}/{join}/{miter}"
                    );
                }
            }
        }
    }
}

#[test]
fn transformed_subtrees_and_markers_union_the_approximate_boxes() {
    let (mut doc, svg) = page("");
    let mut m = doc.mutate();
    let marker = add(
        &mut m,
        svg,
        "marker",
        &[
            ("id", "m"),
            ("markerUnits", "userSpaceOnUse"),
            ("markerWidth", "20"),
            ("markerHeight", "20"),
            ("overflow", "visible"),
        ],
    );
    add(
        &mut m,
        marker,
        "rect",
        &[("x", "5"), ("y", "5"), ("width", "2"), ("height", "2")],
    );
    let group = add(&mut m, svg, "g", &[("transform", "translate(10 20)")]);
    add(
        &mut m,
        group,
        "line",
        &[
            ("x2", "4"),
            ("stroke", "red"),
            ("stroke-width", "2"),
            ("marker-end", "url(#m)"),
        ],
    );
    add(
        &mut m,
        group,
        "rect",
        &[
            ("transform", "scale(2 3)"),
            ("width", "2"),
            ("height", "2"),
            ("stroke", "red"),
            ("stroke-width", "2"),
        ],
    );
    drop(m);
    let element = capture(&mut doc, group).unwrap();
    assert_eq!(
        (
            element.bounds.x(),
            element.bounds.y(),
            element.bounds.width(),
            element.bounds.height()
        ),
        (8.0, 17.0, 13.0, 12.0)
    );
    let paths = paths(&element);
    assert_eq!(paths.len(), 3);
    // Source transforms are absent from paint; native marker placement stays.
    assert_eq!(
        paths
            .iter()
            .filter(|p| p.abs_transform() == usvg::Transform::identity())
            .count(),
        2
    );
    assert!(
        paths
            .iter()
            .any(|p| p.abs_transform() == usvg::Transform::from_translate(4.0, 0.0))
    );
}

#[test]
fn a_shape_without_stroke_keeps_its_fill_bounds() {
    let (mut doc, svg) = page("");
    let mut m = doc.mutate();
    let rect = add(
        &mut m,
        svg,
        "rect",
        &[
            ("width", "2"),
            ("height", "3"),
            ("stroke", "none"),
            ("stroke-width", "100"),
        ],
    );
    drop(m);
    let bounds = capture(&mut doc, rect).unwrap().bounds;
    assert_eq!(
        (bounds.x(), bounds.y(), bounds.width(), bounds.height()),
        (0.0, 0.0, 2.0, 3.0)
    );
}

#[test]
fn a_single_segment_path_uses_line_bounds_without_a_miter_multiplier() {
    for tag in ["path", "polyline"] {
        let geometry = if tag == "path" {
            [("d", "M1 1L2 1")]
        } else {
            [("points", "1,1 2,1")]
        };
        let bounds = shape_bounds(tag, &geometry, "butt", "miter", "10");
        assert_eq!(
            (bounds.x(), bounds.y(), bounds.width(), bounds.height()),
            (0.0, 0.0, 3.0, 2.0)
        );
    }
}

#[test]
fn closed_and_multi_segment_paths_retain_the_miter_multiplier() {
    for data in ["M1 1L2 1Z", "M1 1L2 1L3 1"] {
        let bounds = shape_bounds("path", &[("d", data)], "butt", "miter", "10");
        assert_eq!(bounds.x(), -9.0);
        assert_eq!(bounds.y(), -9.0);
        assert_eq!(bounds.height(), 20.0);
    }
}
