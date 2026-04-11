use cssimpler_core::{
    LengthPercentageValue, Style, TransformMatrix3d, TransformOperation, TransformOrigin,
    TransformStyleMode,
};
use lightningcss::properties::PropertyId;
use lightningcss::properties::custom::{Token, TokenOrValue, UnparsedProperty};
use lightningcss::properties::transform::{
    Matrix3d as CssMatrix3d, Perspective as CssPerspective, Rotate as CssRotate, Scale as CssScale,
    Transform as CssTransform, TransformList as CssTransformList,
    TransformStyle as CssTransformStyle, Translate as CssTranslate,
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
            .try_fold(Vec::new(), |mut operations, transform| {
                operations.extend(transform_operations_from_css(transform)?);
                Ok::<_, StyleError>(operations)
            })?,
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

pub(super) fn perspective_declarations(
    value: &CssPerspective,
) -> Result<Vec<Declaration>, StyleError> {
    Ok(vec![Declaration::Perspective(match value {
        CssPerspective::None => None,
        CssPerspective::Length(length) => Some(length_in_px(length)?),
    })])
}

pub(super) fn transform_style_declarations(
    value: &CssTransformStyle,
) -> Result<Vec<Declaration>, StyleError> {
    Ok(vec![Declaration::TransformStyle(match value {
        CssTransformStyle::Flat => TransformStyleMode::Flat,
        CssTransformStyle::Preserve3d => TransformStyleMode::Preserve3d,
    })])
}

pub(super) fn unparsed_transform_style_declarations(
    value: &UnparsedProperty<'_>,
) -> Result<Vec<Declaration>, StyleError> {
    if !matches!(value.property_id, PropertyId::TransformStyle(_)) {
        return Err(StyleError::UnsupportedValue(format!(
            "{:?}",
            value.property_id
        )));
    }

    let keyword = value.value.0.iter().find_map(|token| match token {
        TokenOrValue::Token(Token::Ident(ident)) => Some(ident.as_ref()),
        _ => None,
    });

    match keyword {
        Some(value) if value.eq_ignore_ascii_case("flat") => {
            Ok(vec![Declaration::TransformStyle(TransformStyleMode::Flat)])
        }
        Some(value) if value.eq_ignore_ascii_case("preserve-3d") => {
            Ok(vec![Declaration::TransformStyle(
                TransformStyleMode::Preserve3d,
            )])
        }
        _ => Err(StyleError::UnsupportedValue("transform-style".to_string())),
    }
}

pub(super) fn apply_transform_operations(style: &mut Style, operations: &[TransformOperation]) {
    style.visual.transform.operations = operations.to_vec();
}

pub(super) fn apply_transform_origin(style: &mut Style, origin: TransformOrigin) {
    style.visual.transform.origin = origin;
}

pub(super) fn apply_perspective(style: &mut Style, perspective: Option<f32>) {
    style.visual.perspective = perspective;
}

pub(super) fn apply_transform_style(style: &mut Style, mode: TransformStyleMode) {
    style.visual.transform_style = mode;
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

fn transform_operations_from_css(
    value: &CssTransform,
) -> Result<Vec<TransformOperation>, StyleError> {
    match value {
        CssTransform::Translate(x, y) => Ok(vec![TransformOperation::Translate {
            x: length_percentage_from_css(x)?,
            y: length_percentage_from_css(y)?,
        }]),
        CssTransform::TranslateX(x) => Ok(vec![TransformOperation::Translate {
            x: length_percentage_from_css(x)?,
            y: LengthPercentageValue::ZERO,
        }]),
        CssTransform::TranslateY(y) => Ok(vec![TransformOperation::Translate {
            x: LengthPercentageValue::ZERO,
            y: length_percentage_from_css(y)?,
        }]),
        CssTransform::TranslateZ(z) => Ok(vec![TransformOperation::TranslateZ {
            z: length_in_px(z)?,
        }]),
        CssTransform::Translate3d(x, y, z) if length_is_zero(z)? => {
            Ok(vec![TransformOperation::Translate {
                x: length_percentage_from_css(x)?,
                y: length_percentage_from_css(y)?,
            }])
        }
        CssTransform::Translate3d(x, y, z) => Ok(vec![
            TransformOperation::Translate {
                x: length_percentage_from_css(x)?,
                y: length_percentage_from_css(y)?,
            },
            TransformOperation::TranslateZ {
                z: length_in_px(z)?,
            },
        ]),
        CssTransform::Scale(x, y) => Ok(vec![TransformOperation::Scale {
            x: number_or_percentage_to_factor(x),
            y: number_or_percentage_to_factor(y),
        }]),
        CssTransform::ScaleX(x) => Ok(vec![TransformOperation::Scale {
            x: number_or_percentage_to_factor(x),
            y: 1.0,
        }]),
        CssTransform::ScaleY(y) => Ok(vec![TransformOperation::Scale {
            x: 1.0,
            y: number_or_percentage_to_factor(y),
        }]),
        CssTransform::ScaleZ(z) => matrix3d_transform_declarations(TransformMatrix3d::scale(
            1.0,
            1.0,
            number_or_percentage_to_factor(z),
        )),
        CssTransform::Scale3d(x, y, z)
            if (number_or_percentage_to_factor(z) - 1.0).abs() <= f32::EPSILON =>
        {
            Ok(vec![TransformOperation::Scale {
                x: number_or_percentage_to_factor(x),
                y: number_or_percentage_to_factor(y),
            }])
        }
        CssTransform::Scale3d(x, y, z) => {
            matrix3d_transform_declarations(TransformMatrix3d::scale(
                number_or_percentage_to_factor(x),
                number_or_percentage_to_factor(y),
                number_or_percentage_to_factor(z),
            ))
        }
        CssTransform::Rotate(angle) => Ok(vec![TransformOperation::Rotate {
            degrees: angle.to_degrees(),
        }]),
        CssTransform::RotateX(angle) => Ok(vec![TransformOperation::RotateX {
            degrees: angle.to_degrees(),
        }]),
        CssTransform::RotateY(angle) => Ok(vec![TransformOperation::RotateY {
            degrees: angle.to_degrees(),
        }]),
        CssTransform::RotateZ(angle) => Ok(vec![TransformOperation::RotateZ {
            degrees: angle.to_degrees(),
        }]),
        CssTransform::Rotate3d(x, y, z, angle) if *x == 0.0 && *y == 0.0 && *z == 1.0 => {
            Ok(vec![TransformOperation::RotateZ {
                degrees: angle.to_degrees(),
            }])
        }
        CssTransform::Rotate3d(x, y, z, angle) if *x == 1.0 && *y == 0.0 && *z == 0.0 => {
            Ok(vec![TransformOperation::RotateX {
                degrees: angle.to_degrees(),
            }])
        }
        CssTransform::Rotate3d(x, y, z, angle) if *x == 0.0 && *y == 1.0 && *z == 0.0 => {
            Ok(vec![TransformOperation::RotateY {
                degrees: angle.to_degrees(),
            }])
        }
        CssTransform::Rotate3d(x, y, z, angle) => matrix3d_transform_declarations(
            TransformMatrix3d::rotate(*x, *y, *z, angle.to_degrees()),
        ),
        CssTransform::Perspective(length) => matrix3d_transform_declarations(
            TransformMatrix3d::perspective(length_in_px(length)?)
                .ok_or_else(|| StyleError::UnsupportedValue(format!("{value:?}")))?,
        ),
        CssTransform::Matrix3d(matrix) => {
            matrix3d_transform_declarations(transform_matrix3d_from_css(matrix))
        }
        _ => Err(StyleError::UnsupportedValue(format!("{value:?}"))),
    }
}

fn matrix3d_transform_declarations(
    matrix: TransformMatrix3d,
) -> Result<Vec<TransformOperation>, StyleError> {
    if matrix.is_identity() {
        Ok(Vec::new())
    } else {
        Ok(vec![TransformOperation::Matrix3d { matrix }])
    }
}

fn transform_matrix3d_from_css(matrix: &CssMatrix3d<f32>) -> TransformMatrix3d {
    TransformMatrix3d {
        m11: matrix.m11,
        m12: matrix.m21,
        m13: matrix.m31,
        m14: matrix.m41,
        m21: matrix.m12,
        m22: matrix.m22,
        m23: matrix.m32,
        m24: matrix.m42,
        m31: matrix.m13,
        m32: matrix.m23,
        m33: matrix.m33,
        m34: matrix.m43,
        m41: matrix.m14,
        m42: matrix.m24,
        m43: matrix.m34,
        m44: matrix.m44,
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
