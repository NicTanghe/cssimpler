use std::collections::HashMap;

use cssimpler_core::CustomProperties;
use lightningcss::printer::PrinterOptions;
use lightningcss::properties::custom::{CustomProperty, TokenList, TokenOrValue, UnparsedProperty};
use lightningcss::properties::{Property, PropertyId};
use lightningcss::stylesheet::ParserOptions;

use crate::{Declaration, StyleError};

pub(crate) fn extract_property(
    property: &Property<'_>,
) -> Option<Result<Vec<Declaration>, StyleError>> {
    match property {
        Property::Unparsed(unparsed) if contains_var_reference(&unparsed.value) => {
            Some(variable_dependent_property_declaration(
                unparsed.property_id.name(),
                property_value_to_css(property),
            ))
        }
        Property::Custom(custom)
            if !custom.name.as_ref().starts_with("--") && contains_var_reference(&custom.value) =>
        {
            Some(variable_dependent_property_declaration(
                custom.name.as_ref(),
                property_value_to_css(property),
            ))
        }
        _ => None,
    }
}

pub(crate) fn resolve_declaration(
    declaration: &Declaration,
    custom_properties: &CustomProperties,
) -> Option<Result<Vec<Declaration>, StyleError>> {
    let Declaration::VariableDependentProperty {
        property_name,
        value_css,
    } = declaration
    else {
        return None;
    };

    Some(resolve_variable_dependent_property(
        property_name,
        value_css,
        custom_properties,
    ))
}

fn resolve_variable_dependent_property(
    property_name: &str,
    value_css: &str,
    custom_properties: &CustomProperties,
) -> Result<Vec<Declaration>, StyleError> {
    let property = Property::parse_string(
        PropertyId::from(property_name),
        value_css,
        ParserOptions::default(),
    )
    .map_err(|_| {
        unresolved_property_error(
            property_name,
            value_css,
            "the declaration could not be reparsed before substitution",
        )
    })?;
    let variables = custom_property_tokens(custom_properties)?;
    let resolved_value =
        resolve_property_value_css(property_name, value_css, property, &variables)?;
    let resolved_property = Property::parse_string(
        PropertyId::from(property_name),
        &resolved_value,
        ParserOptions::default(),
    )
    .map_err(|_| {
        unresolved_property_error(
            property_name,
            value_css,
            "the substituted declaration could not be reparsed",
        )
    })?;

    if property_contains_var_reference(&resolved_property) {
        return Err(unresolved_property_error(
            property_name,
            value_css,
            "a referenced custom property was missing or invalid",
        ));
    }

    let declarations = super::extract_property(&resolved_property)?;

    if declarations.is_empty() {
        return Err(unresolved_property_error(
            property_name,
            value_css,
            "resolved declaration is not supported yet",
        ));
    }

    if declarations
        .iter()
        .any(|declaration| matches!(declaration, Declaration::VariableDependentProperty { .. }))
    {
        return Err(unresolved_property_error(
            property_name,
            value_css,
            "a referenced custom property was missing or invalid",
        ));
    }

    Ok(declarations)
}

fn custom_property_tokens<'a>(
    custom_properties: &'a CustomProperties,
) -> Result<HashMap<&'a str, TokenList<'a>>, StyleError> {
    let mut variables = HashMap::new();

    for (name, value) in custom_properties.iter() {
        let property =
            Property::parse_string(PropertyId::from(name), value, ParserOptions::default())
                .map_err(|_| invalid_custom_property_value(name, value))?;
        let Property::Custom(custom) = property else {
            return Err(invalid_custom_property_value(name, value));
        };
        variables.insert(name, custom.value);
    }

    Ok(variables)
}

fn resolve_property_value_css<'a>(
    property_name: &str,
    value_css: &str,
    property: Property<'a>,
    variables: &HashMap<&'a str, TokenList<'a>>,
) -> Result<String, StyleError> {
    match property {
        Property::Unparsed(unparsed) => {
            let resolved_tokens = substitute_token_list(
                &unparsed.value,
                variables,
                &mut Vec::new(),
                property_name,
                value_css,
            )?;
            Property::Unparsed(UnparsedProperty {
                property_id: unparsed.property_id,
                value: resolved_tokens,
            })
            .value_to_css_string(PrinterOptions::default())
            .map_err(|error| {
                unresolved_property_error(
                    property_name,
                    value_css,
                    &format!("the substituted value could not be serialized: {error}"),
                )
            })
        }
        Property::Custom(custom) if !custom.name.as_ref().starts_with("--") => {
            let resolved_tokens = substitute_token_list(
                &custom.value,
                variables,
                &mut Vec::new(),
                property_name,
                value_css,
            )?;
            Property::Custom(CustomProperty {
                name: custom.name,
                value: resolved_tokens,
            })
            .value_to_css_string(PrinterOptions::default())
            .map_err(|error| {
                unresolved_property_error(
                    property_name,
                    value_css,
                    &format!("the substituted value could not be serialized: {error}"),
                )
            })
        }
        other => property_value_to_css(&other),
    }
}

fn property_value_to_css(property: &Property<'_>) -> Result<String, StyleError> {
    property
        .value_to_css_string(PrinterOptions::default())
        .map_err(|error| StyleError::UnsupportedValue(error.to_string()))
}

fn variable_dependent_property_declaration(
    property_name: &str,
    value_css: Result<String, StyleError>,
) -> Result<Vec<Declaration>, StyleError> {
    Ok(vec![Declaration::VariableDependentProperty {
        property_name: property_name.to_string(),
        value_css: value_css?,
    }])
}

fn property_contains_var_reference(property: &Property<'_>) -> bool {
    match property {
        Property::Unparsed(unparsed) => contains_var_reference(&unparsed.value),
        Property::Custom(custom) => contains_var_reference(&custom.value),
        _ => false,
    }
}

fn contains_var_reference(tokens: &TokenList<'_>) -> bool {
    tokens.0.iter().any(token_contains_var_reference)
}

fn token_contains_var_reference(token: &TokenOrValue<'_>) -> bool {
    match token {
        TokenOrValue::Var(_) => true,
        TokenOrValue::Function(function) => contains_var_reference(&function.arguments),
        TokenOrValue::Env(environment) => environment
            .fallback
            .as_ref()
            .is_some_and(contains_var_reference),
        _ => false,
    }
}

fn substitute_token_list<'a>(
    tokens: &'a TokenList<'a>,
    variables: &'a HashMap<&'a str, TokenList<'a>>,
    stack: &mut Vec<&'a str>,
    property_name: &str,
    value_css: &str,
) -> Result<TokenList<'a>, StyleError> {
    let mut resolved = Vec::with_capacity(tokens.0.len());

    for token in &tokens.0 {
        match token {
            TokenOrValue::Var(variable) => {
                let name = variable.name.ident.0.as_ref();

                if stack.contains(&name) {
                    return Err(unresolved_property_error(
                        property_name,
                        value_css,
                        "a circular custom property reference was detected",
                    ));
                }

                if let Some(value) = variables.get(name) {
                    stack.push(name);
                    let substituted =
                        substitute_token_list(value, variables, stack, property_name, value_css)?;
                    stack.pop();
                    resolved.extend(substituted.0);
                    continue;
                }

                if let Some(fallback) = &variable.fallback {
                    let substituted = substitute_token_list(
                        fallback,
                        variables,
                        stack,
                        property_name,
                        value_css,
                    )?;
                    resolved.extend(substituted.0);
                    continue;
                }

                return Err(unresolved_property_error(
                    property_name,
                    value_css,
                    "a referenced custom property was missing or invalid",
                ));
            }
            TokenOrValue::Function(function) => {
                resolved.push(TokenOrValue::Function(
                    lightningcss::properties::custom::Function {
                        name: function.name.clone(),
                        arguments: substitute_token_list(
                            &function.arguments,
                            variables,
                            stack,
                            property_name,
                            value_css,
                        )?,
                    },
                ));
            }
            TokenOrValue::Env(environment) => {
                resolved.push(TokenOrValue::Env(
                    lightningcss::properties::custom::EnvironmentVariable {
                        name: environment.name.clone(),
                        indices: environment.indices.clone(),
                        fallback: environment
                            .fallback
                            .as_ref()
                            .map(|fallback| {
                                substitute_token_list(
                                    fallback,
                                    variables,
                                    stack,
                                    property_name,
                                    value_css,
                                )
                            })
                            .transpose()?,
                    },
                ));
            }
            other => resolved.push(other.clone()),
        }
    }

    Ok(TokenList(resolved))
}

fn invalid_custom_property_value(name: &str, value: &str) -> StyleError {
    StyleError::UnsupportedValue(format!(
        "failed to parse custom property `{name}` with value `{value}`"
    ))
}

fn unresolved_property_error(property_name: &str, value_css: &str, reason: &str) -> StyleError {
    StyleError::UnsupportedValue(format!(
        "failed to resolve variable-backed declaration `{property_name}: {value_css}`: {reason}"
    ))
}

#[cfg(test)]
mod tests {
    use cssimpler_core::{Color, Node};
    use taffy::prelude::{
        LengthPercentageAuto as TaffyLengthPercentageAuto, Position as TaffyPosition,
    };

    use crate::{Declaration, build_render_tree, parse_stylesheet, resolve_style};

    #[test]
    fn parser_tracks_variable_dependent_properties_for_runtime_resolution() {
        let stylesheet = parse_stylesheet(".card { font-size: var(--fs-size); }")
            .expect("variable-backed stylesheet should parse");

        assert!(stylesheet.rules[0].declarations.contains(
            &Declaration::VariableDependentProperty {
                property_name: "font-size".to_string(),
                value_css: "var(--fs-size)".to_string(),
            }
        ));
    }

    #[test]
    fn variable_dependent_font_size_uses_custom_properties_declared_later_in_the_same_rule() {
        let stylesheet = parse_stylesheet(".card { font-size: var(--fs-size); --fs-size: 24px; }")
            .expect("variable-backed font-size stylesheet should parse");
        let element = Node::element("div").with_class("card");
        let resolved = resolve_style(&element, &stylesheet);

        assert_eq!(resolved.visual.text.size_px, 24.0);
    }

    #[test]
    fn descendant_declarations_can_resolve_inherited_custom_properties() {
        let stylesheet = parse_stylesheet(
            ".button { --fs-size: 20px; }
             .label { font-size: var(--fs-size); }",
        )
        .expect("inherited custom-property stylesheet should parse");
        let tree = Node::element("div")
            .with_class("button")
            .with_child(
                Node::element("span")
                    .with_class("label")
                    .with_child(Node::text("Hi"))
                    .into(),
            )
            .into();
        let scene = build_render_tree(&tree, &stylesheet);

        assert_eq!(scene.children[0].style.text.size_px, 20.0);
    }

    #[test]
    fn fallback_values_are_used_when_a_custom_property_is_missing() {
        let stylesheet = parse_stylesheet(".badge { color: var(--accent, #2563eb); }")
            .expect("variable fallback stylesheet should parse");
        let element = Node::element("div").with_class("badge");
        let resolved = resolve_style(&element, &stylesheet);

        assert_eq!(resolved.visual.foreground, Color::rgb(37, 99, 235));
    }

    #[test]
    fn variable_backed_border_shorthands_resolve_before_paint() {
        let stylesheet = parse_stylesheet(
            ".pane {
                --border-right: 6px;
                border-right: var(--border-right) solid #ddeeff;
            }",
        )
        .expect("variable-backed border stylesheet should parse");
        let element = Node::element("div").with_class("pane");
        let resolved = resolve_style(&element, &stylesheet);

        assert_eq!(resolved.visual.border.widths.right, 6.0);
        assert_eq!(resolved.visual.border.color, Color::rgb(221, 238, 255));
    }

    #[test]
    fn variable_backed_inset_shorthand_resolves_before_layout() {
        let stylesheet = parse_stylesheet(
            ".badge {
                --offset: 12px;
                inset: var(--offset);
            }",
        )
        .expect("variable-backed inset stylesheet should parse");
        let element = Node::element("div").with_class("badge");
        let resolved = resolve_style(&element, &stylesheet);

        assert_eq!(resolved.layout.taffy.position, TaffyPosition::Absolute);
        assert_eq!(
            resolved.layout.taffy.inset.top,
            TaffyLengthPercentageAuto::Length(12.0)
        );
        assert_eq!(
            resolved.layout.taffy.inset.right,
            TaffyLengthPercentageAuto::Length(12.0)
        );
        assert_eq!(
            resolved.layout.taffy.inset.bottom,
            TaffyLengthPercentageAuto::Length(12.0)
        );
        assert_eq!(
            resolved.layout.taffy.inset.left,
            TaffyLengthPercentageAuto::Length(12.0)
        );
    }

    #[test]
    fn variable_backed_text_stroke_resolves_before_paint() {
        let stylesheet = parse_stylesheet(
            ".headline {
                --stroke: 2px #ff6600;
                -webkit-text-stroke: var(--stroke);
            }",
        )
        .expect("variable-backed text stroke stylesheet should parse");
        let element = Node::element("div").with_class("headline");
        let resolved = resolve_style(&element, &stylesheet);

        assert_eq!(resolved.visual.text_stroke.width, 2.0);
        assert_eq!(
            resolved.visual.text_stroke.color,
            Some(Color::rgb(255, 102, 0))
        );
    }

    #[test]
    #[should_panic(
        expected = "failed to resolve variable-backed declaration `color: var(--accent)`"
    )]
    fn unsupported_variable_substitutions_fail_clearly() {
        let stylesheet = parse_stylesheet(
            ".badge {
                --accent: 12px;
                color: var(--accent);
            }",
        )
        .expect("invalid variable-backed stylesheet should parse");
        let element = Node::element("div").with_class("badge");

        let _ = resolve_style(&element, &stylesheet);
    }
}
