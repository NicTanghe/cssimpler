use cssimpler_core::{BackdropOcclusion, Style};
use lightningcss::printer::PrinterOptions;
use lightningcss::properties::Property;
use lightningcss::properties::custom::{CustomProperty, TokenList, TokenOrValue};
use lightningcss::properties::effects::{Filter, FilterList};
use lightningcss::traits::ToCss;
use lightningcss::values::length::Length;

use crate::{Declaration, StyleError};

pub(super) fn custom_property_declarations(
    property: &Property<'_>,
) -> Option<Result<Vec<Declaration>, StyleError>> {
    match property {
        Property::Unparsed(unparsed)
            if unparsed
                .property_id
                .name()
                .eq_ignore_ascii_case("backdrop-occlude") =>
        {
            Some(backdrop_occlusion_declaration(&unparsed.value))
        }
        Property::Custom(custom)
            if custom
                .name
                .as_ref()
                .eq_ignore_ascii_case("backdrop-occlude") =>
        {
            Some(backdrop_occlusion_custom_declaration(custom))
        }
        _ => None,
    }
}

pub(super) fn backdrop_filter_declarations(
    filters: &FilterList<'_>,
) -> Result<Vec<Declaration>, StyleError> {
    match filters {
        FilterList::None => Ok(vec![Declaration::BackdropBlur(0.0)]),
        FilterList::Filters(filters) => {
            let [filter] = filters.as_slice() else {
                return Err(unsupported_backdrop_filter_value(filters));
            };
            let Filter::Blur(radius) = filter else {
                return Err(unsupported_backdrop_filter_value(filters));
            };
            Ok(vec![Declaration::BackdropBlur(
                length_to_px(radius)?.max(0.0),
            )])
        }
    }
}

pub(super) fn apply_backdrop_blur(style: &mut Style, radius: f32) {
    style.visual.backdrop_blur_radius = radius.max(0.0);
}

pub(super) fn apply_backdrop_occlusion(style: &mut Style, occlusion: BackdropOcclusion) {
    style.visual.backdrop_occlusion = occlusion;
}

fn backdrop_occlusion_custom_declaration(
    custom: &CustomProperty<'_>,
) -> Result<Vec<Declaration>, StyleError> {
    backdrop_occlusion_declaration(&custom.value)
}

fn backdrop_occlusion_declaration(tokens: &TokenList<'_>) -> Result<Vec<Declaration>, StyleError> {
    let tokens = non_whitespace_tokens(tokens);
    let [token] = tokens.as_slice() else {
        return Err(unsupported_backdrop_occlude_value(tokens.as_slice()));
    };
    let value = token_to_css(token)?;
    let occlusion = match value.trim() {
        value if value.eq_ignore_ascii_case("false") => BackdropOcclusion::None,
        value if value.eq_ignore_ascii_case("scene") => BackdropOcclusion::Scene,
        _ => return Err(unsupported_backdrop_occlude_value(tokens.as_slice())),
    };

    Ok(vec![Declaration::BackdropOcclusion(occlusion)])
}

fn length_to_px(value: &Length) -> Result<f32, StyleError> {
    value
        .to_px()
        .map(|value| value as f32)
        .ok_or_else(|| StyleError::UnsupportedValue(format!("{value:?}")))
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
        TokenOrValue::DashedIdent(ident) => ident
            .to_css_string(PrinterOptions::default())
            .map_err(|error| StyleError::UnsupportedValue(error.to_string())),
        _ => Err(StyleError::UnsupportedValue(format!("{token:?}"))),
    }
}

fn unsupported_backdrop_occlude_value(tokens: &[&TokenOrValue<'_>]) -> StyleError {
    StyleError::UnsupportedValue(format!(
        "unsupported backdrop-occlude value: {} (only false and scene are supported)",
        tokens
            .iter()
            .map(|token| token_to_css(token).unwrap_or_else(|_| format!("{token:?}")))
            .collect::<Vec<_>>()
            .join(" ")
    ))
}

fn unsupported_backdrop_filter_value(filters: &[Filter<'_>]) -> StyleError {
    StyleError::UnsupportedValue(format!(
        "unsupported backdrop-filter value: {} (only blur() is supported)",
        filters
            .iter()
            .map(|filter| {
                filter
                    .to_css_string(PrinterOptions::default())
                    .unwrap_or_else(|_| format!("{filter:?}"))
            })
            .collect::<Vec<_>>()
            .join(" ")
    ))
}
