use cssimpler_core::{Style, SvgPaint};
use lightningcss::properties::svg::SVGPaint as CssSvgPaint;
use lightningcss::values::color::CssColor;
use lightningcss::values::length::LengthPercentage;

use crate::{Declaration, StyleError};

use super::color;

pub(super) fn fill_declaration(paint: &CssSvgPaint<'_>) -> Result<Vec<Declaration>, StyleError> {
    Ok(vec![Declaration::SvgFill(svg_paint_from_css(paint)?)])
}

pub(super) fn stroke_declaration(paint: &CssSvgPaint<'_>) -> Result<Vec<Declaration>, StyleError> {
    Ok(vec![Declaration::SvgStroke(svg_paint_from_css(paint)?)])
}

pub(super) fn stroke_width_declaration(
    value: &LengthPercentage,
) -> Result<Vec<Declaration>, StyleError> {
    Ok(vec![Declaration::SvgStrokeWidth(stroke_width_from_css(
        value,
    )?)])
}

pub(super) fn apply_svg_fill(style: &mut Style, paint: SvgPaint) {
    style.visual.svg.fill = Some(paint);
}

pub(super) fn apply_svg_stroke(style: &mut Style, paint: SvgPaint) {
    style.visual.svg.stroke = Some(paint);
}

pub(super) fn apply_svg_stroke_width(style: &mut Style, width: f32) {
    style.visual.svg.stroke_width = Some(width.max(0.0));
}

fn svg_paint_from_css(paint: &CssSvgPaint<'_>) -> Result<SvgPaint, StyleError> {
    match paint {
        CssSvgPaint::Color(CssColor::CurrentColor) => Ok(SvgPaint::CurrentColor),
        CssSvgPaint::Color(color) => Ok(SvgPaint::Color(color::color_from_css(color)?)),
        CssSvgPaint::None => Ok(SvgPaint::None),
        CssSvgPaint::Url { .. } | CssSvgPaint::ContextFill | CssSvgPaint::ContextStroke => {
            Err(StyleError::UnsupportedValue(format!("{paint:?}")))
        }
    }
}

fn stroke_width_from_css(value: &LengthPercentage) -> Result<f32, StyleError> {
    match value {
        LengthPercentage::Dimension(length) => length
            .to_px()
            .map(|value| value as f32)
            .ok_or_else(|| StyleError::UnsupportedValue(format!("{value:?}"))),
        _ => Err(StyleError::UnsupportedValue(format!("{value:?}"))),
    }
}
