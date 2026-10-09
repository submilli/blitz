//! Coordinates that the engine's SVG parser leaves in primitive units.

/// Primitive coordinates resolved for one filtered region.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanvasSvgUnits {
    /// The filtered bounds `[x, y, width, height]` for bounding-box units.
    bounding_box: Option<[f32; 4]>,
}

impl CanvasSvgUnits {
    /// Units for `[x, y, right, bottom]` bounds; `bounding_box` selects
    /// `primitiveUnits="objectBoundingBox"`.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "Resolved bounds are finite and at most 1e6 in magnitude."
    )]
    pub(crate) fn new(bounds: [f64; 4], bounding_box: bool) -> Self {
        let [x, y, right, bottom] = bounds.map(|v| v as f32);
        Self {
            bounding_box: bounding_box.then_some([x, y, right - x, bottom - y]),
        }
    }

    /// Light positions as user-space coordinates. Bounding-box units scale x and y
    /// by the box and z by its normalized diagonal, as Chrome resolves them.
    /// <https://drafts.fxtf.org/filter-effects/#element-attrdef-fepointlight-z>
    pub fn light(&self, light: usvg::filter::LightSource) -> usvg::filter::LightSource {
        use usvg::filter::LightSource;
        let Some([x, y, width, height]) = self.bounding_box else {
            return light;
        };
        let depth = ((width * width + height * height) / 2.0).sqrt();
        let point = |px: f32, py: f32, pz: f32| {
            (
                saturate(x + px * width),
                saturate(y + py * height),
                saturate(pz * depth),
            )
        };
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
                saturate(parsed / ((width + height) / 2.0) * width)
            }
            _ => parsed,
        }
    }
}

/// Scaled coordinates stay finite, so overflow renders like a huge value instead of
/// failing admission.
fn saturate(value: f32) -> f32 {
    value.clamp(-f32::MAX, f32::MAX)
}
