use std::error::Error;
use std::fmt::{Display, Formatter};

use cssimpler_core::{Color, ElementNode, LayoutBox, Node, RenderNode, VisualStyle};
use lightningcss::declaration::DeclarationBlock;
use lightningcss::properties::Property;
use lightningcss::properties::background::Background;
use lightningcss::properties::size::Size;
use lightningcss::rules::CssRule;
use lightningcss::selector::{Component, Selector as LightningSelector};
use lightningcss::stylesheet::{ParserOptions, StyleSheet};
use lightningcss::values::color::CssColor;
use lightningcss::values::length::{LengthPercentage, LengthPercentageOrAuto};

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
    Left(f32),
    Top(f32),
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
    Parse(String),
    UnsupportedRule(String),
    UnsupportedSelector(String),
    UnsupportedValue(String),
}

impl Display for StyleError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse(error) => write!(f, "failed to parse stylesheet: {error}"),
            Self::UnsupportedRule(rule) => write!(f, "unsupported rule: {rule}"),
            Self::UnsupportedSelector(selector) => write!(f, "unsupported selector: {selector}"),
            Self::UnsupportedValue(value) => write!(f, "unsupported value: {value}"),
        }
    }
}

impl Error for StyleError {}

pub fn parse_stylesheet(source: &str) -> Result<Stylesheet, StyleError> {
    let parsed = StyleSheet::parse(source, ParserOptions::default())
        .map_err(|error| StyleError::Parse(error.to_string()))?;
    let mut stylesheet = Stylesheet::default();

    for rule in &parsed.rules.0 {
        let CssRule::Style(style_rule) = rule else {
            return Err(StyleError::UnsupportedRule(
                "only top-level style rules are supported".to_string(),
            ));
        };

        if !style_rule.rules.0.is_empty() {
            return Err(StyleError::UnsupportedRule(
                "nested style rules are not supported".to_string(),
            ));
        }

        let declarations = extract_declarations(&style_rule.declarations)?;
        for selector in style_rule
            .selectors
            .0
            .iter()
            .map(extract_selector)
            .collect::<Result<Vec<_>, _>>()?
        {
            stylesheet.push(StyleRule::new(selector, declarations.clone()));
        }
    }

    Ok(stylesheet)
}

pub fn build_render_tree(root: &Node, stylesheet: &Stylesheet) -> RenderNode {
    match root {
        Node::Element(element) => element_to_render_node(element, stylesheet),
        Node::Text(_) => panic!("render tree roots must be elements"),
    }
}

fn extract_selector(selector: &LightningSelector<'_>) -> Result<Selector, StyleError> {
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

fn extract_declarations(block: &DeclarationBlock<'_>) -> Result<Vec<Declaration>, StyleError> {
    let mut declarations = Vec::new();

    for (property, _important) in block.iter() {
        if let Some(declaration) = extract_property(property)? {
            declarations.push(declaration);
        }
    }

    Ok(declarations)
}

fn extract_property(property: &Property<'_>) -> Result<Option<Declaration>, StyleError> {
    match property {
        Property::BackgroundColor(color) => {
            Ok(Some(Declaration::Background(color_from_css(color)?)))
        }
        Property::Background(backgrounds) => {
            Ok(extract_background_declaration(backgrounds.first())?)
        }
        Property::Color(color) => Ok(Some(Declaration::Foreground(color_from_css(color)?))),
        Property::Width(size) => Ok(Some(Declaration::Width(px_from_size(size)?))),
        Property::Height(size) => Ok(Some(Declaration::Height(px_from_size(size)?))),
        Property::Left(value) => Ok(Some(Declaration::Left(px_from_inset(value)?))),
        Property::Top(value) => Ok(Some(Declaration::Top(px_from_inset(value)?))),
        _ => Ok(None),
    }
}

fn extract_background_declaration(
    background: Option<&Background<'_>>,
) -> Result<Option<Declaration>, StyleError> {
    let Some(background) = background else {
        return Ok(None);
    };

    Ok(Some(Declaration::Background(color_from_css(
        &background.color,
    )?)))
}

fn color_from_css(color: &CssColor) -> Result<Color, StyleError> {
    let rgb = color
        .to_rgb()
        .map_err(|_| StyleError::UnsupportedValue(format!("{color:?}")))?;

    match rgb {
        CssColor::RGBA(rgba) => Ok(Color::rgba(rgba.red, rgba.green, rgba.blue, rgba.alpha)),
        _ => Err(StyleError::UnsupportedValue(format!("{color:?}"))),
    }
}

fn px_from_size(size: &Size) -> Result<f32, StyleError> {
    match size {
        Size::LengthPercentage(value) => px_from_length_percentage(value),
        _ => Err(StyleError::UnsupportedValue(format!("{size:?}"))),
    }
}

fn px_from_inset(value: &LengthPercentageOrAuto) -> Result<f32, StyleError> {
    match value {
        LengthPercentageOrAuto::LengthPercentage(value) => px_from_length_percentage(value),
        LengthPercentageOrAuto::Auto => Err(StyleError::UnsupportedValue("auto".to_string())),
    }
}

fn px_from_length_percentage(value: &LengthPercentage) -> Result<f32, StyleError> {
    match value {
        LengthPercentage::Dimension(length) => length
            .to_px()
            .map(|value| value as f32)
            .ok_or_else(|| StyleError::UnsupportedValue(format!("{value:?}"))),
        _ => Err(StyleError::UnsupportedValue(format!("{value:?}"))),
    }
}

fn element_to_render_node(element: &ElementNode, stylesheet: &Stylesheet) -> RenderNode {
    let computed = compute_style(element, stylesheet);
    let layout = LayoutBox::new(
        computed.x.unwrap_or(0.0),
        computed.y.unwrap_or(0.0),
        computed.width.unwrap_or(0.0),
        computed.height.unwrap_or(0.0),
    );
    let visual = VisualStyle {
        background: computed.background,
        foreground: computed.foreground.unwrap_or(Color::BLACK),
        ..VisualStyle::default()
    };
    let child_elements: Vec<_> = element
        .children
        .iter()
        .filter_map(|child| match child {
            Node::Element(child) => Some(element_to_render_node(child, stylesheet)),
            Node::Text(_) => None,
        })
        .collect();
    let text = element_text(element);

    if child_elements.is_empty() && !text.is_empty() {
        RenderNode::text(layout, text).with_style(visual)
    } else {
        RenderNode::container(layout)
            .with_style(visual)
            .with_children(child_elements)
    }
}

fn compute_style(element: &ElementNode, stylesheet: &Stylesheet) -> ComputedStyle {
    let mut computed = ComputedStyle::default();
    let element_ref = ElementRef {
        tag: &element.tag,
        id: element.id.as_deref(),
        classes: &element.classes,
    };

    for rule in stylesheet.matching_rules(element_ref) {
        for declaration in &rule.declarations {
            computed.apply(*declaration);
        }
    }

    computed
}

fn element_text(element: &ElementNode) -> String {
    let mut content = String::new();

    for child in &element.children {
        collect_text(child, &mut content);
    }

    content
}

fn collect_text(node: &Node, buffer: &mut String) {
    match node {
        Node::Text(content) => buffer.push_str(content),
        Node::Element(element) => {
            for child in &element.children {
                collect_text(child, buffer);
            }
        }
    }
}

#[derive(Debug, Default)]
struct ComputedStyle {
    background: Option<Color>,
    foreground: Option<Color>,
    x: Option<f32>,
    y: Option<f32>,
    width: Option<f32>,
    height: Option<f32>,
}

impl ComputedStyle {
    fn apply(&mut self, declaration: Declaration) {
        match declaration {
            Declaration::Background(color) => self.background = Some(color),
            Declaration::Foreground(color) => self.foreground = Some(color),
            Declaration::Left(value) => self.x = Some(value),
            Declaration::Top(value) => self.y = Some(value),
            Declaration::Width(value) => self.width = Some(value),
            Declaration::Height(value) => self.height = Some(value),
        }
    }
}

#[cfg(test)]
mod tests {
    use cssimpler_core::{Color, Node};

    use super::{
        Declaration, ElementRef, Selector, StyleRule, Stylesheet, build_render_tree,
        parse_stylesheet,
    };

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
            "#app { width: 960px; height: 540px; background-color: #e2e8f0; }
             .card { left: 160px; top: 128px; width: 640px; height: 220px; background-color: #0f172a; color: #ffffff; }",
        )
        .expect("bootstrap stylesheet should parse");

        assert_eq!(stylesheet.rules.len(), 2);
        assert!(
            stylesheet.rules[0]
                .declarations
                .contains(&Declaration::Background(Color::rgb(226, 232, 240)))
        );
        assert!(
            stylesheet.rules[1]
                .declarations
                .contains(&Declaration::Left(160.0))
        );
    }

    #[test]
    fn render_tree_builder_resolves_styles_outside_the_app() {
        let stylesheet = parse_stylesheet(
            "#app { width: 100px; height: 80px; background-color: #ffffff; }
             .title { left: 10px; top: 12px; width: 40px; height: 16px; color: #0f172a; }",
        )
        .expect("stylesheet should parse");
        let tree = Node::element("div")
            .with_id("app")
            .with_child(
                Node::element("h1")
                    .with_class("title")
                    .with_child(Node::text("hello"))
                    .into(),
            )
            .into();
        let scene = build_render_tree(&tree, &stylesheet);

        assert_eq!(scene.layout.width, 100.0);
        assert_eq!(scene.children.len(), 1);
        assert_eq!(scene.children[0].layout.x, 10.0);
    }
}
