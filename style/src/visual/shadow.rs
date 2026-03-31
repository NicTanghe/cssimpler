use cssimpler_core::{BoxShadow as CoreBoxShadow, Color, Style};
use lightningcss::properties::box_shadow::BoxShadow;
use lightningcss::values::length::Length;

use crate::{Declaration, StyleError};

use super::color;

#[derive(Clone, Debug, PartialEq)]
pub struct ShadowDeclaration {
    color: Option<Color>,
    offset_x: f32,
    offset_y: f32,
    blur_radius: f32,
    spread: f32,
}

pub(super) fn box_shadow_declarations(
    shadows: &[BoxShadow],
) -> Result<Vec<Declaration>, StyleError> {
    Ok(vec![Declaration::BoxShadows(
        shadows
            .iter()
            .filter(|shadow| !shadow.inset)
            .map(|shadow| {
                Ok(ShadowDeclaration {
                    color: color::color_from_css_optional(&shadow.color)?,
                    offset_x: length_to_px(&shadow.x_offset)?,
                    offset_y: length_to_px(&shadow.y_offset)?,
                    blur_radius: length_to_px(&shadow.blur)?,
                    spread: length_to_px(&shadow.spread)?,
                })
            })
            .collect::<Result<Vec<_>, _>>()?,
    )])
}

pub(super) fn apply_box_shadows(style: &mut Style, shadows: &[ShadowDeclaration]) {
    style.visual.shadows = shadows
        .iter()
        .map(|shadow| CoreBoxShadow {
            color: shadow.color.unwrap_or(style.visual.foreground),
            offset_x: shadow.offset_x,
            offset_y: shadow.offset_y,
            blur_radius: shadow.blur_radius,
            spread: shadow.spread,
        })
        .collect();
}

fn length_to_px(value: &Length) -> Result<f32, StyleError> {
    value
        .to_px()
        .map(|value| value as f32)
        .ok_or_else(|| StyleError::UnsupportedValue(format!("{value:?}")))
}
