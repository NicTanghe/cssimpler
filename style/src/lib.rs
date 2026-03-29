use std::error::Error;
use std::fmt::{Display, Formatter};

use cssimpler_core::Color;

#[derive(Clone, Debug, Default)]
pub struct Stylesheet {
    pub rules: Vec<StyleRule>,
}

impl Stylesheet {
    pub fn push(&mut self, rule: StyleRule) {
        self.rules.push(rule);
    }

    pub fn matching_rules<'a>(
        &'a self,
        element: ElementRef<'a>,
    ) -> impl Iterator<Item = &'a StyleRule> {
        self.rules
            .iter()
            .filter(move |rule| rule.selector.matches(element))
    }
}

#[derive(Clone, Debug)]
pub struct StyleRule {
    pub selector: Selector,
    pub declarations: Vec<Declaration>,
}

impl StyleRule {
    pub fn new(selector: Selector, declarations: Vec<Declaration>) -> Self {
        Self {
            selector,
            declarations,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Declaration {
    Background(Color),
    Foreground(Color),
    X(f32),
    Y(f32),
    Width(f32),
    Height(f32),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Selector {
    Class(String),
    Id(String),
    Tag(String),
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
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ElementRef<'a> {
    pub tag: &'a str,
    pub id: Option<&'a str>,
    pub classes: &'a [String],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StyleError {
    InvalidRule(String),
    UnsupportedSelector(String),
    UnsupportedProperty(String),
    InvalidColor(String),
    InvalidLength(String),
}

impl Display for StyleError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidRule(rule) => write!(f, "invalid CSS rule: {rule}"),
            Self::UnsupportedSelector(selector) => {
                write!(f, "unsupported selector: {selector}")
            }
            Self::UnsupportedProperty(property) => {
                write!(f, "unsupported property: {property}")
            }
            Self::InvalidColor(value) => write!(f, "invalid color value: {value}"),
            Self::InvalidLength(value) => write!(f, "invalid length value: {value}"),
        }
    }
}

impl Error for StyleError {}

pub fn parse_stylesheet(source: &str) -> Result<Stylesheet, StyleError> {
    let mut stylesheet = Stylesheet::default();

    for raw_block in source.split('}') {
        let block = raw_block.trim();
        if block.is_empty() {
            continue;
        }

        let (selector_text, body) = block
            .split_once('{')
            .ok_or_else(|| StyleError::InvalidRule(block.to_string()))?;
        let selector = parse_selector(selector_text.trim())?;
        let mut declarations = Vec::new();

        for raw_declaration in body.split(';') {
            let declaration = raw_declaration.trim();
            if declaration.is_empty() {
                continue;
            }

            let (property, value) = declaration
                .split_once(':')
                .ok_or_else(|| StyleError::InvalidRule(declaration.to_string()))?;
            declarations.push(parse_declaration(property.trim(), value.trim())?);
        }

        stylesheet.push(StyleRule::new(selector, declarations));
    }

    Ok(stylesheet)
}

fn parse_selector(selector: &str) -> Result<Selector, StyleError> {
    if selector.is_empty() || selector.chars().any(char::is_whitespace) {
        return Err(StyleError::UnsupportedSelector(selector.to_string()));
    }

    if let Some(value) = selector.strip_prefix('.') {
        return Ok(Selector::Class(value.to_string()));
    }

    if let Some(value) = selector.strip_prefix('#') {
        return Ok(Selector::Id(value.to_string()));
    }

    Ok(Selector::Tag(selector.to_string()))
}

fn parse_declaration(property: &str, value: &str) -> Result<Declaration, StyleError> {
    match property {
        "background" | "background-color" => Ok(Declaration::Background(parse_color(value)?)),
        "color" => Ok(Declaration::Foreground(parse_color(value)?)),
        "x" => Ok(Declaration::X(parse_length(value)?)),
        "y" => Ok(Declaration::Y(parse_length(value)?)),
        "width" => Ok(Declaration::Width(parse_length(value)?)),
        "height" => Ok(Declaration::Height(parse_length(value)?)),
        other => Err(StyleError::UnsupportedProperty(other.to_string())),
    }
}

fn parse_color(value: &str) -> Result<Color, StyleError> {
    let value = value.trim();
    let Some(hex) = value.strip_prefix('#') else {
        return Err(StyleError::InvalidColor(value.to_string()));
    };

    if hex.len() != 6 {
        return Err(StyleError::InvalidColor(value.to_string()));
    }

    let rgb =
        u32::from_str_radix(hex, 16).map_err(|_| StyleError::InvalidColor(value.to_string()))?;

    Ok(Color::rgb(
        ((rgb >> 16) & 0xFF) as u8,
        ((rgb >> 8) & 0xFF) as u8,
        (rgb & 0xFF) as u8,
    ))
}

fn parse_length(value: &str) -> Result<f32, StyleError> {
    let normalized = value.trim().strip_suffix("px").unwrap_or(value.trim());
    normalized
        .parse::<f32>()
        .map_err(|_| StyleError::InvalidLength(value.to_string()))
}

#[cfg(test)]
mod tests {
    use cssimpler_core::Color;

    use super::{Declaration, ElementRef, Selector, StyleRule, Stylesheet, parse_stylesheet};

    #[test]
    fn selectors_match_supported_primitives() {
        let classes = vec!["card".to_string(), "selected".to_string()];
        let element = ElementRef {
            tag: "div",
            id: Some("hero"),
            classes: &classes,
        };

        assert!(Selector::Class("card".to_string()).matches(element));
        assert!(Selector::Id("hero".to_string()).matches(element));
        assert!(Selector::Tag("div".to_string()).matches(element));
        assert!(!Selector::Class("ghost".to_string()).matches(element));
    }

    #[test]
    fn matching_rules_are_returned_in_insertion_order() {
        let classes = vec!["card".to_string()];
        let mut stylesheet = Stylesheet::default();
        stylesheet.push(StyleRule::new(
            Selector::Class("card".to_string()),
            vec![Declaration::Width(300.0)],
        ));
        stylesheet.push(StyleRule::new(
            Selector::Tag("p".to_string()),
            vec![Declaration::Height(40.0)],
        ));

        let matching: Vec<_> = stylesheet
            .matching_rules(ElementRef {
                tag: "div",
                id: None,
                classes: &classes,
            })
            .collect();

        assert_eq!(matching.len(), 1);
    }

    #[test]
    fn parser_supports_a_small_bootstrap_stylesheet() {
        let stylesheet = parse_stylesheet(
            "#app { width: 960px; height: 540px; background: #e2e8f0; }
             .card { x: 160px; y: 128px; width: 640px; height: 220px; background: #0f172a; color: #ffffff; }",
        )
        .expect("bootstrap stylesheet should parse");

        assert_eq!(stylesheet.rules.len(), 2);
        assert!(matches!(
            stylesheet.rules[0].declarations[2],
            Declaration::Background(Color {
                r: 226,
                g: 232,
                b: 240,
                a: 255
            })
        ));
    }
}
