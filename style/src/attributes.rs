use lightningcss::printer::PrinterOptions;
use lightningcss::properties::Property;
use lightningcss::properties::custom::{Function, TokenList, TokenOrValue};
use lightningcss::traits::ToCss;

use crate::{ElementRef, StyleError};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttributeTextSource {
    Literal(String),
    Attribute(String),
}

impl AttributeTextSource {
    pub fn resolve(&self, element: ElementRef<'_>) -> String {
        match self {
            Self::Literal(value) => value.clone(),
            Self::Attribute(name) => element.attribute(name).unwrap_or_default().to_string(),
        }
    }
}

pub fn parse_attribute_text_source(
    tokens: &TokenList<'_>,
) -> Result<AttributeTextSource, StyleError> {
    let tokens = non_whitespace_tokens(tokens);
    let [token] = tokens.as_slice() else {
        return Err(unsupported_attribute_text_source(&tokens));
    };

    match token {
        TokenOrValue::Token(token) => {
            let serialized = token
                .to_css_string(PrinterOptions::default())
                .map_err(|_| unsupported_attribute_text_source(&tokens))?;
            let Some(literal) = serialized
                .strip_prefix('"')
                .and_then(|value| value.strip_suffix('"'))
                .or_else(|| {
                    serialized
                        .strip_prefix('\'')
                        .and_then(|value| value.strip_suffix('\''))
                })
            else {
                return Err(unsupported_attribute_text_source(&tokens));
            };

            Ok(AttributeTextSource::Literal(literal.to_string()))
        }
        TokenOrValue::Function(function) => parse_attr_function(function),
        _ => Err(unsupported_attribute_text_source(&tokens)),
    }
}

pub fn reject_unsupported_attr_usage(property: &Property<'_>) -> Result<(), StyleError> {
    let property_name = match property {
        Property::Unparsed(unparsed) => Some((unparsed.property_id.name(), &unparsed.value)),
        Property::Custom(custom) if custom.name.as_ref() == "content" => {
            Some((custom.name.as_ref(), &custom.value))
        }
        _ => None,
    };

    let Some((property_name, tokens)) = property_name else {
        return Ok(());
    };

    if contains_attr_function(tokens) {
        return Err(StyleError::UnsupportedValue(format!(
            "attr() is not supported for `{property_name}` yet"
        )));
    }

    Ok(())
}

fn parse_attr_function(function: &Function<'_>) -> Result<AttributeTextSource, StyleError> {
    if function.name.as_ref() != "attr" {
        return Err(unsupported_attribute_text_source(&non_whitespace_tokens(
            &function.arguments,
        )));
    }

    let arguments = non_whitespace_tokens(&function.arguments);
    let [token] = arguments.as_slice() else {
        return Err(unsupported_attribute_text_source(&arguments));
    };
    let TokenOrValue::Token(token) = token else {
        return Err(unsupported_attribute_text_source(&arguments));
    };
    let attribute_name = token
        .to_css_string(PrinterOptions::default())
        .map_err(|_| unsupported_attribute_text_source(&arguments))?;

    if !is_supported_attribute_name(&attribute_name) {
        return Err(unsupported_attribute_text_source(&arguments));
    }

    Ok(AttributeTextSource::Attribute(attribute_name))
}

fn non_whitespace_tokens<'a, 'i>(tokens: &'a TokenList<'i>) -> Vec<&'a TokenOrValue<'i>> {
    tokens
        .0
        .iter()
        .filter(|token| !token.is_whitespace())
        .collect()
}

fn contains_attr_function(tokens: &TokenList<'_>) -> bool {
    tokens.0.iter().any(token_contains_attr_function)
}

fn token_contains_attr_function(token: &TokenOrValue<'_>) -> bool {
    match token {
        TokenOrValue::Function(function) => {
            function.name.as_ref() == "attr" || contains_attr_function(&function.arguments)
        }
        TokenOrValue::Var(variable) => variable
            .fallback
            .as_ref()
            .is_some_and(contains_attr_function),
        TokenOrValue::Env(environment) => environment
            .fallback
            .as_ref()
            .is_some_and(contains_attr_function),
        _ => false,
    }
}

fn unsupported_attribute_text_source(tokens: &[&TokenOrValue<'_>]) -> StyleError {
    StyleError::UnsupportedValue(format!(
        "unsupported attribute text source: {}",
        serialize_tokens(tokens)
    ))
}

fn serialize_tokens(tokens: &[&TokenOrValue<'_>]) -> String {
    format!("{tokens:?}")
}

fn is_supported_attribute_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | ':')
        })
}

#[cfg(test)]
mod tests {
    use lightningcss::properties::{Property, PropertyId};
    use lightningcss::stylesheet::ParserOptions;

    use cssimpler_core::Node;

    use crate::parse_stylesheet;

    use super::{AttributeTextSource, parse_attribute_text_source};

    #[test]
    fn attribute_text_sources_support_literals_and_attr_lookups() {
        let literal = Property::parse_string(
            PropertyId::from("content"),
            "\"hello\"",
            ParserOptions::default(),
        )
        .expect("content should parse");
        let Property::Custom(literal) = literal else {
            panic!("expected custom content property");
        };

        assert_eq!(
            parse_attribute_text_source(&literal.value).expect("literal content should parse"),
            AttributeTextSource::Literal("hello".to_string())
        );

        let attribute = Property::parse_string(
            PropertyId::from("content"),
            "attr(data-text)",
            ParserOptions::default(),
        )
        .expect("content should parse");
        let Property::Custom(attribute) = attribute else {
            panic!("expected custom content property");
        };

        let source =
            parse_attribute_text_source(&attribute.value).expect("attr() content should parse");
        let element = Node::element("div").with_attribute("data-text", "uiverse");

        assert_eq!(
            source,
            AttributeTextSource::Attribute("data-text".to_string())
        );
        assert_eq!(source.resolve((&element).into()), "uiverse");
    }

    #[test]
    fn attribute_text_sources_reject_browser_only_attr_shapes() {
        let property = Property::parse_string(
            PropertyId::from("content"),
            "attr(data-text string)",
            ParserOptions::default(),
        )
        .expect("content should parse");
        let Property::Custom(property) = property else {
            panic!("expected custom content property");
        };
        let error =
            parse_attribute_text_source(&property.value).expect_err("typed attr() should fail");

        assert!(matches!(
            error,
            crate::StyleError::UnsupportedValue(message)
                if message.contains("unsupported attribute text source")
        ));
    }

    #[test]
    fn parse_stylesheet_rejects_attr_in_unsupported_declaration_contexts() {
        let error = parse_stylesheet(".card { color: attr(data-text); }")
            .expect_err("attr() outside supported contexts should fail");

        assert!(matches!(
            error,
            crate::StyleError::UnsupportedValue(message)
                if message.contains("attr() is not supported for `color` yet")
        ));
    }
}
