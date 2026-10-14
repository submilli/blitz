//! Raster resolution of an SVG filter, independent of its final affine placement.
//! A uniform local grid preserves the primitive's axes through rotations/shears;
//! the output affine maps that grid to device space after filtering.

use kurbo::{Affine, Rect};
use vello_common::filter::graph::GraphError;
use vello_common::filter_effects::{FilterPrimitive, LightSource};

#[derive(Clone, Copy)]
pub(super) struct Resolution(pub f64);

impl Resolution {
    pub(super) fn new(device: Affine) -> Result<Self, GraphError> {
        let [a, b, c, d, _, _] = device.as_coeffs();
        let scale = a.hypot(b).max(c.hypot(d));
        if !(1e-6..=1e6).contains(&scale) {
            return Err(GraphError::Parameter);
        }
        Ok(Self(scale))
    }

    pub(super) fn rect(self, rect: Rect) -> Rect {
        Affine::scale(self.0).transform_rect_bbox(rect)
    }

    /// Scale geometry, leaving color arithmetic and topology unchanged. The
    /// input is produced only by the shared parsed-SVG primitive converter.
    pub(super) fn operation(self, mut primitive: FilterPrimitive) -> FilterPrimitive {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "Resolution is finite and within 1e-6..=1e6; graph admission rejects overflowing parameters."
        )]
        let scale = self.0 as f32;
        match &mut primitive {
            FilterPrimitive::AxisGaussianBlur {
                std_deviation_x,
                std_deviation_y,
                ..
            } => {
                *std_deviation_x *= scale;
                *std_deviation_y *= scale;
            }
            FilterPrimitive::AxisDropShadow {
                dx,
                dy,
                std_deviation_x,
                std_deviation_y,
                ..
            } => {
                *dx *= scale;
                *dy *= scale;
                *std_deviation_x *= scale;
                *std_deviation_y *= scale;
            }
            FilterPrimitive::Offset { dx, dy } => {
                *dx *= scale;
                *dy *= scale;
            }
            FilterPrimitive::Morphology {
                radius_x, radius_y, ..
            } => {
                *radius_x *= scale;
                *radius_y *= scale;
            }
            FilterPrimitive::DisplacementMap { scale: amount, .. } => *amount *= scale,
            FilterPrimitive::Turbulence {
                base_frequency_x,
                base_frequency_y,
                ..
            } => {
                *base_frequency_x /= scale;
                *base_frequency_y /= scale;
            }
            FilterPrimitive::DiffuseLighting {
                surface_scale,
                light_source,
                ..
            }
            | FilterPrimitive::SpecularLighting {
                surface_scale,
                light_source,
                ..
            } => {
                *surface_scale *= scale;
                match light_source {
                    LightSource::Point { x, y, z } => {
                        *x *= scale;
                        *y *= scale;
                        *z *= scale;
                    }
                    LightSource::Spot {
                        x,
                        y,
                        z,
                        points_at_x,
                        points_at_y,
                        points_at_z,
                        ..
                    } => {
                        *x *= scale;
                        *y *= scale;
                        *z *= scale;
                        *points_at_x *= scale;
                        *points_at_y *= scale;
                        *points_at_z *= scale;
                    }
                    LightSource::Distant { .. } => {}
                }
            }
            FilterPrimitive::GaussianBlur { std_deviation, .. } => *std_deviation *= scale,
            FilterPrimitive::DropShadow {
                dx,
                dy,
                std_deviation,
                ..
            }
            | FilterPrimitive::DropShadowOnly {
                dx,
                dy,
                std_deviation,
                ..
            } => {
                *dx *= scale;
                *dy *= scale;
                *std_deviation *= scale;
            }
            FilterPrimitive::Flood { .. }
            | FilterPrimitive::ColorMatrix { .. }
            | FilterPrimitive::Composite { .. }
            | FilterPrimitive::Blend { .. }
            | FilterPrimitive::ConvolveMatrix { .. }
            | FilterPrimitive::ComponentTransfer { .. }
            | FilterPrimitive::Image { .. }
            | FilterPrimitive::Tile => {}
        }
        primitive
    }
}
