use cssimpler_core::{Color, NativeMaterial, Style};
use lightningcss::properties::custom::CustomProperty;
use lightningcss::properties::{Property, PropertyId};
use lightningcss::stylesheet::{ParserOptions, PrinterOptions};

use crate::{Declaration, StyleError};

use super::color;

pub(super) fn custom_property_declarations(
    property: &Property<'_>,
) -> Option<Result<Vec<Declaration>, StyleError>> {
    let Property::Custom(custom) = property else {
        return None;
    };

    if custom.name.as_ref().eq_ignore_ascii_case("native-material") {
        return Some(native_material_declaration(custom));
    }

    if custom.name.as_ref().eq_ignore_ascii_case("glass-tint") {
        return Some(glass_tint_declaration(custom));
    }

    None
}

pub(super) fn apply_native_material(style: &mut Style, material: NativeMaterial) {
    style.visual.native_material = material;
}

pub(super) fn apply_glass_tint(style: &mut Style, tint: Option<Color>) {
    style.visual.glass_tint = tint;
}

fn native_material_declaration(
    custom: &CustomProperty<'_>,
) -> Result<Vec<Declaration>, StyleError> {
    let value = custom_value_to_css(custom)?;
    let material = match value.trim() {
        value if value.eq_ignore_ascii_case("none") => NativeMaterial::None,
        value if value.eq_ignore_ascii_case("glass") => NativeMaterial::Glass,
        value => return Err(StyleError::UnsupportedValue(value.to_string())),
    };

    Ok(vec![Declaration::NativeMaterial(material)])
}

fn glass_tint_declaration(custom: &CustomProperty<'_>) -> Result<Vec<Declaration>, StyleError> {
    let value = custom_value_to_css(custom)?;
    let value = value.trim();
    if value.eq_ignore_ascii_case("none") || value.eq_ignore_ascii_case("auto") {
        return Ok(vec![Declaration::GlassTint(None)]);
    }

    Ok(vec![Declaration::GlassTint(Some(parse_css_color(value)?))])
}

fn custom_value_to_css(custom: &CustomProperty<'_>) -> Result<String, StyleError> {
    Property::Custom(custom.clone())
        .value_to_css_string(PrinterOptions::default())
        .map_err(|error| StyleError::UnsupportedValue(error.to_string()))
}

fn parse_css_color(value: &str) -> Result<Color, StyleError> {
    let property =
        Property::parse_string(PropertyId::from("color"), value, ParserOptions::default())
            .map_err(|_| StyleError::UnsupportedValue(value.to_string()))?;

    match property {
        Property::Color(color_value) => color::color_from_css(&color_value),
        _ => Err(StyleError::UnsupportedValue(value.to_string())),
    }
}
