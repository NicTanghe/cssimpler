use cssimpler_core::{Color, Style};
use lightningcss::values::color::CssColor;

use crate::{Declaration, StyleError};

pub(super) fn background_color_declaration(
    color: &CssColor,
) -> Result<Vec<Declaration>, StyleError> {
    Ok(vec![Declaration::Background(color_from_css(color)?)])
}

pub(super) fn foreground_color_declaration(
    color: &CssColor,
) -> Result<Vec<Declaration>, StyleError> {
    Ok(vec![Declaration::Foreground(color_from_css(color)?)])
}

pub(super) fn apply_background(style: &mut Style, color: Color) {
    style.visual.background = Some(color);
}

pub(super) fn apply_foreground(style: &mut Style, color: Color) {
    style.visual.foreground = color;
}

pub(super) fn color_from_css(color: &CssColor) -> Result<Color, StyleError> {
    let rgb = color
        .to_rgb()
        .map_err(|_| StyleError::UnsupportedValue(format!("{color:?}")))?;

    match rgb {
        CssColor::RGBA(rgba) => Ok(Color::rgba(rgba.red, rgba.green, rgba.blue, rgba.alpha)),
        _ => Err(StyleError::UnsupportedValue(format!("{color:?}"))),
    }
}

pub(super) fn color_from_css_optional(color: &CssColor) -> Result<Option<Color>, StyleError> {
    match color {
        CssColor::CurrentColor => Ok(None),
        _ => color_from_css(color).map(Some),
    }
}
