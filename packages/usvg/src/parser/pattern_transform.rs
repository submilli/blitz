// Copyright 2026 the Resvg Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Absolute coordinates after delayed pattern content normalization.

use crate::*;

/// Move this content coordinate system without changing nested paint servers'
/// own local coordinates. Resource trees (filters, clips, images) stay local.
pub(super) fn rebase(group: &mut Group, transform: Transform, depth: usize) -> Option<()> {
    if depth > 128 {
        return None;
    }
    group.abs_transform = transform.pre_concat(group.abs_transform);
    for node in &mut group.children {
        match node {
            Node::Group(child) => rebase(child, transform, depth + 1)?,
            Node::Path(path) => {
                let mut moved = Path::new(
                    path.id.clone(),
                    path.visible,
                    path.fill.clone(),
                    path.stroke.clone(),
                    path.paint_order,
                    path.rendering_mode,
                    path.data.clone(),
                    transform.pre_concat(path.abs_transform),
                )?;
                moved.source_rect = path.source_rect;
                moved.shape_kind = path.shape_kind;
                **path = moved;
            }
            Node::Image(image) => {
                image.abs_transform = transform.pre_concat(image.abs_transform);
                image.abs_bounding_box = image.abs_bounding_box.transform(transform)?;
            }
            Node::Text(text) => {
                text.abs_transform = transform.pre_concat(text.abs_transform);
                text.abs_bounding_box = text.abs_bounding_box.transform(transform)?;
                text.abs_stroke_bounding_box = text.abs_stroke_bounding_box.transform(transform)?;
                rebase(&mut text.flattened, transform, depth + 1)?;
            }
        }
    }
    group.calculate_bounding_boxes()?;
    Some(())
}

#[cfg(test)]
mod pattern_tests {
    use crate::{Node, Options, Paint, Transform, Tree};

    #[test]
    fn delayed_pattern_normalization_updates_leaf_and_group_absolute_coordinates() {
        let tree = Tree::from_str("<svg xmlns='http://www.w3.org/2000/svg' width='8' height='8'><pattern id='p' width='.5' height='.5' patternContentUnits='objectBoundingBox'><g transform='translate(.1 .1)'><rect width='.25' height='.25'/></g></pattern><rect width='8' height='8' fill='url(#p)'/></svg>",&Options::default()).unwrap();
        let pattern = &tree.patterns()[0];
        let mut nodes = vec![pattern.root()];
        let mut found = false;
        while let Some(group) = nodes.pop() {
            for node in group.children() {
                match node {
                    Node::Group(child) => nodes.push(child),
                    Node::Path(path) => {
                        assert_eq!(
                            path.abs_transform(),
                            Transform::from_row(8.0, 0.0, 0.0, 8.0, 0.8, 0.8)
                        );
                        assert!((path.abs_bounding_box().width() - 2.0).abs() < 1e-5);
                        assert!(matches!(path.fill().unwrap().paint(), Paint::Color(_)));
                        found = true;
                    }
                    _ => {}
                }
            }
        }
        assert!(found);
    }
}
