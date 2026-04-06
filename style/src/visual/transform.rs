use cssimpler_core::{LengthPercentageValue, Style, TransformOperation, TransformOrigin};
use lightningcss::properties::transform::{
    Rotate as CssRotate, Scale as CssScale, Transform as CssTransform,
    TransformList as CssTransformList, Translate as CssTranslate,
};
use lightningcss::values::length::{Length, LengthPercentage};
use lightningcss::values::percentage::NumberOrPercentage;
use lightningcss::values::position::{
    HorizontalPosition, HorizontalPositionKeyword, Position, VerticalPosition,
    VerticalPositionKeyword,
};

use crate::{Declaration, StyleError};

pub(super) fn transform_declarations(
    transforms: &CssTransformList,
) -> Result<Vec<Declaration>, StyleError> {
    Ok(vec![Declaration::TransformOperations(
        transforms
            .0
            .iter()
            .map(transform_operation_from_css)
            .collect::<Result<Vec<_>, _>>()?,
    )])
}

pub(super) fn transform_origin_declarations(
    position: &Position,
) -> Result<Vec<Declaration>, StyleError> {
    Ok(vec![Declaration::TransformOrigin(
        transform_origin_from_css(position)?,
    )])
}

pub(super) fn translate_declarations(value: &CssTranslate) -> Result<Vec<Declaration>, StyleError> {
    transform_declarations(&CssTransformList(vec![value.to_transform()]))
}

pub(super) fn rotate_declarations(value: &CssRotate) -> Result<Vec<Declaration>, StyleError> {
    transform_declarations(&CssTransformList(vec![value.to_transform()]))
}

pub(super) fn scale_declarations(value: &CssScale) -> Result<Vec<Declaration>, StyleError> {
    transform_declarations(&CssTransformList(vec![value.to_transform()]))
}

pub(super) fn apply_transform_operations(style: &mut Style, operations: &[TransformOperation]) {
    style.visual.transform.operations = operations.to_vec();
}

pub(super) fn apply_transform_origin(style: &mut Style, origin: TransformOrigin) {
    style.visual.transform.origin = origin;
}

fn transform_origin_from_css(position: &Position) -> Result<TransformOrigin, StyleError> {
    Ok(TransformOrigin {
        x: horizontal_position_from_css(&position.x)?,
        y: vertical_position_from_css(&position.y)?,
    })
}

fn horizontal_position_from_css(
    value: &HorizontalPosition,
) -> Result<LengthPercentageValue, StyleError> {
    match value {
        HorizontalPosition::Center => Ok(LengthPercentageValue::from_fraction(0.5)),
        HorizontalPosition::Length(value) => length_percentage_from_css(value),
        HorizontalPosition::Side { side, offset } => {
            let offset = offset
                .as_ref()
                .map(length_percentage_from_css)
                .transpose()?
                .unwrap_or(LengthPercentageValue::ZERO);
            Ok(match side {
                HorizontalPositionKeyword::Left => offset,
                HorizontalPositionKeyword::Right => LengthPercentageValue {
                    px: -offset.px,
                    fraction: 1.0 - offset.fraction,
                },
            })
        }
    }
}

fn vertical_position_from_css(
    value: &VerticalPosition,
) -> Result<LengthPercentageValue, StyleError> {
    match value {
        VerticalPosition::Center => Ok(LengthPercentageValue::from_fraction(0.5)),
        VerticalPosition::Length(value) => length_percentage_from_css(value),
        VerticalPosition::Side { side, offset } => {
            let offset = offset
                .as_ref()
                .map(length_percentage_from_css)
                .transpose()?
                .unwrap_or(LengthPercentageValue::ZERO);
            Ok(match side {
                VerticalPositionKeyword::Top => offset,
                VerticalPositionKeyword::Bottom => LengthPercentageValue {
                    px: -offset.px,
                    fraction: 1.0 - offset.fraction,
                },
            })
        }
    }
}

fn transform_operation_from_css(value: &CssTransform) -> Result<TransformOperation, StyleError> {
    match value {
        CssTransform::Translate(x, y) => Ok(TransformOperation::Translate {
            x: length_percentage_from_css(x)?,
            y: length_percentage_from_css(y)?,
        }),
        CssTransform::TranslateX(x) => Ok(TransformOperation::Translate {
            x: length_percentage_from_css(x)?,
            y: LengthPercentageValue::ZERO,
        }),
        CssTransform::TranslateY(y) => Ok(TransformOperation::Translate {
            x: LengthPercentageValue::ZERO,
            y: length_percentage_from_css(y)?,
        }),
        CssTransform::Translate3d(x, y, z) if length_is_zero(z)? => {
            Ok(TransformOperation::Translate {
                x: length_percentage_from_css(x)?,
                y: length_percentage_from_css(y)?,
            })
        }
        CssTransform::Scale(x, y) => Ok(TransformOperation::Scale {
            x: number_or_percentage_to_factor(x),
            y: number_or_percentage_to_factor(y),
        }),
        CssTransform::ScaleX(x) => Ok(TransformOperation::Scale {
            x: number_or_percentage_to_factor(x),
            y: 1.0,
        }),
        CssTransform::ScaleY(y) => Ok(TransformOperation::Scale {
            x: 1.0,
            y: number_or_percentage_to_factor(y),
        }),
        CssTransform::Scale3d(x, y, z)
            if (number_or_percentage_to_factor(z) - 1.0).abs() <= f32::EPSILON =>
        {
            Ok(TransformOperation::Scale {
                x: number_or_percentage_to_factor(x),
                y: number_or_percentage_to_factor(y),
            })
        }
        CssTransform::Rotate(angle) | CssTransform::RotateZ(angle) => {
            Ok(TransformOperation::Rotate {
                degrees: angle.to_degrees(),
            })
        }
        CssTransform::Rotate3d(x, y, z, angle) if *x == 0.0 && *y == 0.0 && *z == 1.0 => {
            Ok(TransformOperation::Rotate {
                degrees: angle.to_degrees(),
            })
        }
        _ => Err(StyleError::UnsupportedValue(format!("{value:?}"))),
    }
}

fn length_percentage_from_css(
    value: &LengthPercentage,
) -> Result<LengthPercentageValue, StyleError> {
    match value {
        LengthPercentage::Dimension(length) => Ok(LengthPercentageValue::from_px(
            length
                .to_px()
                .map(|value| value as f32)
                .ok_or_else(|| StyleError::UnsupportedValue(format!("{value:?}")))?,
        )),
        LengthPercentage::Percentage(percentage) => {
            Ok(LengthPercentageValue::from_fraction(percentage.0))
        }
        _ => Err(StyleError::UnsupportedValue(format!("{value:?}"))),
    }
}

fn length_in_px(value: &Length) -> Result<f32, StyleError> {
    value
        .to_px()
        .map(|value| value as f32)
        .ok_or_else(|| StyleError::UnsupportedValue(format!("{value:?}")))
}

fn length_is_zero(value: &Length) -> Result<bool, StyleError> {
    Ok(length_in_px(value)?.abs() <= f32::EPSILON)
}

fn number_or_percentage_to_factor(value: &NumberOrPercentage) -> f32 {
    match value {
        NumberOrPercentage::Number(value) => *value,
        NumberOrPercentage::Percentage(value) => value.0,
    }
}
