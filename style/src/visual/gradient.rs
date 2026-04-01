use cssimpler_core::{
    AnglePercentageValue, BackgroundLayer, CircleRadius as CoreCircleRadius,
    ConicGradient as CoreConicGradient, EllipseRadius as CoreEllipseRadius,
    GradientDirection as CoreGradientDirection, GradientHorizontal as CoreGradientHorizontal,
    GradientInterpolation, GradientPoint as CoreGradientPoint, GradientStop as CoreGradientStop,
    GradientVertical as CoreGradientVertical, LengthPercentageValue,
    LinearGradient as CoreLinearGradient, RadialGradient as CoreRadialGradient,
    RadialShape as CoreRadialShape, ShapeExtent as CoreShapeExtent, Style,
};
use lightningcss::properties::background::Background;
use lightningcss::values::angle::{Angle, AnglePercentage};
use lightningcss::values::gradient::{
    Circle as CssCircle, ConicGradient as CssConicGradient, Ellipse as CssEllipse,
    EndingShape as CssEndingShape, Gradient as CssGradient, GradientItem,
    LineDirection as CssLineDirection, LinearGradient as CssLinearGradient,
    RadialGradient as CssRadialGradient, ShapeExtent as CssShapeExtent,
};
use lightningcss::values::image::Image;
use lightningcss::values::length::{Length, LengthPercentage, LengthValue};
use lightningcss::values::position::{
    HorizontalPosition, HorizontalPositionKeyword as CssHorizontalPositionKeyword,
    Position as CssPosition, PositionComponent, VerticalPosition,
    VerticalPositionKeyword as CssVerticalPositionKeyword,
};

use crate::{Declaration, StyleError};

use super::color;

#[derive(Clone, Debug, PartialEq)]
pub enum BackgroundLayerDeclaration {
    LinearGradient(LinearGradientDeclaration),
    RadialGradient(RadialGradientDeclaration),
    ConicGradient(ConicGradientDeclaration),
}

#[derive(Clone, Debug, PartialEq)]
pub struct LinearGradientDeclaration {
    repeating: bool,
    direction: CoreGradientDirection,
    stops: Vec<GradientStopDeclaration<LengthPercentageValue>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RadialGradientDeclaration {
    repeating: bool,
    shape: CoreRadialShape,
    center: CoreGradientPoint,
    stops: Vec<GradientStopDeclaration<LengthPercentageValue>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ConicGradientDeclaration {
    repeating: bool,
    angle: f32,
    center: CoreGradientPoint,
    stops: Vec<GradientStopDeclaration<AnglePercentageValue>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GradientStopDeclaration<P> {
    color: Option<cssimpler_core::Color>,
    position: P,
}

#[derive(Clone, Debug)]
struct PendingGradientStop<P> {
    color: Option<cssimpler_core::Color>,
    position: Option<P>,
}

trait StopPosition: Copy {
    fn start() -> Self;
    fn end() -> Self;
    fn lerp(self, other: Self, t: f32) -> Self;
}

impl StopPosition for LengthPercentageValue {
    fn start() -> Self {
        Self::ZERO
    }

    fn end() -> Self {
        Self::from_fraction(1.0)
    }

    fn lerp(self, other: Self, t: f32) -> Self {
        self.lerp(other, t)
    }
}

impl StopPosition for AnglePercentageValue {
    fn start() -> Self {
        Self::ZERO
    }

    fn end() -> Self {
        Self::from_turns(1.0)
    }

    fn lerp(self, other: Self, t: f32) -> Self {
        self.lerp(other, t)
    }
}

pub(super) fn background_declarations(
    backgrounds: &[Background<'_>],
) -> Result<Vec<Declaration>, StyleError> {
    let Some(last_background) = backgrounds.last() else {
        return Ok(Vec::new());
    };

    Ok(vec![
        Declaration::Background(color::color_from_css(&last_background.color)?),
        Declaration::BackgroundLayers(background_layers_from_backgrounds(backgrounds)?),
    ])
}

pub(super) fn background_image_declarations(
    images: &[Image<'_>],
) -> Result<Vec<Declaration>, StyleError> {
    Ok(vec![Declaration::BackgroundLayers(
        background_layers_from_images(images)?,
    )])
}

pub(super) fn apply_background_layers(style: &mut Style, layers: &[BackgroundLayerDeclaration]) {
    style.visual.background_layers = layers
        .iter()
        .map(|layer| background_layer_from_declaration(layer, style.visual.foreground))
        .collect();
}

fn background_layers_from_backgrounds(
    backgrounds: &[Background<'_>],
) -> Result<Vec<BackgroundLayerDeclaration>, StyleError> {
    let mut layers = Vec::new();

    for background in backgrounds {
        if let Some(layer) = background_layer_from_image(&background.image)? {
            layers.push(layer);
        }
    }

    Ok(layers)
}

fn background_layers_from_images(
    images: &[Image<'_>],
) -> Result<Vec<BackgroundLayerDeclaration>, StyleError> {
    let mut layers = Vec::new();

    for image in images {
        if let Some(layer) = background_layer_from_image(image)? {
            layers.push(layer);
        }
    }

    Ok(layers)
}

fn background_layer_from_declaration(
    declaration: &BackgroundLayerDeclaration,
    foreground: cssimpler_core::Color,
) -> BackgroundLayer {
    match declaration {
        BackgroundLayerDeclaration::LinearGradient(gradient) => {
            BackgroundLayer::LinearGradient(CoreLinearGradient {
                direction: gradient.direction,
                interpolation: GradientInterpolation::Oklab,
                repeating: gradient.repeating,
                stops: gradient
                    .stops
                    .iter()
                    .map(|stop| CoreGradientStop {
                        color: stop.color.unwrap_or(foreground),
                        position: stop.position,
                    })
                    .collect(),
            })
        }
        BackgroundLayerDeclaration::RadialGradient(gradient) => {
            BackgroundLayer::RadialGradient(CoreRadialGradient {
                shape: gradient.shape,
                center: gradient.center,
                interpolation: GradientInterpolation::Oklab,
                repeating: gradient.repeating,
                stops: gradient
                    .stops
                    .iter()
                    .map(|stop| CoreGradientStop {
                        color: stop.color.unwrap_or(foreground),
                        position: stop.position,
                    })
                    .collect(),
            })
        }
        BackgroundLayerDeclaration::ConicGradient(gradient) => {
            BackgroundLayer::ConicGradient(CoreConicGradient {
                angle: gradient.angle,
                center: gradient.center,
                interpolation: GradientInterpolation::Oklab,
                repeating: gradient.repeating,
                stops: gradient
                    .stops
                    .iter()
                    .map(|stop| CoreGradientStop {
                        color: stop.color.unwrap_or(foreground),
                        position: stop.position,
                    })
                    .collect(),
            })
        }
    }
}

fn background_layer_from_image(
    image: &Image<'_>,
) -> Result<Option<BackgroundLayerDeclaration>, StyleError> {
    match image {
        Image::None => Ok(None),
        Image::Gradient(gradient) => gradient_from_css(gradient).map(Some),
        Image::Url(_) | Image::ImageSet(_) => {
            Err(StyleError::UnsupportedValue(format!("{image:?}")))
        }
    }
}

fn gradient_from_css(gradient: &CssGradient) -> Result<BackgroundLayerDeclaration, StyleError> {
    match gradient {
        CssGradient::Linear(gradient) => linear_gradient_from_css(gradient, false),
        CssGradient::RepeatingLinear(gradient) => linear_gradient_from_css(gradient, true),
        CssGradient::Radial(gradient) => radial_gradient_from_css(gradient, false),
        CssGradient::RepeatingRadial(gradient) => radial_gradient_from_css(gradient, true),
        CssGradient::Conic(gradient) => conic_gradient_from_css(gradient, false),
        CssGradient::RepeatingConic(gradient) => conic_gradient_from_css(gradient, true),
        CssGradient::WebKitGradient(_) => {
            Err(StyleError::UnsupportedValue(format!("{gradient:?}")))
        }
    }
}

fn linear_gradient_from_css(
    gradient: &CssLinearGradient,
    repeating: bool,
) -> Result<BackgroundLayerDeclaration, StyleError> {
    Ok(BackgroundLayerDeclaration::LinearGradient(
        LinearGradientDeclaration {
            repeating,
            direction: direction_from_css(&gradient.direction),
            stops: normalize_stops(
                gradient
                    .items
                    .iter()
                    .map(length_stop_from_css)
                    .collect::<Result<Vec<_>, _>>()?,
            )?,
        },
    ))
}

fn radial_gradient_from_css(
    gradient: &CssRadialGradient,
    repeating: bool,
) -> Result<BackgroundLayerDeclaration, StyleError> {
    Ok(BackgroundLayerDeclaration::RadialGradient(
        RadialGradientDeclaration {
            repeating,
            shape: radial_shape_from_css(&gradient.shape)?,
            center: point_from_css(&gradient.position)?,
            stops: normalize_stops(
                gradient
                    .items
                    .iter()
                    .map(length_stop_from_css)
                    .collect::<Result<Vec<_>, _>>()?,
            )?,
        },
    ))
}

fn conic_gradient_from_css(
    gradient: &CssConicGradient,
    repeating: bool,
) -> Result<BackgroundLayerDeclaration, StyleError> {
    Ok(BackgroundLayerDeclaration::ConicGradient(
        ConicGradientDeclaration {
            repeating,
            angle: angle_to_degrees(&gradient.angle),
            center: point_from_css(&gradient.position)?,
            stops: normalize_stops(
                gradient
                    .items
                    .iter()
                    .map(angle_stop_from_css)
                    .collect::<Result<Vec<_>, _>>()?,
            )?,
        },
    ))
}

fn length_stop_from_css(
    item: &GradientItem<LengthPercentage>,
) -> Result<PendingGradientStop<LengthPercentageValue>, StyleError> {
    match item {
        GradientItem::ColorStop(stop) => Ok(PendingGradientStop {
            color: color::color_from_css_optional(&stop.color)?,
            position: stop
                .position
                .as_ref()
                .map(length_percentage_from_css)
                .transpose()?,
        }),
        GradientItem::Hint(_) => Err(StyleError::UnsupportedValue(
            "gradient interpolation hints are not supported".to_string(),
        )),
    }
}

fn angle_stop_from_css(
    item: &GradientItem<AnglePercentage>,
) -> Result<PendingGradientStop<AnglePercentageValue>, StyleError> {
    match item {
        GradientItem::ColorStop(stop) => Ok(PendingGradientStop {
            color: color::color_from_css_optional(&stop.color)?,
            position: stop
                .position
                .as_ref()
                .map(angle_percentage_from_css)
                .transpose()?,
        }),
        GradientItem::Hint(_) => Err(StyleError::UnsupportedValue(
            "gradient interpolation hints are not supported".to_string(),
        )),
    }
}

fn normalize_stops<P: StopPosition>(
    mut stops: Vec<PendingGradientStop<P>>,
) -> Result<Vec<GradientStopDeclaration<P>>, StyleError> {
    if stops.is_empty() {
        return Err(StyleError::UnsupportedValue(
            "gradients need at least one color stop".to_string(),
        ));
    }

    if stops.len() == 1 && stops[0].position.is_none() {
        stops[0].position = Some(P::start());
    }

    if stops.first().is_some_and(|stop| stop.position.is_none()) {
        stops[0].position = Some(P::start());
    }

    let last_index = stops.len() - 1;
    if stops[last_index].position.is_none() {
        stops[last_index].position = Some(P::end());
    }

    let mut index = 0;
    while index < stops.len() {
        if stops[index].position.is_some() {
            index += 1;
            continue;
        }

        let start_index = index - 1;
        let start_position = stops[start_index].position.unwrap_or(P::start());
        let mut end_index = index;
        while end_index < stops.len() && stops[end_index].position.is_none() {
            end_index += 1;
        }

        let end_position = stops[end_index].position.unwrap_or(start_position);
        let span = (end_index - start_index) as f32;
        for offset in 1..(end_index - start_index) {
            let t = offset as f32 / span;
            stops[start_index + offset].position = Some(start_position.lerp(end_position, t));
        }

        index = end_index + 1;
    }

    Ok(stops
        .into_iter()
        .map(|stop| GradientStopDeclaration {
            color: stop.color,
            position: stop.position.unwrap_or(P::start()),
        })
        .collect())
}

fn length_percentage_from_css(
    value: &LengthPercentage,
) -> Result<LengthPercentageValue, StyleError> {
    match value {
        LengthPercentage::Dimension(length) => {
            Ok(LengthPercentageValue::from_px(length_value_to_px(length)?))
        }
        LengthPercentage::Percentage(percentage) => {
            Ok(LengthPercentageValue::from_fraction(percentage.0))
        }
        _ => Err(StyleError::UnsupportedValue(format!("{value:?}"))),
    }
}

fn angle_percentage_from_css(value: &AnglePercentage) -> Result<AnglePercentageValue, StyleError> {
    match value {
        AnglePercentage::Dimension(angle) => {
            Ok(AnglePercentageValue::from_degrees(angle_to_degrees(angle)))
        }
        AnglePercentage::Percentage(percentage) => {
            Ok(AnglePercentageValue::from_turns(percentage.0))
        }
        _ => Err(StyleError::UnsupportedValue(format!("{value:?}"))),
    }
}

fn point_from_css(position: &CssPosition) -> Result<CoreGradientPoint, StyleError> {
    Ok(CoreGradientPoint {
        x: horizontal_position_from_css(&position.x)?,
        y: vertical_position_from_css(&position.y)?,
    })
}

fn horizontal_position_from_css(
    position: &HorizontalPosition,
) -> Result<LengthPercentageValue, StyleError> {
    match position {
        PositionComponent::Center => Ok(LengthPercentageValue::from_fraction(0.5)),
        PositionComponent::Length(value) => length_percentage_from_css(value),
        PositionComponent::Side { side, offset } => {
            let offset = offset
                .as_ref()
                .map(length_percentage_from_css)
                .transpose()?
                .unwrap_or(LengthPercentageValue::ZERO);
            match side {
                CssHorizontalPositionKeyword::Left => Ok(offset),
                CssHorizontalPositionKeyword::Right => Ok(LengthPercentageValue {
                    px: -offset.px,
                    fraction: 1.0 - offset.fraction,
                }),
            }
        }
    }
}

fn vertical_position_from_css(
    position: &VerticalPosition,
) -> Result<LengthPercentageValue, StyleError> {
    match position {
        PositionComponent::Center => Ok(LengthPercentageValue::from_fraction(0.5)),
        PositionComponent::Length(value) => length_percentage_from_css(value),
        PositionComponent::Side { side, offset } => {
            let offset = offset
                .as_ref()
                .map(length_percentage_from_css)
                .transpose()?
                .unwrap_or(LengthPercentageValue::ZERO);
            match side {
                CssVerticalPositionKeyword::Top => Ok(offset),
                CssVerticalPositionKeyword::Bottom => Ok(LengthPercentageValue {
                    px: -offset.px,
                    fraction: 1.0 - offset.fraction,
                }),
            }
        }
    }
}

fn radial_shape_from_css(shape: &CssEndingShape) -> Result<CoreRadialShape, StyleError> {
    match shape {
        CssEndingShape::Circle(circle) => {
            Ok(CoreRadialShape::Circle(circle_radius_from_css(circle)?))
        }
        CssEndingShape::Ellipse(ellipse) => {
            Ok(CoreRadialShape::Ellipse(ellipse_radius_from_css(ellipse)?))
        }
    }
}

fn circle_radius_from_css(circle: &CssCircle) -> Result<CoreCircleRadius, StyleError> {
    match circle {
        CssCircle::Radius(length) => Ok(CoreCircleRadius::Explicit(length_to_px(length)?)),
        CssCircle::Extent(extent) => Ok(CoreCircleRadius::Extent(shape_extent_from_css(*extent))),
    }
}

fn ellipse_radius_from_css(ellipse: &CssEllipse) -> Result<CoreEllipseRadius, StyleError> {
    match ellipse {
        CssEllipse::Size { x, y } => Ok(CoreEllipseRadius::Explicit {
            x: length_percentage_from_css(x)?,
            y: length_percentage_from_css(y)?,
        }),
        CssEllipse::Extent(extent) => Ok(CoreEllipseRadius::Extent(shape_extent_from_css(*extent))),
    }
}

fn shape_extent_from_css(extent: CssShapeExtent) -> CoreShapeExtent {
    match extent {
        CssShapeExtent::ClosestSide => CoreShapeExtent::ClosestSide,
        CssShapeExtent::FarthestSide => CoreShapeExtent::FarthestSide,
        CssShapeExtent::ClosestCorner => CoreShapeExtent::ClosestCorner,
        CssShapeExtent::FarthestCorner => CoreShapeExtent::FarthestCorner,
    }
}

fn direction_from_css(direction: &CssLineDirection) -> CoreGradientDirection {
    match direction {
        CssLineDirection::Angle(angle) => CoreGradientDirection::Angle(angle_to_degrees(angle)),
        CssLineDirection::Horizontal(direction) => {
            CoreGradientDirection::Horizontal(horizontal_from_css(*direction))
        }
        CssLineDirection::Vertical(direction) => {
            CoreGradientDirection::Vertical(vertical_from_css(*direction))
        }
        CssLineDirection::Corner {
            horizontal,
            vertical,
        } => CoreGradientDirection::Corner {
            horizontal: horizontal_from_css(*horizontal),
            vertical: vertical_from_css(*vertical),
        },
    }
}

fn horizontal_from_css(direction: CssHorizontalPositionKeyword) -> CoreGradientHorizontal {
    match direction {
        CssHorizontalPositionKeyword::Left => CoreGradientHorizontal::Left,
        CssHorizontalPositionKeyword::Right => CoreGradientHorizontal::Right,
    }
}

fn vertical_from_css(direction: CssVerticalPositionKeyword) -> CoreGradientVertical {
    match direction {
        CssVerticalPositionKeyword::Top => CoreGradientVertical::Top,
        CssVerticalPositionKeyword::Bottom => CoreGradientVertical::Bottom,
    }
}

fn angle_to_degrees(angle: &Angle) -> f32 {
    angle.to_degrees() as f32
}

fn length_to_px(value: &Length) -> Result<f32, StyleError> {
    value
        .to_px()
        .map(|value| value as f32)
        .ok_or_else(|| StyleError::UnsupportedValue(format!("{value:?}")))
}

fn length_value_to_px(value: &LengthValue) -> Result<f32, StyleError> {
    value
        .to_px()
        .map(|value| value as f32)
        .ok_or_else(|| StyleError::UnsupportedValue(format!("{value:?}")))
}
