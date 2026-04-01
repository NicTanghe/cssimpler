use std::collections::BTreeMap;

use cssimpler_core::ElementNode;
use lightningcss::printer::PrinterOptions;
use lightningcss::selector::{Component, Selector as LightningSelector};
use lightningcss::traits::ToCss;

use crate::StyleError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Selector {
    Class(String),
    Id(String),
    Tag(String),
    AttributeExists(String),
    AttributeEquals { name: String, value: String },
}

impl Selector {
    pub fn matches(&self, element: ElementRef<'_>) -> bool {
        match self {
            Self::Class(expected) => element
                .classes
                .iter()
                .any(|class_name| class_name == expected),
            Self::Id(expected) => element.id.is_some_and(|actual| actual == expected),
            Self::Tag(expected) => element.tag == expected,
            Self::AttributeExists(name) => element.attribute(name).is_some(),
            Self::AttributeEquals { name, value } => {
                element.attribute(name) == Some(value.as_str())
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ElementRef<'a> {
    pub tag: &'a str,
    pub id: Option<&'a str>,
    pub classes: &'a [String],
    pub attributes: &'a BTreeMap<String, String>,
}

impl<'a> ElementRef<'a> {
    pub fn attribute(&self, name: &str) -> Option<&'a str> {
        self.attributes.get(name).map(String::as_str)
    }
}

impl<'a> From<&'a ElementNode> for ElementRef<'a> {
    fn from(element: &'a ElementNode) -> Self {
        Self {
            tag: &element.tag,
            id: element.id.as_deref(),
            classes: &element.classes,
            attributes: element.attributes(),
        }
    }
}

pub fn extract_selector(selector: &LightningSelector<'_>) -> Result<Selector, StyleError> {
    let mut resolved = None;

    for component in selector.iter_raw_match_order() {
        let candidate = match component {
            Component::Class(name) => Selector::Class(name.0.to_string()),
            Component::ID(name) => Selector::Id(name.0.to_string()),
            Component::LocalName(name) => Selector::Tag(name.name.0.to_string()),
            Component::AttributeInNoNamespaceExists { local_name, .. } => {
                extract_attribute_selector(selector, local_name.as_ref(), None)?
            }
            Component::AttributeInNoNamespace {
                local_name, value, ..
            } => extract_attribute_selector(selector, local_name.as_ref(), Some(value.as_ref()))?,
            Component::ExplicitUniversalType => continue,
            _ => {
                return Err(StyleError::UnsupportedSelector(format!("{selector:?}")));
            }
        };

        if resolved.replace(candidate).is_some() {
            return Err(StyleError::UnsupportedSelector(format!("{selector:?}")));
        }
    }

    resolved.ok_or_else(|| StyleError::UnsupportedSelector(format!("{selector:?}")))
}

fn extract_attribute_selector(
    selector: &LightningSelector<'_>,
    name: &str,
    value: Option<&str>,
) -> Result<Selector, StyleError> {
    let serialized = selector
        .to_css_string(PrinterOptions::default())
        .map_err(|_| StyleError::UnsupportedSelector(format!("{selector:?}")))?;
    let Some(inner) = serialized
        .trim()
        .strip_prefix('[')
        .and_then(|selector| selector.strip_suffix(']'))
    else {
        return Err(StyleError::UnsupportedSelector(format!("{selector:?}")));
    };
    let Some(suffix) = inner.trim().strip_prefix(name) else {
        return Err(StyleError::UnsupportedSelector(format!("{selector:?}")));
    };
    let suffix = suffix.trim_start();

    match value {
        None if suffix.is_empty() => Ok(Selector::AttributeExists(name.to_string())),
        Some(value) if is_supported_attribute_equals_selector(suffix) => {
            Ok(Selector::AttributeEquals {
                name: name.to_string(),
                value: value.to_string(),
            })
        }
        _ => Err(StyleError::UnsupportedSelector(format!("{selector:?}"))),
    }
}

fn is_supported_attribute_equals_selector(suffix: &str) -> bool {
    let Some(value) = suffix.strip_prefix('=') else {
        return false;
    };
    let value = value.trim();

    is_supported_attribute_selector_value(value)
}

fn is_supported_attribute_selector_value(value: &str) -> bool {
    match value.as_bytes() {
        [b'"', middle @ .., b'"'] | [b'\'', middle @ .., b'\''] => !middle.ends_with(&[b'\\']),
        _ => is_identifier_like(value),
    }
}

fn is_identifier_like(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use cssimpler_core::{Color, Node};

    use crate::{StyleError, parse_stylesheet, resolve_style};

    use super::{ElementRef, Selector};

    #[test]
    fn selectors_match_supported_primitives() {
        let classes = vec!["card".to_string(), "selected".to_string()];
        let attributes = BTreeMap::new();
        let element = ElementRef {
            tag: "div",
            id: Some("hero"),
            classes: &classes,
            attributes: &attributes,
        };

        assert!(Selector::Class("card".to_string()).matches(element));
        assert!(Selector::Id("hero".to_string()).matches(element));
        assert!(Selector::Tag("div".to_string()).matches(element));
        assert!(!Selector::Class("ghost".to_string()).matches(element));
    }

    #[test]
    fn attribute_selectors_match_presence_and_exact_values() {
        let element = Node::element("div")
            .with_attribute("data-text", "uiverse")
            .with_attribute("aria-hidden", "true");
        let element_ref = ElementRef::from(&element);

        assert!(Selector::AttributeExists("data-text".to_string()).matches(element_ref));
        assert!(
            Selector::AttributeEquals {
                name: "aria-hidden".to_string(),
                value: "true".to_string(),
            }
            .matches(element_ref)
        );
        assert!(
            !Selector::AttributeEquals {
                name: "aria-hidden".to_string(),
                value: "false".to_string(),
            }
            .matches(element_ref)
        );
    }

    #[test]
    fn style_time_element_refs_expose_generic_attributes() {
        let element = Node::element("div")
            .with_id("hero")
            .with_class("card")
            .with_attribute("data-text", "uiverse")
            .with_attribute("aria-hidden", "true");
        let element_ref = ElementRef::from(&element);

        assert_eq!(element_ref.id, Some("hero"));
        assert_eq!(element_ref.attribute("class"), Some("card"));
        assert_eq!(element_ref.attribute("data-text"), Some("uiverse"));
        assert_eq!(element_ref.attribute("aria-hidden"), Some("true"));
    }

    #[test]
    fn parser_supports_attribute_presence_and_equality_selectors() {
        let stylesheet = parse_stylesheet(
            "[data-text] { width: 120px; } [aria-hidden=\"true\"] { height: 40px; }",
        )
        .expect("attribute selectors should parse");

        assert_eq!(stylesheet.rules.len(), 2);
        assert_eq!(
            stylesheet.rules[0].selector,
            Selector::AttributeExists("data-text".to_string())
        );
        assert_eq!(
            stylesheet.rules[1].selector,
            Selector::AttributeEquals {
                name: "aria-hidden".to_string(),
                value: "true".to_string(),
            }
        );
    }

    #[test]
    fn attribute_selectors_participate_in_style_resolution() {
        let stylesheet = parse_stylesheet(
            "[data-text] { color: #2563eb; } [aria-hidden=\"true\"] { width: 120px; }",
        )
        .expect("attribute selectors should parse");
        let element = Node::element("div")
            .with_attribute("data-text", "uiverse")
            .with_attribute("aria-hidden", "true");
        let resolved = resolve_style(&element, &stylesheet);

        assert_eq!(resolved.visual.foreground, Color::rgb(37, 99, 235));
        assert_eq!(
            resolved.layout.taffy.size.width,
            taffy::prelude::Dimension::Length(120.0)
        );
    }

    #[test]
    fn parser_rejects_unsupported_attribute_selector_operators() {
        let error = parse_stylesheet("[data-text^=\"ui\"] { width: 120px; }")
            .expect_err("prefix attribute selectors should be rejected");

        assert!(matches!(error, StyleError::UnsupportedSelector(_)));
    }
}
