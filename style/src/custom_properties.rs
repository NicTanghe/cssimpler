use cssimpler_core::{CustomProperties, Style};
use lightningcss::printer::PrinterOptions;
use lightningcss::properties::Property;

use crate::{Declaration, StyleError};

pub(crate) fn extract_property(
    property: &Property<'_>,
) -> Option<Result<Vec<Declaration>, StyleError>> {
    let Property::Custom(custom) = property else {
        return None;
    };

    let name = custom.name.as_ref();
    if !name.starts_with("--") {
        return None;
    }

    Some(custom_property_declaration(property, name))
}

pub(crate) fn apply_declaration(style: &mut Style, declaration: &Declaration) -> bool {
    let Declaration::CustomProperty { name, value } = declaration else {
        return false;
    };

    style.custom_properties.set(name.clone(), value.clone());
    true
}

pub(crate) fn inherit(style: &mut Style, inherited: &CustomProperties) {
    style.custom_properties.inherit_from(inherited);
}

fn custom_property_declaration(
    property: &Property<'_>,
    name: &str,
) -> Result<Vec<Declaration>, StyleError> {
    let value = property
        .value_to_css_string(PrinterOptions::default())
        .map_err(|error| StyleError::UnsupportedValue(error.to_string()))?;

    Ok(vec![Declaration::CustomProperty {
        name: name.to_string(),
        value,
    }])
}

#[cfg(test)]
mod tests {
    use cssimpler_core::Style;
    use lightningcss::properties::{Property, PropertyId};
    use lightningcss::stylesheet::ParserOptions;

    use crate::{Declaration, parse_stylesheet};

    use super::{apply_declaration, extract_property, inherit};

    #[test]
    fn parser_stores_author_defined_custom_properties() {
        let stylesheet = parse_stylesheet(".button { --animation-color: #2563eb; }")
            .expect("custom property stylesheet should parse");

        assert!(
            stylesheet.rules[0]
                .declarations
                .contains(&Declaration::CustomProperty {
                    name: "--animation-color".to_string(),
                    value: "#2563eb".to_string(),
                })
        );
    }

    #[test]
    fn extractor_ignores_unknown_non_custom_properties() {
        let property = Property::parse_string(
            PropertyId::from("scrollbar-width"),
            "thin",
            ParserOptions::default(),
        )
        .expect("unknown property should parse");

        assert!(extract_property(&property).is_none());
    }

    #[test]
    fn custom_property_declarations_store_values_on_resolved_styles() {
        let declaration = Declaration::CustomProperty {
            name: "--animation-color".to_string(),
            value: "#2563eb".to_string(),
        };
        let mut style = Style::default();

        assert!(apply_declaration(&mut style, &declaration));
        assert_eq!(
            style.custom_properties.get("--animation-color"),
            Some("#2563eb")
        );
    }

    #[test]
    fn inherited_custom_properties_fill_in_missing_local_values() {
        let mut inherited_style = Style::default();
        inherited_style
            .custom_properties
            .set("--animation-color", "#2563eb");
        inherited_style
            .custom_properties
            .set("--animation-speed", "180ms");

        let mut local_style = Style::default();
        local_style
            .custom_properties
            .set("--animation-speed", "240ms");

        inherit(&mut local_style, &inherited_style.custom_properties);

        assert_eq!(
            local_style.custom_properties.get("--animation-color"),
            Some("#2563eb")
        );
        assert_eq!(
            local_style.custom_properties.get("--animation-speed"),
            Some("240ms")
        );
    }
}
