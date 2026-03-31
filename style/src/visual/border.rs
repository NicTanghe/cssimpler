use cssimpler_core::{Color, Style};
use lightningcss::properties::border::{
    Border, BorderBottom, BorderLeft, BorderRight, BorderSideWidth as CssBorderSideWidth,
    BorderTop, BorderWidth, LineStyle as CssLineStyle,
};
use lightningcss::properties::border_radius::BorderRadius;
use lightningcss::values::color::CssColor;
use lightningcss::values::length::LengthPercentage;
use lightningcss::values::size::Size2D;
use taffy::prelude::LengthPercentage as TaffyLengthPercentage;

use crate::{Declaration, StyleError};

use super::color;

pub(super) fn border_radius_declarations(
    radius: &BorderRadius,
) -> Result<Vec<Declaration>, StyleError> {
    Ok(vec![
        Declaration::CornerTopLeft(corner_radius_to_px(&radius.top_left)?),
        Declaration::CornerTopRight(corner_radius_to_px(&radius.top_right)?),
        Declaration::CornerBottomRight(corner_radius_to_px(&radius.bottom_right)?),
        Declaration::CornerBottomLeft(corner_radius_to_px(&radius.bottom_left)?),
    ])
}

pub(super) fn border_top_left_radius_declaration(
    radius: &Size2D<LengthPercentage>,
) -> Result<Vec<Declaration>, StyleError> {
    Ok(vec![Declaration::CornerTopLeft(corner_radius_to_px(
        radius,
    )?)])
}

pub(super) fn border_top_right_radius_declaration(
    radius: &Size2D<LengthPercentage>,
) -> Result<Vec<Declaration>, StyleError> {
    Ok(vec![Declaration::CornerTopRight(corner_radius_to_px(
        radius,
    )?)])
}

pub(super) fn border_bottom_right_radius_declaration(
    radius: &Size2D<LengthPercentage>,
) -> Result<Vec<Declaration>, StyleError> {
    Ok(vec![Declaration::CornerBottomRight(corner_radius_to_px(
        radius,
    )?)])
}

pub(super) fn border_bottom_left_radius_declaration(
    radius: &Size2D<LengthPercentage>,
) -> Result<Vec<Declaration>, StyleError> {
    Ok(vec![Declaration::CornerBottomLeft(corner_radius_to_px(
        radius,
    )?)])
}

pub(super) fn border_shorthand_declarations(
    border: &Border,
) -> Result<Vec<Declaration>, StyleError> {
    if matches!(border.style, CssLineStyle::None | CssLineStyle::Hidden) {
        return Ok(vec![
            Declaration::BorderTopWidth(0.0),
            Declaration::BorderRightWidth(0.0),
            Declaration::BorderBottomWidth(0.0),
            Declaration::BorderLeftWidth(0.0),
        ]);
    }

    let width = border_width_to_px(&border.width)?;
    Ok(vec![
        Declaration::BorderTopWidth(width),
        Declaration::BorderRightWidth(width),
        Declaration::BorderBottomWidth(width),
        Declaration::BorderLeftWidth(width),
        Declaration::BorderColor(color::color_from_css_optional(&border.color)?),
    ])
}

pub(super) fn border_top_declarations(border: &BorderTop) -> Result<Vec<Declaration>, StyleError> {
    extract_border_side_declarations(border, BorderSide::Top)
}

pub(super) fn border_right_declarations(
    border: &BorderRight,
) -> Result<Vec<Declaration>, StyleError> {
    extract_border_side_declarations(border, BorderSide::Right)
}

pub(super) fn border_bottom_declarations(
    border: &BorderBottom,
) -> Result<Vec<Declaration>, StyleError> {
    extract_border_side_declarations(border, BorderSide::Bottom)
}

pub(super) fn border_left_declarations(
    border: &BorderLeft,
) -> Result<Vec<Declaration>, StyleError> {
    extract_border_side_declarations(border, BorderSide::Left)
}

pub(super) fn border_width_declarations(
    widths: &BorderWidth,
) -> Result<Vec<Declaration>, StyleError> {
    Ok(vec![
        Declaration::BorderTopWidth(border_width_to_px(&widths.top)?),
        Declaration::BorderRightWidth(border_width_to_px(&widths.right)?),
        Declaration::BorderBottomWidth(border_width_to_px(&widths.bottom)?),
        Declaration::BorderLeftWidth(border_width_to_px(&widths.left)?),
    ])
}

pub(super) fn border_top_width_declaration(
    value: &CssBorderSideWidth,
) -> Result<Vec<Declaration>, StyleError> {
    Ok(vec![Declaration::BorderTopWidth(border_width_to_px(
        value,
    )?)])
}

pub(super) fn border_right_width_declaration(
    value: &CssBorderSideWidth,
) -> Result<Vec<Declaration>, StyleError> {
    Ok(vec![Declaration::BorderRightWidth(border_width_to_px(
        value,
    )?)])
}

pub(super) fn border_bottom_width_declaration(
    value: &CssBorderSideWidth,
) -> Result<Vec<Declaration>, StyleError> {
    Ok(vec![Declaration::BorderBottomWidth(border_width_to_px(
        value,
    )?)])
}

pub(super) fn border_left_width_declaration(
    value: &CssBorderSideWidth,
) -> Result<Vec<Declaration>, StyleError> {
    Ok(vec![Declaration::BorderLeftWidth(border_width_to_px(
        value,
    )?)])
}

pub(super) fn border_color_declaration(color: &CssColor) -> Result<Vec<Declaration>, StyleError> {
    Ok(vec![Declaration::BorderColor(
        color::color_from_css_optional(color)?,
    )])
}

pub(super) fn apply_corner_top_left(style: &mut Style, value: f32) {
    style.visual.corner_radius.top_left = value;
}

pub(super) fn apply_corner_top_right(style: &mut Style, value: f32) {
    style.visual.corner_radius.top_right = value;
}

pub(super) fn apply_corner_bottom_right(style: &mut Style, value: f32) {
    style.visual.corner_radius.bottom_right = value;
}

pub(super) fn apply_corner_bottom_left(style: &mut Style, value: f32) {
    style.visual.corner_radius.bottom_left = value;
}

pub(super) fn apply_border_top_width(style: &mut Style, value: f32) {
    style.visual.border.widths.top = value;
    style.layout.taffy.border.top = TaffyLengthPercentage::Length(value);
}

pub(super) fn apply_border_right_width(style: &mut Style, value: f32) {
    style.visual.border.widths.right = value;
    style.layout.taffy.border.right = TaffyLengthPercentage::Length(value);
}

pub(super) fn apply_border_bottom_width(style: &mut Style, value: f32) {
    style.visual.border.widths.bottom = value;
    style.layout.taffy.border.bottom = TaffyLengthPercentage::Length(value);
}

pub(super) fn apply_border_left_width(style: &mut Style, value: f32) {
    style.visual.border.widths.left = value;
    style.layout.taffy.border.left = TaffyLengthPercentage::Length(value);
}

pub(super) fn apply_border_color(style: &mut Style, color: Option<Color>) {
    style.visual.border.color = color.unwrap_or(style.visual.foreground);
}

#[derive(Clone, Copy)]
enum BorderSide {
    Top,
    Right,
    Bottom,
    Left,
}

trait BorderSideAccess {
    fn width(&self) -> &CssBorderSideWidth;
    fn line_style(&self) -> CssLineStyle;
    fn color(&self) -> &CssColor;
}

impl BorderSideAccess for BorderTop {
    fn width(&self) -> &CssBorderSideWidth {
        &self.width
    }

    fn line_style(&self) -> CssLineStyle {
        self.style
    }

    fn color(&self) -> &CssColor {
        &self.color
    }
}

impl BorderSideAccess for BorderRight {
    fn width(&self) -> &CssBorderSideWidth {
        &self.width
    }

    fn line_style(&self) -> CssLineStyle {
        self.style
    }

    fn color(&self) -> &CssColor {
        &self.color
    }
}

impl BorderSideAccess for BorderBottom {
    fn width(&self) -> &CssBorderSideWidth {
        &self.width
    }

    fn line_style(&self) -> CssLineStyle {
        self.style
    }

    fn color(&self) -> &CssColor {
        &self.color
    }
}

impl BorderSideAccess for BorderLeft {
    fn width(&self) -> &CssBorderSideWidth {
        &self.width
    }

    fn line_style(&self) -> CssLineStyle {
        self.style
    }

    fn color(&self) -> &CssColor {
        &self.color
    }
}

fn extract_border_side_declarations<T>(
    border: &T,
    side: BorderSide,
) -> Result<Vec<Declaration>, StyleError>
where
    T: BorderSideAccess,
{
    if matches!(
        border.line_style(),
        CssLineStyle::None | CssLineStyle::Hidden
    ) {
        return Ok(vec![border_width_declaration(side, 0.0)]);
    }

    Ok(vec![
        border_width_declaration(side, border_width_to_px(border.width())?),
        Declaration::BorderColor(color::color_from_css_optional(border.color())?),
    ])
}

fn corner_radius_to_px(value: &Size2D<LengthPercentage>) -> Result<f32, StyleError> {
    length_percentage_to_px(&value.0)
}

fn length_percentage_to_px(value: &LengthPercentage) -> Result<f32, StyleError> {
    match value {
        LengthPercentage::Dimension(length) => length
            .to_px()
            .map(|value| value as f32)
            .ok_or_else(|| StyleError::UnsupportedValue(format!("{value:?}"))),
        _ => Err(StyleError::UnsupportedValue(format!("{value:?}"))),
    }
}

fn border_width_to_px(value: &CssBorderSideWidth) -> Result<f32, StyleError> {
    match value {
        CssBorderSideWidth::Thin => Ok(1.0),
        CssBorderSideWidth::Medium => Ok(3.0),
        CssBorderSideWidth::Thick => Ok(5.0),
        CssBorderSideWidth::Length(length) => length
            .to_px()
            .map(|value| value as f32)
            .ok_or_else(|| StyleError::UnsupportedValue(format!("{value:?}"))),
    }
}

fn border_width_declaration(side: BorderSide, value: f32) -> Declaration {
    match side {
        BorderSide::Top => Declaration::BorderTopWidth(value),
        BorderSide::Right => Declaration::BorderRightWidth(value),
        BorderSide::Bottom => Declaration::BorderBottomWidth(value),
        BorderSide::Left => Declaration::BorderLeftWidth(value),
    }
}
