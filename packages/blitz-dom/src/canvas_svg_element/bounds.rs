//! The placed subtree's visual rectangle for an `feImage` picture cull.

use usvg::{LineCap, LineJoin, Node, Path, Rect, ShapeKind, Tree};

/// Union placed leaves, including the parser's expanded `<use>` and marker
/// content. Paint-server content is not a descendant and adds no bounds.
pub(super) fn measure(tree: &Tree) -> Option<Rect> {
    let mut pending = vec![tree.root()];
    let mut bounds: Option<Rect> = None;
    while let Some(group) = pending.pop() {
        for node in group.children() {
            let rect = match node {
                Node::Group(child) => {
                    pending.push(child);
                    continue;
                }
                Node::Path(path) => approximate(path)?,
                Node::Image(_) | Node::Text(_) => node.abs_stroke_bounding_box(),
            };
            bounds = Some(match bounds {
                Some(bounds) => bounds.join(&rect)?,
                None => rect,
            });
        }
    }
    bounds
}

/// Blink's LayoutSVGShape::ApproximateStrokeBoundingBox outsets the fill box
/// before transforming it. Exact stroked outlines are deliberately not used:
/// a butt-capped line's cull extends past its endpoints too.
fn approximate(path: &Path) -> Option<Rect> {
    let rect = path.bounding_box();
    let delta = path.stroke().map_or(0.0, |stroke| {
        let square = stroke.linecap() == LineCap::Square;
        let factor = match path.shape_kind() {
            ShapeKind::Simple => 1.0,
            ShapeKind::Path if stroke.linejoin() == LineJoin::Miter => {
                stroke.miterlimit().get().max(if square {
                    std::f32::consts::SQRT_2
                } else {
                    1.0
                })
            }
            ShapeKind::Path | ShapeKind::Line if square => std::f32::consts::SQRT_2,
            ShapeKind::Path | ShapeKind::Line => 1.0,
        };
        stroke.width().get() * 0.5 * factor
    });
    Rect::from_ltrb(
        rect.left() - delta,
        rect.top() - delta,
        rect.right() + delta,
        rect.bottom() + delta,
    )?
    .transform(path.abs_transform())
}
