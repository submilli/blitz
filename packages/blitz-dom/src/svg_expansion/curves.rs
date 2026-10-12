//! How many curves usvg makes of a document's shapes: kurbo's
//! `Arc::to_cubic_beziers` splits an arc into a number of cubics that grows
//! with its radius, so `<circle r="3e38">` alone becomes millions of curves.

/// How many cubic curves the parser makes of an arc of `radius`: kurbo's
/// count for a full turn at an accuracy of 0.1, at least four.
pub(crate) fn arc_curves(radius: f64) -> usize {
    let count = (1.1163 * radius.abs() / 0.1)
        .powf(1.0 / 6.0)
        .ceil()
        .max(4.0);
    // Past this many, a document is refused anyway; saturating keeps the
    // sum of counts an upper bound.
    if count.is_finite() && count < 1e6 {
        count as usize
    } else {
        usize::MAX
    }
}

/// How many curves `d`'s arcs and other segments become: kurbo scales an
/// arc's radii up to half its chord, so the endpoints count too.
pub(crate) fn path_curves(d: &str) -> usize {
    use svgtypes::PathSegment;
    let (mut x, mut y, mut start) = (0.0, 0.0, (0.0, 0.0));
    let mut curves = 0usize;
    for segment in svgtypes::PathParser::from(d).map_while(Result::ok) {
        let (to, count) = match segment {
            PathSegment::EllipticalArc {
                abs,
                rx,
                ry,
                x: ex,
                y: ey,
                ..
            } => {
                let to = if abs { (ex, ey) } else { (x + ex, y + ey) };
                let chord = ((to.0 - x).powi(2) + (to.1 - y).powi(2)).sqrt() / 2.0;
                (Some(to), arc_curves(rx.abs().max(ry.abs()).max(chord)))
            }
            PathSegment::ClosePath { .. } => (Some(start), 1),
            other => (end_point(other, x, y), 1),
        };
        if let PathSegment::MoveTo { .. } = segment {
            start = to.unwrap_or((x, y));
        }
        if let Some((nx, ny)) = to {
            (x, y) = (nx, ny);
        }
        curves = curves.saturating_add(count);
    }
    curves
}

/// Where a segment other than an arc or a close ends.
fn end_point(segment: svgtypes::PathSegment, x: f64, y: f64) -> Option<(f64, f64)> {
    use svgtypes::PathSegment::*;
    let point = |abs: bool, px: f64, py: f64| if abs { (px, py) } else { (x + px, y + py) };
    Some(match segment {
        MoveTo { abs, x: px, y: py }
        | LineTo { abs, x: px, y: py }
        | CurveTo {
            abs, x: px, y: py, ..
        }
        | SmoothCurveTo {
            abs, x: px, y: py, ..
        }
        | Quadratic {
            abs, x: px, y: py, ..
        }
        | SmoothQuadratic { abs, x: px, y: py } => point(abs, px, py),
        HorizontalLineTo { abs, x: px } => (if abs { px } else { x + px }, y),
        VerticalLineTo { abs, y: py } => (x, if abs { py } else { y + py }),
        EllipticalArc { .. } | ClosePath { .. } => return None,
    })
}
