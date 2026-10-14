//! Renderer operation conversion from the SVG engine's parsed primitive types.
use vello_common::filter::graph::GraphError;
fn limit_error() -> GraphError {
    GraphError::Unsupported
}
use blitz_dom::canvas_svg_types as svg;
use vello_common::color::{AlphaColor, Srgb};
use vello_common::filter_effects::{
    ColorChannel, CompositeOperator, ConvolutionKernel, EdgeMode, Filter, FilterFunction,
    FilterPrimitive, LightSource, MorphologyOperator, TransferFunction, TurbulenceType,
};

/// One SVG primitive and its source connections, before graph assembly.
pub struct Operation<'a> {
    /// Renderer operation.
    pub primitive: FilterPrimitive,
    /// Primary SVG source, absent for generators.
    pub input: Option<&'a svg::Input>,
    /// Secondary SVG source for binary operations.
    pub input2: Option<&'a svg::Input>,
}

/// The inputs a primitive reads, in document order. Keep in step with
/// [`operation`], which connects the same inputs.
/// Inputs consumed by the primitive, in lowering order.
pub fn inputs(kind: &svg::Kind) -> Vec<&svg::Input> {
    match kind {
        svg::Kind::Blend(value) => vec![value.input1(), value.input2()],
        svg::Kind::Composite(value) => vec![value.input1(), value.input2()],
        svg::Kind::DisplacementMap(value) => vec![value.input1(), value.input2()],
        svg::Kind::Merge(value) => value.inputs().iter().collect(),
        svg::Kind::ColorMatrix(value) => vec![value.input()],
        svg::Kind::ComponentTransfer(value) => vec![value.input()],
        svg::Kind::ConvolveMatrix(value) => vec![value.input()],
        svg::Kind::DiffuseLighting(value) => vec![value.input()],
        svg::Kind::DropShadow(value) => vec![value.input()],
        svg::Kind::GaussianBlur(value) => vec![value.input()],
        svg::Kind::Morphology(value) => vec![value.input()],
        svg::Kind::Offset(value) => vec![value.input()],
        svg::Kind::SpecularLighting(value) => vec![value.input()],
        svg::Kind::Tile(value) => vec![value.input()],
        svg::Kind::Flood(_) | svg::Kind::Image(_) | svg::Kind::Turbulence(_) => Vec::new(),
    }
}

/// Convert a parsed SVG operation, applying unresolved primitive units if supplied.
pub fn operation(
    kind: &svg::Kind,
    units: Option<blitz_dom::CanvasSvgUnits>,
) -> Result<Operation<'_>, GraphError> {
    let (primitive, input, input2) = match kind {
        svg::Kind::ColorMatrix(value) => (
            FilterPrimitive::ColorMatrix {
                matrix: matrix(value.kind())?,
            },
            Some(value.input()),
            None,
        ),
        svg::Kind::GaussianBlur(value) => (
            FilterPrimitive::AxisGaussianBlur {
                std_deviation_x: value.std_dev_x().get(),
                std_deviation_y: value.std_dev_y().get(),
                edge_mode: EdgeMode::None,
            },
            Some(value.input()),
            None,
        ),
        svg::Kind::Offset(value) => (
            FilterPrimitive::Offset {
                dx: value.dx(),
                dy: value.dy(),
            },
            Some(value.input()),
            None,
        ),
        svg::Kind::Flood(value) => (
            FilterPrimitive::Flood {
                color: color(value.color(), value.opacity().get()),
            },
            None,
            None,
        ),
        svg::Kind::DropShadow(value) => (
            FilterPrimitive::AxisDropShadow {
                dx: value.dx(),
                dy: value.dy(),
                std_deviation_x: value.std_dev_x().get(),
                std_deviation_y: value.std_dev_y().get(),
                color: color(value.color(), value.opacity().get()),
            },
            Some(value.input()),
            None,
        ),
        svg::Kind::Composite(value) => (
            FilterPrimitive::Composite {
                operator: composite(value.operator()),
            },
            Some(value.input1()),
            Some(value.input2()),
        ),
        svg::Kind::Blend(value) => (
            FilterPrimitive::Blend {
                mode: blend(value.mode()),
            },
            Some(value.input1()),
            Some(value.input2()),
        ),
        svg::Kind::ComponentTransfer(value) => (
            FilterPrimitive::ComponentTransfer {
                red_function: Some(transfer(value.func_r())),
                green_function: Some(transfer(value.func_g())),
                blue_function: Some(transfer(value.func_b())),
                alpha_function: Some(transfer(value.func_a())),
            },
            Some(value.input()),
            None,
        ),
        svg::Kind::Morphology(value) => (
            FilterPrimitive::Morphology {
                operator: match value.operator() {
                    svg::MorphologyOperator::Erode => MorphologyOperator::Erode,
                    svg::MorphologyOperator::Dilate => MorphologyOperator::Dilate,
                },
                radius_x: value.radius_x().get(),
                radius_y: value.radius_y().get(),
            },
            Some(value.input()),
            None,
        ),
        svg::Kind::ConvolveMatrix(value) => (convolve(value), Some(value.input()), None),
        svg::Kind::DisplacementMap(value) => (
            FilterPrimitive::DisplacementMap {
                scale: units.map_or(value.scale(), |units| {
                    units.displacement_scale(value.scale())
                }),
                x_channel: channel(value.x_channel_selector()),
                y_channel: channel(value.y_channel_selector()),
            },
            Some(value.input1()),
            Some(value.input2()),
        ),
        svg::Kind::Turbulence(value) => (
            FilterPrimitive::Turbulence {
                base_frequency_x: value.base_frequency_x().get(),
                base_frequency_y: value.base_frequency_y().get(),
                num_octaves: value.num_octaves(),
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "The engine already rounded the seed to an integer."
                )]
                seed: value.seed() as f32,
                stitch_tiles: value.stitch_tiles(),
                turbulence_type: match value.kind() {
                    svg::TurbulenceKind::FractalNoise => TurbulenceType::FractalNoise,
                    svg::TurbulenceKind::Turbulence => TurbulenceType::Turbulence,
                },
            },
            None,
            None,
        ),
        svg::Kind::Tile(value) => (FilterPrimitive::Tile, Some(value.input()), None),
        svg::Kind::DiffuseLighting(value) => (
            FilterPrimitive::DiffuseLighting {
                surface_scale: value.surface_scale(),
                diffuse_constant: value.diffuse_constant(),
                color: color(value.lighting_color(), 1.0),
                light_source: light(units.map_or(value.light_source(), |units| {
                    units.light(value.light_source())
                })),
            },
            Some(value.input()),
            None,
        ),
        svg::Kind::SpecularLighting(value) => (
            FilterPrimitive::SpecularLighting {
                surface_scale: value.surface_scale(),
                specular_constant: value.specular_constant(),
                specular_exponent: value.specular_exponent(),
                color: color(value.lighting_color(), 1.0),
                light_source: light(units.map_or(value.light_source(), |units| {
                    units.light(value.light_source())
                })),
            },
            Some(value.input()),
            None,
        ),
        // Image inputs carry resources; Merge expands in graph assembly.
        svg::Kind::Image(_) | svg::Kind::Merge(_) => return Err(limit_error()),
    };
    Ok(Operation {
        primitive,
        input,
        input2,
    })
}

fn color(color: blitz_dom::CanvasSvgColor, alpha: f32) -> AlphaColor<Srgb> {
    AlphaColor::new([
        f32::from(color.red) / 255.0,
        f32::from(color.green) / 255.0,
        f32::from(color.blue) / 255.0,
        alpha,
    ])
}

fn matrix(value: &svg::ColorMatrixKind) -> Result<[f32; 20], GraphError> {
    Ok(match value {
        svg::ColorMatrixKind::Matrix(values) => {
            values.as_slice().try_into().map_err(|_| limit_error())?
        }
        svg::ColorMatrixKind::Saturate(amount) => css_matrix(FilterFunction::Saturate {
            amount: amount.get(),
        })?,
        svg::ColorMatrixKind::HueRotate(angle) => {
            css_matrix(FilterFunction::HueRotate { angle: *angle })?
        }
        svg::ColorMatrixKind::LuminanceToAlpha => {
            let mut matrix = [0.0; 20];
            matrix[15] = 0.2125;
            matrix[16] = 0.7154;
            matrix[17] = 0.0721;
            matrix
        }
    })
}

fn css_matrix(function: FilterFunction) -> Result<[f32; 20], GraphError> {
    match Filter::from_function(function).graph.primitives.first() {
        Some(FilterPrimitive::ColorMatrix { matrix }) => Ok(*matrix),
        _ => Err(limit_error()),
    }
}

fn composite(value: svg::CompositeOperator) -> CompositeOperator {
    match value {
        svg::CompositeOperator::Over => CompositeOperator::Over,
        svg::CompositeOperator::In => CompositeOperator::In,
        svg::CompositeOperator::Out => CompositeOperator::Out,
        svg::CompositeOperator::Atop => CompositeOperator::Atop,
        svg::CompositeOperator::Xor => CompositeOperator::Xor,
        svg::CompositeOperator::Arithmetic { k1, k2, k3, k4 } => {
            CompositeOperator::Arithmetic { k1, k2, k3, k4 }
        }
    }
}

fn transfer(value: &svg::TransferFunction) -> TransferFunction {
    match value {
        svg::TransferFunction::Identity => TransferFunction::Identity,
        svg::TransferFunction::Table(values) => TransferFunction::Table {
            values: values.clone(),
        },
        svg::TransferFunction::Discrete(values) => TransferFunction::Discrete {
            values: values.clone(),
        },
        svg::TransferFunction::Linear { slope, intercept } => TransferFunction::Linear {
            slope: *slope,
            intercept: *intercept,
        },
        svg::TransferFunction::Gamma {
            amplitude,
            exponent,
            offset,
        } => TransferFunction::Gamma {
            amplitude: *amplitude,
            exponent: *exponent,
            offset: *offset,
        },
    }
}

fn blend(mode: blitz_dom::CanvasSvgBlendMode) -> vello_common::peniko::Mix {
    use blitz_dom::CanvasSvgBlendMode as S;
    use vello_common::peniko::Mix as V;
    match mode {
        S::Normal => V::Normal,
        S::Multiply => V::Multiply,
        S::Screen => V::Screen,
        S::Overlay => V::Overlay,
        S::Darken => V::Darken,
        S::Lighten => V::Lighten,
        S::ColorDodge => V::ColorDodge,
        S::ColorBurn => V::ColorBurn,
        S::HardLight => V::HardLight,
        S::SoftLight => V::SoftLight,
        S::Difference => V::Difference,
        S::Exclusion => V::Exclusion,
        S::Hue => V::Hue,
        S::Saturation => V::Saturation,
        S::Color => V::Color,
        S::Luminosity => V::Luminosity,
    }
}

fn convolve(value: &svg::ConvolveMatrix) -> FilterPrimitive {
    let matrix = value.matrix();
    FilterPrimitive::ConvolveMatrix {
        kernel: ConvolutionKernel {
            columns: matrix.columns(),
            rows: matrix.rows(),
            values: matrix.data().to_vec(),
            target_x: matrix.target_x(),
            target_y: matrix.target_y(),
            divisor: value.divisor().get(),
            bias: value.bias(),
            edge_mode: match value.edge_mode() {
                svg::EdgeMode::None => EdgeMode::None,
                svg::EdgeMode::Duplicate => EdgeMode::Duplicate,
                svg::EdgeMode::Wrap => EdgeMode::Wrap,
            },
            preserve_alpha: value.preserve_alpha(),
        },
    }
}

fn channel(value: svg::ColorChannel) -> ColorChannel {
    match value {
        svg::ColorChannel::R => ColorChannel::Red,
        svg::ColorChannel::G => ColorChannel::Green,
        svg::ColorChannel::B => ColorChannel::Blue,
        svg::ColorChannel::A => ColorChannel::Alpha,
    }
}

fn light(value: svg::LightSource) -> LightSource {
    match value {
        svg::LightSource::DistantLight(light) => LightSource::Distant {
            azimuth: light.azimuth,
            elevation: light.elevation,
        },
        svg::LightSource::PointLight(light) => LightSource::Point {
            x: light.x,
            y: light.y,
            z: light.z,
        },
        svg::LightSource::SpotLight(light) => LightSource::Spot {
            x: light.x,
            y: light.y,
            z: light.z,
            points_at_x: light.points_at_x,
            points_at_y: light.points_at_y,
            points_at_z: light.points_at_z,
            specular_exponent: light.specular_exponent.get(),
            limiting_cone_angle: light.limiting_cone_angle,
        },
    }
}
