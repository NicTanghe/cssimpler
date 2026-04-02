use std::collections::BTreeMap;

use cssimpler_core::{ElementInteractionState, ElementNode, ElementPath};
use cssparser::ToCss;
use lightningcss::selector::{
    Combinator as LightningCombinator, Component, PseudoClass,
    PseudoElement as LightningPseudoElement, Selector as LightningSelector,
};

use crate::StyleError;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct InteractionDependencies {
    pub hover: bool,
    pub active: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PseudoElementKind {
    Before,
    After,
}

impl InteractionDependencies {
    pub(crate) const fn is_empty(self) -> bool {
        !self.hover && !self.active
    }

    pub(crate) const fn intersects(self, other: Self) -> bool {
        (self.hover && other.hover) || (self.active && other.active)
    }

    const fn merge(self, other: Self) -> Self {
        Self {
            hover: self.hover || other.hover,
            active: self.active || other.active,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SimpleSelector {
    Class(String),
    Id(String),
    Tag(String),
    AttributeExists(String),
    AttributeEquals { name: String, value: String },
    Hover,
    Active,
}

impl SimpleSelector {
    pub(crate) const fn interaction_dependencies(&self) -> InteractionDependencies {
        match self {
            Self::Hover => InteractionDependencies {
                hover: true,
                active: false,
            },
            Self::Active => InteractionDependencies {
                hover: false,
                active: true,
            },
            _ => InteractionDependencies {
                hover: false,
                active: false,
            },
        }
    }

    pub fn matches(&self, element: ElementRef<'_>) -> bool {
        self.matches_with_interaction(
            element,
            &ElementPath::root(0),
            &ElementInteractionState::default(),
        )
    }

    pub fn matches_with_interaction(
        &self,
        element: ElementRef<'_>,
        element_path: &ElementPath,
        interaction: &ElementInteractionState,
    ) -> bool {
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
            Self::Hover => interaction.is_hovered(element_path),
            Self::Active => interaction.is_active(element_path),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompoundSelector {
    pub simple_selectors: Vec<SimpleSelector>,
}

impl CompoundSelector {
    pub fn new(simple_selectors: Vec<SimpleSelector>) -> Self {
        assert!(
            !simple_selectors.is_empty(),
            "compound selectors require at least one simple selector"
        );
        Self { simple_selectors }
    }

    pub fn matches(&self, element: ElementRef<'_>) -> bool {
        self.matches_with_interaction(
            element,
            &ElementPath::root(0),
            &ElementInteractionState::default(),
        )
    }

    pub fn matches_with_interaction(
        &self,
        element: ElementRef<'_>,
        element_path: &ElementPath,
        interaction: &ElementInteractionState,
    ) -> bool {
        self.simple_selectors
            .iter()
            .all(|selector| selector.matches_with_interaction(element, element_path, interaction))
    }

    pub(crate) fn interaction_dependencies(&self) -> InteractionDependencies {
        self.simple_selectors.iter().fold(
            InteractionDependencies::default(),
            |dependencies, selector| dependencies.merge(selector.interaction_dependencies()),
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectorCombinator {
    Descendant,
    Child,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AncestorSelector {
    pub combinator: SelectorCombinator,
    pub compound: CompoundSelector,
}

impl AncestorSelector {
    pub fn new(combinator: SelectorCombinator, compound: CompoundSelector) -> Self {
        Self {
            combinator,
            compound,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selector {
    pub rightmost: CompoundSelector,
    pub ancestors: Vec<AncestorSelector>,
    pub pseudo_element: Option<PseudoElementKind>,
}

impl Selector {
    pub fn class(name: impl Into<String>) -> Self {
        Self::compound(vec![SimpleSelector::Class(name.into())])
    }

    pub fn id(name: impl Into<String>) -> Self {
        Self::compound(vec![SimpleSelector::Id(name.into())])
    }

    pub fn tag(name: impl Into<String>) -> Self {
        Self::compound(vec![SimpleSelector::Tag(name.into())])
    }

    pub fn attribute_exists(name: impl Into<String>) -> Self {
        Self::compound(vec![SimpleSelector::AttributeExists(name.into())])
    }

    pub fn attribute_equals(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self::compound(vec![SimpleSelector::AttributeEquals {
            name: name.into(),
            value: value.into(),
        }])
    }

    pub fn compound(simple_selectors: Vec<SimpleSelector>) -> Self {
        Self {
            rightmost: CompoundSelector::new(simple_selectors),
            ancestors: Vec::new(),
            pseudo_element: None,
        }
    }

    pub fn complex(
        rightmost: CompoundSelector,
        ancestors: Vec<AncestorSelector>,
        pseudo_element: Option<PseudoElementKind>,
    ) -> Self {
        Self {
            rightmost,
            ancestors,
            pseudo_element,
        }
    }

    pub fn matches(&self, element: ElementRef<'_>) -> bool {
        self.matches_with_ancestors(element, &[])
    }

    pub fn matches_with_ancestors(
        &self,
        element: ElementRef<'_>,
        ancestors: &[ElementRef<'_>],
    ) -> bool {
        let mut path = ElementPath::root(0);
        for _ in ancestors {
            path = path.with_child(0);
        }
        self.matches_with_ancestors_interaction_and_pseudo(
            element,
            ancestors,
            &path,
            &ElementInteractionState::default(),
            None,
        )
    }

    pub fn matches_with_ancestors_and_interaction(
        &self,
        element: ElementRef<'_>,
        ancestors: &[ElementRef<'_>],
        element_path: &ElementPath,
        interaction: &ElementInteractionState,
    ) -> bool {
        self.matches_with_ancestors_interaction_and_pseudo(
            element,
            ancestors,
            element_path,
            interaction,
            None,
        )
    }

    pub fn matches_with_ancestors_interaction_and_pseudo(
        &self,
        element: ElementRef<'_>,
        ancestors: &[ElementRef<'_>],
        element_path: &ElementPath,
        interaction: &ElementInteractionState,
        pseudo_element: Option<PseudoElementKind>,
    ) -> bool {
        if self.pseudo_element != pseudo_element {
            return false;
        }

        if !self
            .rightmost
            .matches_with_interaction(element, element_path, interaction)
        {
            return false;
        }

        let mut ancestor_index = 0;
        for selector in &self.ancestors {
            match selector.combinator {
                SelectorCombinator::Child => {
                    let Some(parent) = ancestors.get(ancestor_index) else {
                        return false;
                    };
                    let Some(parent_path) = ancestor_path(element_path, ancestor_index) else {
                        return false;
                    };
                    if !selector.compound.matches_with_interaction(
                        *parent,
                        &parent_path,
                        interaction,
                    ) {
                        return false;
                    }
                    ancestor_index += 1;
                }
                SelectorCombinator::Descendant => {
                    let Some(offset) = ancestors[ancestor_index..].iter().enumerate().position(
                        |(offset, ancestor)| {
                            ancestor_path(element_path, ancestor_index + offset).is_some_and(
                                |ancestor_path| {
                                    selector.compound.matches_with_interaction(
                                        *ancestor,
                                        &ancestor_path,
                                        interaction,
                                    )
                                },
                            )
                        },
                    ) else {
                        return false;
                    };
                    ancestor_index += offset + 1;
                }
            }
        }

        true
    }

    pub(crate) fn interaction_dependencies(&self) -> InteractionDependencies {
        self.ancestors.iter().fold(
            self.rightmost.interaction_dependencies(),
            |dependencies, ancestor| {
                dependencies.merge(ancestor.compound.interaction_dependencies())
            },
        )
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
    let components = selector.iter_raw_match_order().as_slice();
    let compounds = components
        .split(|component| {
            matches!(
                component,
                Component::Combinator(combinator)
                    if *combinator != LightningCombinator::PseudoElement
            )
        })
        .map(|compound| extract_compound_selector(selector, compound))
        .collect::<Result<Vec<_>, _>>()?;
    let mut pseudo_element = None;
    let mut compound_selectors = Vec::with_capacity(compounds.len());
    for (index, (compound, compound_pseudo)) in compounds.into_iter().enumerate() {
        if index > 0 && compound_pseudo.is_some() {
            return Err(unsupported_selector(selector));
        }
        if let Some(compound_pseudo) = compound_pseudo {
            if pseudo_element.replace(compound_pseudo).is_some() {
                return Err(unsupported_selector(selector));
            }
        }
        compound_selectors.push(compound);
    }
    let combinators = components
        .iter()
        .filter_map(|component| match component {
            Component::Combinator(LightningCombinator::PseudoElement) => None,
            Component::Combinator(combinator) => Some(*combinator),
            _ => None,
        })
        .map(|combinator| extract_combinator(selector, combinator))
        .collect::<Result<Vec<_>, _>>()?;

    let Some((rightmost, ancestors)) = compound_selectors.split_first() else {
        return Err(unsupported_selector(selector));
    };
    if ancestors.len() != combinators.len() {
        return Err(unsupported_selector(selector));
    }

    Ok(Selector::complex(
        rightmost.clone(),
        ancestors
            .iter()
            .cloned()
            .zip(combinators)
            .map(|(compound, combinator)| AncestorSelector::new(combinator, compound))
            .collect(),
        pseudo_element,
    ))
}

fn extract_compound_selector(
    selector: &LightningSelector<'_>,
    compound: &[Component<'_>],
) -> Result<(CompoundSelector, Option<PseudoElementKind>), StyleError> {
    let mut pseudo_element = None;
    let simple_selectors = compound
        .iter()
        .map(|component| {
            extract_simple_selector(selector, component).map(
                |(simple_selector, component_pseudo)| {
                    if let Some(component_pseudo) = component_pseudo {
                        if pseudo_element.replace(component_pseudo).is_some() {
                            return Err(unsupported_selector(selector));
                        }
                    }
                    Ok(simple_selector)
                },
            )?
        })
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();

    if simple_selectors.is_empty() {
        return Err(unsupported_selector(selector));
    }

    Ok((CompoundSelector::new(simple_selectors), pseudo_element))
}

fn extract_simple_selector(
    selector: &LightningSelector<'_>,
    component: &Component<'_>,
) -> Result<(Option<SimpleSelector>, Option<PseudoElementKind>), StyleError> {
    match component {
        Component::Class(name) => Ok((Some(SimpleSelector::Class(name.0.to_string())), None)),
        Component::ID(name) => Ok((Some(SimpleSelector::Id(name.0.to_string())), None)),
        Component::LocalName(name) => {
            Ok((Some(SimpleSelector::Tag(name.name.0.to_string())), None))
        }
        Component::AttributeInNoNamespaceExists { local_name, .. } => Ok((
            Some(extract_attribute_selector(
                component,
                selector,
                local_name.as_ref(),
                None,
            )?),
            None,
        )),
        Component::AttributeInNoNamespace {
            local_name, value, ..
        } => Ok((
            Some(extract_attribute_selector(
                component,
                selector,
                local_name.as_ref(),
                Some(value.as_ref()),
            )?),
            None,
        )),
        Component::NonTSPseudoClass(PseudoClass::Hover) => Ok((Some(SimpleSelector::Hover), None)),
        Component::NonTSPseudoClass(PseudoClass::Active) => {
            Ok((Some(SimpleSelector::Active), None))
        }
        Component::PseudoElement(LightningPseudoElement::Before) => {
            Ok((None, Some(PseudoElementKind::Before)))
        }
        Component::PseudoElement(LightningPseudoElement::After) => {
            Ok((None, Some(PseudoElementKind::After)))
        }
        Component::Combinator(LightningCombinator::PseudoElement) => Ok((None, None)),
        Component::ExplicitUniversalType => Ok((None, None)),
        _ => Err(unsupported_selector(selector)),
    }
}

fn extract_combinator(
    selector: &LightningSelector<'_>,
    combinator: LightningCombinator,
) -> Result<SelectorCombinator, StyleError> {
    match combinator {
        LightningCombinator::Descendant => Ok(SelectorCombinator::Descendant),
        LightningCombinator::Child => Ok(SelectorCombinator::Child),
        _ => Err(unsupported_selector(selector)),
    }
}

fn extract_attribute_selector(
    component: &Component<'_>,
    selector: &LightningSelector<'_>,
    name: &str,
    value: Option<&str>,
) -> Result<SimpleSelector, StyleError> {
    let serialized = component.to_css_string();
    let Some(inner) = serialized
        .trim()
        .strip_prefix('[')
        .and_then(|selector| selector.strip_suffix(']'))
    else {
        return Err(unsupported_selector(selector));
    };
    let Some(suffix) = inner.trim().strip_prefix(name) else {
        return Err(unsupported_selector(selector));
    };
    let suffix = suffix.trim_start();

    match value {
        None if suffix.is_empty() => Ok(SimpleSelector::AttributeExists(name.to_string())),
        Some(value) if is_supported_attribute_equals_selector(suffix) => {
            Ok(SimpleSelector::AttributeEquals {
                name: name.to_string(),
                value: value.to_string(),
            })
        }
        _ => Err(unsupported_selector(selector)),
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

fn unsupported_selector(selector: &LightningSelector<'_>) -> StyleError {
    StyleError::UnsupportedSelector(format!("{selector:?}"))
}

fn ancestor_path(path: &ElementPath, ancestor_index: usize) -> Option<ElementPath> {
    path.ancestor(ancestor_index + 1)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use cssimpler_core::{Color, ElementInteractionState, ElementPath, Node};

    use crate::{StyleError, parse_stylesheet, resolve_style, resolve_style_with_interaction};

    use super::{
        AncestorSelector, CompoundSelector, ElementRef, PseudoElementKind, Selector,
        SelectorCombinator, SimpleSelector,
    };

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

        assert!(SimpleSelector::Class("card".to_string()).matches(element));
        assert!(SimpleSelector::Id("hero".to_string()).matches(element));
        assert!(SimpleSelector::Tag("div".to_string()).matches(element));
        assert!(!SimpleSelector::Class("ghost".to_string()).matches(element));
    }

    #[test]
    fn compound_selectors_require_all_simple_selectors() {
        let classes = vec!["button".to_string(), "primary".to_string()];
        let attributes = BTreeMap::new();
        let element = ElementRef {
            tag: "button",
            id: None,
            classes: &classes,
            attributes: &attributes,
        };

        assert!(
            Selector::compound(vec![
                SimpleSelector::Tag("button".to_string()),
                SimpleSelector::Class("button".to_string()),
                SimpleSelector::Class("primary".to_string()),
            ])
            .matches(element)
        );
        assert!(
            !Selector::compound(vec![
                SimpleSelector::Tag("button".to_string()),
                SimpleSelector::Class("ghost".to_string()),
            ])
            .matches(element)
        );
    }

    #[test]
    fn combinator_selectors_match_against_ancestor_context() {
        let root_classes = vec!["panel".to_string()];
        let button_classes = vec!["button".to_string()];
        let text_classes = vec!["hover-text".to_string()];
        let attributes = BTreeMap::new();
        let element = ElementRef {
            tag: "span",
            id: None,
            classes: &text_classes,
            attributes: &attributes,
        };
        let button = ElementRef {
            tag: "button",
            id: None,
            classes: &button_classes,
            attributes: &attributes,
        };
        let root = ElementRef {
            tag: "div",
            id: None,
            classes: &root_classes,
            attributes: &attributes,
        };

        let descendant = Selector::complex(
            CompoundSelector::new(vec![SimpleSelector::Class("hover-text".to_string())]),
            vec![AncestorSelector::new(
                SelectorCombinator::Descendant,
                CompoundSelector::new(vec![SimpleSelector::Class("button".to_string())]),
            )],
            None,
        );
        let child = Selector::complex(
            CompoundSelector::new(vec![SimpleSelector::Class("hover-text".to_string())]),
            vec![AncestorSelector::new(
                SelectorCombinator::Child,
                CompoundSelector::new(vec![SimpleSelector::Class("button".to_string())]),
            )],
            None,
        );

        assert!(descendant.matches_with_ancestors(element, &[button, root]));
        assert!(child.matches_with_ancestors(element, &[button, root]));
        assert!(!child.matches_with_ancestors(element, &[root, button]));
    }

    #[test]
    fn attribute_selectors_match_presence_and_exact_values() {
        let element = Node::element("div")
            .with_attribute("data-text", "uiverse")
            .with_attribute("aria-hidden", "true");
        let element_ref = ElementRef::from(&element);

        assert!(SimpleSelector::AttributeExists("data-text".to_string()).matches(element_ref));
        assert!(
            SimpleSelector::AttributeEquals {
                name: "aria-hidden".to_string(),
                value: "true".to_string(),
            }
            .matches(element_ref)
        );
        assert!(
            !SimpleSelector::AttributeEquals {
                name: "aria-hidden".to_string(),
                value: "false".to_string(),
            }
            .matches(element_ref)
        );
    }

    #[test]
    fn interactive_selectors_match_current_and_ancestor_paths() {
        let classes = vec!["button".to_string()];
        let attributes = BTreeMap::new();
        let button = ElementRef {
            tag: "button",
            id: None,
            classes: &classes,
            attributes: &attributes,
        };
        let label = ElementPath::root(0).with_child(0).with_child(1);
        let button_path = label.ancestor(1).expect("button ancestor path");
        let interaction = ElementInteractionState {
            hovered: Some(label.clone()),
            active: Some(label),
        };

        assert!(SimpleSelector::Hover.matches_with_interaction(button, &button_path, &interaction));
        assert!(SimpleSelector::Active.matches_with_interaction(
            button,
            &button_path,
            &interaction
        ));
        assert!(!SimpleSelector::Hover.matches_with_interaction(
            button,
            &ElementPath::root(1),
            &interaction
        ));
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
            Selector::attribute_exists("data-text")
        );
        assert_eq!(
            stylesheet.rules[1].selector,
            Selector::attribute_equals("aria-hidden", "true")
        );
    }

    #[test]
    fn parser_supports_compound_and_combinator_selectors() {
        let stylesheet = parse_stylesheet("button.button.primary > .hover-text { width: 120px; }")
            .expect("compound selectors should parse");

        assert_eq!(stylesheet.rules.len(), 1);
        assert_eq!(
            stylesheet.rules[0].selector,
            Selector::complex(
                CompoundSelector::new(vec![SimpleSelector::Class("hover-text".to_string())]),
                vec![AncestorSelector::new(
                    SelectorCombinator::Child,
                    CompoundSelector::new(vec![
                        SimpleSelector::Tag("button".to_string()),
                        SimpleSelector::Class("button".to_string()),
                        SimpleSelector::Class("primary".to_string()),
                    ]),
                )],
                None,
            )
        );
    }

    #[test]
    fn parser_supports_hover_and_active_pseudo_classes() {
        let stylesheet =
            parse_stylesheet(".button:hover { width: 120px; } .button:active { height: 40px; }")
                .expect("interactive pseudo classes should parse");

        assert_eq!(
            stylesheet.rules[0].selector,
            Selector::compound(vec![
                SimpleSelector::Class("button".to_string()),
                SimpleSelector::Hover,
            ])
        );
        assert_eq!(
            stylesheet.rules[1].selector,
            Selector::compound(vec![
                SimpleSelector::Class("button".to_string()),
                SimpleSelector::Active,
            ])
        );
    }

    #[test]
    fn parser_supports_before_and_after_pseudo_elements() {
        let stylesheet = parse_stylesheet(
            ".button::before { color: #2563eb; } .button:hover::after { width: 40px; }",
        )
        .expect("generated content pseudo elements should parse");

        assert_eq!(
            stylesheet.rules[0].selector,
            Selector::complex(
                CompoundSelector::new(vec![SimpleSelector::Class("button".to_string())]),
                Vec::new(),
                Some(PseudoElementKind::Before),
            )
        );
        assert_eq!(
            stylesheet.rules[1].selector,
            Selector::complex(
                CompoundSelector::new(vec![
                    SimpleSelector::Class("button".to_string()),
                    SimpleSelector::Hover,
                ]),
                Vec::new(),
                Some(PseudoElementKind::After),
            )
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
    fn interactive_pseudo_classes_participate_in_style_resolution() {
        let stylesheet =
            parse_stylesheet(".button:hover { color: #2563eb; } .button:active { width: 120px; }")
                .expect("interactive selectors should parse");
        let button = Node::element("button").with_class("button");
        let interaction = ElementInteractionState {
            hovered: Some(ElementPath::root(0)),
            active: Some(ElementPath::root(0)),
        };
        let resolved = resolve_style_with_interaction(
            &button,
            &stylesheet,
            &interaction,
            &ElementPath::root(0),
        );

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

    #[test]
    fn parser_rejects_unsupported_combinators() {
        let error = parse_stylesheet(".button + .hover-text { width: 120px; }")
            .expect_err("sibling selectors should be rejected");

        assert!(matches!(error, StyleError::UnsupportedSelector(_)));
    }

    #[test]
    fn parser_rejects_unsupported_pseudo_classes() {
        let error = parse_stylesheet(".button:focus { width: 120px; }")
            .expect_err("focus stays out of scope for this slice");

        assert!(matches!(error, StyleError::UnsupportedSelector(_)));
    }
}
