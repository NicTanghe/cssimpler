use cssimpler_core::{BoxShadow as CoreBoxShadow, Color, ShadowEffect as CoreShadowEffect, Style};
use lightningcss::printer::PrinterOptions;
use lightningcss::properties::Property;
use lightningcss::properties::PropertyId;
use lightningcss::properties::box_shadow::BoxShadow;
use lightningcss::properties::custom::{TokenList, TokenOrValue};
use lightningcss::properties::effects::{DropShadow, Filter, FilterList};
use lightningcss::properties::text::{Spacing, TextShadow};
use lightningcss::stylesheet::ParserOptions;
use lightningcss::traits::ToCss;
use lightningcss::values::length::Length;

use crate::{Declaration, StyleError};

use super::color;

#[derive(Clone, Debug, PartialEq)]
pub struct ShadowDeclaration {
    pub(crate) color: Option<Color>,
    pub(crate) offset_x: f32,
    pub(crate) offset_y: f32,
    pub(crate) blur_radius: f32,
    pub(crate) spread: f32,
}

pub(super) fn extract_unparsed_property(
    property: &Property<'_>,
) -> Option<Result<Vec<Declaration>, StyleError>> {
    match property {
        Property::Unparsed(unparsed) => match unparsed.property_id.name() {
            "-webkit-text-stroke" => Some(text_stroke_shorthand_declarations(&unparsed.value)),
            "-webkit-text-stroke-width" => Some(text_stroke_width_declaration(&unparsed.value)),
            "-webkit-text-stroke-color" => Some(text_stroke_color_declaration(&unparsed.value)),
            _ => None,
        },
        Property::Custom(custom) => match custom.name.as_ref() {
            "-webkit-text-stroke" => Some(text_stroke_shorthand_declarations(&custom.value)),
            "-webkit-text-stroke-width" => Some(text_stroke_width_declaration(&custom.value)),
            "-webkit-text-stroke-color" => Some(text_stroke_color_declaration(&custom.value)),
            _ => None,
        },
        _ => None,
    }
}

pub(super) fn box_shadow_declarations(
    shadows: &[BoxShadow],
) -> Result<Vec<Declaration>, StyleError> {
    Ok(vec![Declaration::BoxShadows(
        shadows
            .iter()
            .filter(|shadow| !shadow.inset)
            .map(box_shadow_declaration)
            .collect::<Result<Vec<_>, _>>()?,
    )])
}

pub(super) fn text_shadow_declarations(
    shadows: &[TextShadow],
) -> Result<Vec<Declaration>, StyleError> {
    Ok(vec![Declaration::TextShadows(
        shadows
            .iter()
            .map(text_shadow_declaration)
            .collect::<Result<Vec<_>, _>>()?,
    )])
}

pub(super) fn filter_drop_shadow_declarations(
    filters: &FilterList<'_>,
) -> Result<Vec<Declaration>, StyleError> {
    match filters {
        FilterList::None => Ok(vec![Declaration::FilterDropShadows(Vec::new())]),
        FilterList::Filters(filters) => Ok(vec![Declaration::FilterDropShadows(
            filters
                .iter()
                .map(|filter| match filter {
                    Filter::DropShadow(shadow) => drop_shadow_declaration(shadow),
                    _ => Err(unsupported_filter_value(filters)),
                })
                .collect::<Result<Vec<_>, _>>()?,
        )]),
    }
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

pub(super) fn apply_text_shadows(style: &mut Style, shadows: &[ShadowDeclaration]) {
    style.visual.text_shadows = shadow_effects(shadows);
}

pub(super) fn apply_filter_drop_shadows(style: &mut Style, shadows: &[ShadowDeclaration]) {
    style.visual.filter_drop_shadows = shadow_effects(shadows);
}

pub(super) fn apply_text_stroke_width(style: &mut Style, width: f32) {
    style.visual.text_stroke.width = width.max(0.0);
}

pub(super) fn apply_text_stroke_color(style: &mut Style, color: Option<Color>) {
    style.visual.text_stroke.color = color;
}

fn shadow_effects(shadows: &[ShadowDeclaration]) -> Vec<CoreShadowEffect> {
    shadows
        .iter()
        .map(|shadow| CoreShadowEffect {
            color: shadow.color,
            offset_x: shadow.offset_x,
            offset_y: shadow.offset_y,
            blur_radius: shadow.blur_radius,
            spread: shadow.spread,
        })
        .collect()
}

fn box_shadow_declaration(shadow: &BoxShadow) -> Result<ShadowDeclaration, StyleError> {
    Ok(ShadowDeclaration {
        color: color::color_from_css_optional(&shadow.color)?,
        offset_x: length_to_px(&shadow.x_offset)?,
        offset_y: length_to_px(&shadow.y_offset)?,
        blur_radius: length_to_px(&shadow.blur)?,
        spread: length_to_px(&shadow.spread)?,
    })
}

fn text_shadow_declaration(shadow: &TextShadow) -> Result<ShadowDeclaration, StyleError> {
    Ok(ShadowDeclaration {
        color: color::color_from_css_optional(&shadow.color)?,
        offset_x: length_to_px(&shadow.x_offset)?,
        offset_y: length_to_px(&shadow.y_offset)?,
        blur_radius: length_to_px(&shadow.blur)?,
        spread: length_to_px(&shadow.spread)?,
    })
}

fn drop_shadow_declaration(shadow: &DropShadow) -> Result<ShadowDeclaration, StyleError> {
    Ok(ShadowDeclaration {
        color: color::color_from_css_optional(&shadow.color)?,
        offset_x: length_to_px(&shadow.x_offset)?,
        offset_y: length_to_px(&shadow.y_offset)?,
        blur_radius: length_to_px(&shadow.blur)?,
        spread: 0.0,
    })
}

fn text_stroke_shorthand_declarations(
    tokens: &TokenList<'_>,
) -> Result<Vec<Declaration>, StyleError> {
    let tokens = non_whitespace_tokens(tokens);
    let mut width = None;
    let mut color = None;

    for token in &tokens {
        if width.is_none() {
            if let Ok(value) = text_stroke_width_from_token(token) {
                width = Some(value);
                continue;
            }
        }

        if color.is_none() {
            if let Ok(value) = text_stroke_color_from_token(token) {
                color = Some(value);
                continue;
            }
        }

        return Err(unsupported_text_stroke_value(
            "-webkit-text-stroke",
            tokens.as_slice(),
        ));
    }

    let width = width
        .ok_or_else(|| unsupported_text_stroke_value("-webkit-text-stroke", tokens.as_slice()))?;
    let mut declarations = vec![Declaration::TextStrokeWidth(width)];
    if let Some(color) = color {
        declarations.push(Declaration::TextStrokeColor(color));
    }

    Ok(declarations)
}

fn text_stroke_width_declaration(tokens: &TokenList<'_>) -> Result<Vec<Declaration>, StyleError> {
    let tokens = non_whitespace_tokens(tokens);
    let [token] = tokens.as_slice() else {
        return Err(unsupported_text_stroke_value(
            "-webkit-text-stroke-width",
            tokens.as_slice(),
        ));
    };

    Ok(vec![Declaration::TextStrokeWidth(
        text_stroke_width_from_token(token)?,
    )])
}

fn text_stroke_color_declaration(tokens: &TokenList<'_>) -> Result<Vec<Declaration>, StyleError> {
    let tokens = non_whitespace_tokens(tokens);
    let [token] = tokens.as_slice() else {
        return Err(unsupported_text_stroke_value(
            "-webkit-text-stroke-color",
            tokens.as_slice(),
        ));
    };

    Ok(vec![Declaration::TextStrokeColor(
        text_stroke_color_from_token(token)?,
    )])
}

fn text_stroke_width_from_token(token: &TokenOrValue<'_>) -> Result<f32, StyleError> {
    let token_css = token_to_css(token)?;
    let property = Property::parse_string(
        PropertyId::from("letter-spacing"),
        &token_css,
        ParserOptions::default(),
    )
    .map_err(|_| unsupported_text_stroke_token(token))?;
    let Property::LetterSpacing(value) = property else {
        return Err(unsupported_text_stroke_token(token));
    };
    let Spacing::Length(length) = value else {
        return Err(unsupported_text_stroke_token(token));
    };

    length_to_px(&length)
}

fn text_stroke_color_from_token(token: &TokenOrValue<'_>) -> Result<Option<Color>, StyleError> {
    let token_css = token_to_css(token)?;
    let property = Property::parse_string(
        PropertyId::from("color"),
        &token_css,
        ParserOptions::default(),
    )
    .map_err(|_| unsupported_text_stroke_token(token))?;
    let Property::Color(color) = property else {
        return Err(unsupported_text_stroke_token(token));
    };

    color::color_from_css_optional(&color)
}

fn non_whitespace_tokens<'a, 'i>(tokens: &'a TokenList<'i>) -> Vec<&'a TokenOrValue<'i>> {
    tokens
        .0
        .iter()
        .filter(|token| !token.is_whitespace())
        .collect()
}

fn token_to_css(token: &TokenOrValue<'_>) -> Result<String, StyleError> {
    match token {
        TokenOrValue::Token(token) => token
            .to_css_string(PrinterOptions::default())
            .map_err(|error| StyleError::UnsupportedValue(error.to_string())),
        TokenOrValue::Color(color) => color
            .to_css_string(PrinterOptions::default())
            .map_err(|error| StyleError::UnsupportedValue(error.to_string())),
        TokenOrValue::Url(url) => url
            .to_css_string(PrinterOptions::default())
            .map_err(|error| StyleError::UnsupportedValue(error.to_string())),
        TokenOrValue::Length(length) => length
            .to_css_string(PrinterOptions::default())
            .map_err(|error| StyleError::UnsupportedValue(error.to_string())),
        TokenOrValue::Angle(angle) => angle
            .to_css_string(PrinterOptions::default())
            .map_err(|error| StyleError::UnsupportedValue(error.to_string())),
        TokenOrValue::Time(time) => time
            .to_css_string(PrinterOptions::default())
            .map_err(|error| StyleError::UnsupportedValue(error.to_string())),
        TokenOrValue::Resolution(resolution) => resolution
            .to_css_string(PrinterOptions::default())
            .map_err(|error| StyleError::UnsupportedValue(error.to_string())),
        TokenOrValue::DashedIdent(ident) => ident
            .to_css_string(PrinterOptions::default())
            .map_err(|error| StyleError::UnsupportedValue(error.to_string())),
        TokenOrValue::AnimationName(name) => name
            .to_css_string(PrinterOptions::default())
            .map_err(|error| StyleError::UnsupportedValue(error.to_string())),
        TokenOrValue::Function(function) => Ok(format!(
            "{}({})",
            function.name.as_ref(),
            token_list_to_css(&function.arguments)?
        )),
        _ => Err(StyleError::UnsupportedValue(format!("{token:?}"))),
    }
}

fn token_list_to_css(tokens: &TokenList<'_>) -> Result<String, StyleError> {
    let mut css = String::new();
    for token in &tokens.0 {
        css.push_str(&token_to_css(token)?);
    }
    Ok(css)
}

fn unsupported_filter_value(filters: &[Filter<'_>]) -> StyleError {
    StyleError::UnsupportedValue(format!(
        "unsupported filter value: {} (only drop-shadow() is supported)",
        serialize_filters(filters)
    ))
}

fn serialize_filters(filters: &[Filter<'_>]) -> String {
    filters
        .iter()
        .map(|filter| {
            filter
                .to_css_string(PrinterOptions::default())
                .unwrap_or_else(|_| format!("{filter:?}"))
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn unsupported_text_stroke_token(token: &TokenOrValue<'_>) -> StyleError {
    StyleError::UnsupportedValue(format!(
        "unsupported text stroke token: {}",
        token_to_css(token).unwrap_or_else(|_| format!("{token:?}"))
    ))
}

fn unsupported_text_stroke_value(property_name: &str, tokens: &[&TokenOrValue<'_>]) -> StyleError {
    StyleError::UnsupportedValue(format!(
        "unsupported `{property_name}` value: {}",
        serialize_tokens(tokens)
    ))
}

fn serialize_tokens(tokens: &[&TokenOrValue<'_>]) -> String {
    tokens
        .iter()
        .map(|token| token_to_css(token).unwrap_or_else(|_| format!("{token:?}")))
        .collect::<Vec<_>>()
        .join(" ")
}

fn length_to_px(value: &Length) -> Result<f32, StyleError> {
    value
        .to_px()
        .map(|value| value as f32)
        .ok_or_else(|| StyleError::UnsupportedValue(format!("{value:?}")))
}
