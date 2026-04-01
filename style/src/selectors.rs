use std::collections::BTreeMap;

use cssimpler_core::ElementNode;
use lightningcss::selector::{Component, Selector as LightningSelector};

use crate::StyleError;

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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use cssimpler_core::Node;

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
}
