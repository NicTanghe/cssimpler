use std::collections::{BTreeMap, HashSet};

use cssimpler_core::{CustomProperties, Style};
use cssparser::{Parser, ParserInput};
use lightningcss::printer::PrinterOptions;
use lightningcss::properties::Property;
use lightningcss::rules::{CssRule, CssRuleList, property::PropertyRule};
use lightningcss::traits::ToCss;
use lightningcss::values::syntax::SyntaxString;

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

#[derive(Clone, Debug)]
pub(crate) struct Registration {
    name: String,
    syntax: SyntaxString,
    inherits: bool,
    initial: Option<String>,
    pub(crate) media_conditions: Vec<crate::media::Condition>,
}

impl Registration {
    pub(crate) fn from_rule(rule: &PropertyRule<'_>) -> Result<Self, StyleError> {
        let initial = rule
            .initial_value
            .as_ref()
            .map(|value| {
                value
                    .to_css_string(PrinterOptions::default())
                    .map_err(|error| StyleError::UnsupportedValue(error.to_string()))
            })
            .transpose()?;
        Ok(Self {
            name: rule.name.0.to_string(),
            syntax: rule.syntax.clone(),
            inherits: rule.inherits,
            initial,
            media_conditions: Vec::new(),
        })
    }

    fn accepts(&self, value: &str) -> bool {
        let mut input = ParserInput::new(value);
        let mut parser = Parser::new(&mut input);
        self.syntax.parse_value(&mut parser).is_ok() && parser.is_exhausted()
    }
}

pub(crate) fn collect_registrations(rules: &CssRuleList<'_, impl Clone>) -> Vec<Registration> {
    fn collect<R: Clone>(
        rules: &CssRuleList<'_, R>,
        conditions: &mut Vec<crate::media::Condition>,
        output: &mut Vec<Registration>,
    ) {
        for rule in &rules.0 {
            match rule {
                CssRule::Property(rule) => {
                    if let Ok(mut registration) = Registration::from_rule(rule) {
                        registration.media_conditions = conditions.clone();
                        output.push(registration);
                    }
                }
                CssRule::Media(media) => {
                    conditions.push(crate::media::Condition::from_list(&media.query));
                    collect(&media.rules, conditions, output);
                    conditions.pop();
                }
                CssRule::LayerBlock(layer) => collect(&layer.rules, conditions, output),
                _ => {}
            }
        }
    }

    let mut registrations = Vec::new();
    collect(rules, &mut Vec::new(), &mut registrations);
    registrations
}

pub(crate) fn compute_registered_values(
    properties: &mut CustomProperties,
    inherited: Option<&CustomProperties>,
    registrations: &[Registration],
) {
    let mut registrations_by_name = std::collections::BTreeMap::new();
    for registration in registrations
        .iter()
        .filter(|entry| entry.media_conditions.is_empty())
    {
        registrations_by_name.insert(registration.name.as_str(), registration);
    }
    if registrations_by_name.is_empty() {
        return;
    }

    let specified = properties.clone();
    let mut evaluator = RegisteredValueEvaluator {
        specified: &specified,
        inherited,
        registrations: &registrations_by_name,
        values: properties,
        stack: Vec::new(),
        settled: HashSet::new(),
        cyclic: HashSet::new(),
    };
    for &name in registrations_by_name.keys() {
        evaluator.resolve(name);
    }
}

struct RegisteredValueEvaluator<'a> {
    specified: &'a CustomProperties,
    inherited: Option<&'a CustomProperties>,
    registrations: &'a BTreeMap<&'a str, &'a Registration>,
    values: &'a mut CustomProperties,
    stack: Vec<String>,
    settled: HashSet<String>,
    cyclic: HashSet<String>,
}

impl RegisteredValueEvaluator<'_> {
    fn resolve(&mut self, name: &str) {
        if self.settled.contains(name) {
            return;
        }
        if let Some(start) = self.stack.iter().position(|member| member == name) {
            self.cyclic.extend(self.stack[start..].iter().cloned());
            return;
        }

        let specified = self.specified.get(name);
        self.stack.push(name.to_owned());
        if let Some(value) = specified {
            for dependency in crate::variable_resolution::custom_property_dependencies(name, value)
            {
                self.resolve(&dependency);
            }
        }
        self.stack.pop();

        if let Some(registration) = self.registrations.get(name) {
            let parent_value = self.inherited.and_then(|properties| properties.get(name));
            let inherited_or_initial = parent_value.or(registration.initial.as_deref());
            let fallback = if registration.inherits {
                inherited_or_initial
            } else {
                registration.initial.as_deref()
            };
            let computed = match specified {
                Some(value) if value.eq_ignore_ascii_case("initial") => {
                    registration.initial.clone()
                }
                Some(value) if value.eq_ignore_ascii_case("inherit") => {
                    inherited_or_initial.map(str::to_owned)
                }
                Some(value)
                    if !value.eq_ignore_ascii_case("unset") && !self.cyclic.contains(name) =>
                {
                    crate::variable_resolution::resolve_custom_property_value(
                        name,
                        value,
                        self.values,
                    )
                    .ok()
                    .filter(|value| registration.accepts(value))
                    .or_else(|| fallback.map(str::to_owned))
                }
                _ => fallback.map(str::to_owned),
            };
            if let Some(value) = computed {
                self.values.set(name, value);
            } else {
                self.values.remove(name);
            }
        } else if self.cyclic.contains(name) {
            self.values.remove(name);
        }
        self.settled.insert(name.to_owned());
    }
}

pub(crate) fn inherit_with_registrations(
    style: &mut Style,
    inherited: &CustomProperties,
    registrations: &[Registration],
) {
    for (name, value) in inherited.iter() {
        let inherits = registrations
            .iter()
            .rev()
            .find(|entry| entry.name == name && entry.media_conditions.is_empty())
            .is_none_or(|entry| entry.inherits);
        if inherits && style.custom_properties.get(name).is_none() {
            style.custom_properties.set(name, value);
        }
    }
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

#[cfg(test)]
mod registration_tests {
    use crate::{parse_stylesheet, parse_stylesheet_lossy, resolve_style};
    use cssimpler_core::{ElementNode, GeneratedTextSource, Node};

    #[test]
    fn nested_string_registration_validates_values_and_uses_initial_value() {
        let css = r#"
            @media (min-width: 400px) {
                @layer controls {
                    @property --caption { syntax: "<string>"; inherits: false; initial-value: "Idle"; }
                }
            }
            .caption { content: var(--caption, "Fallback"); }
            .invalid { --caption: 42; }
            .valid { --source: "Ready"; --caption: var(--source); }
        "#;
        for sheet in [
            parse_stylesheet(css).unwrap(),
            parse_stylesheet_lossy(css).unwrap(),
        ] {
            let caption = ElementNode::new("label").with_class("caption");
            let text = |element: &ElementNode, width| {
                resolve_style(element, &sheet.for_viewport(width, 800)).generated_text
            };
            assert_eq!(
                text(&caption, 300),
                Some(GeneratedTextSource::Literal("Fallback".into()))
            );
            assert_eq!(
                text(&caption, 500),
                Some(GeneratedTextSource::Literal("Idle".into()))
            );
            assert_eq!(
                text(&caption.clone().with_class("invalid"), 500),
                Some(GeneratedTextSource::Literal("Idle".into()))
            );
            assert_eq!(
                text(&caption.with_class("valid"), 500),
                Some(GeneratedTextSource::Literal("Ready".into()))
            );
        }
    }

    #[test]
    fn registered_dependencies_use_validated_values_and_cycles_use_initial_values() {
        let sheet = parse_stylesheet(
            r#"
            @property --a { syntax: "<string>"; inherits: false; initial-value: "A"; }
            @property --b { syntax: "<string>"; inherits: false; initial-value: "B"; }
            label { --a: var(--alias); --alias: var(--b); --b: 18; content: var(--a); }
            .cycle { --b: var(--a); }
        "#,
        )
        .unwrap();
        let label = ElementNode::new("label");
        let plain = resolve_style(&label, &sheet);
        assert_eq!(
            plain.generated_text,
            Some(GeneratedTextSource::Literal("B".into()))
        );
        let cycle = resolve_style(&label.with_class("cycle"), &sheet);
        assert_eq!(
            cycle.generated_text,
            Some(GeneratedTextSource::Literal("A".into()))
        );
        assert_eq!(cycle.custom_properties.get("--b"), Some("\"B\""));
    }

    #[test]
    fn noninheriting_registration_starts_fresh_on_each_element() {
        let sheet = parse_stylesheet(
            r#"
            @property --tone { syntax: "<color>"; inherits: false; initial-value: red; }
            panel { --tone: blue; }
            label { color: var(--tone); }
        "#,
        )
        .unwrap();
        let tree = Node::from(
            ElementNode::new("panel").with_child(
                ElementNode::new("label")
                    .with_id("label")
                    .with_child(Node::text("test"))
                    .into(),
            ),
        );
        let rendered = crate::build_render_tree(&tree, &sheet);
        assert_eq!(
            rendered.children[0].style.foreground,
            cssimpler_core::Color::rgb(255, 0, 0)
        );
    }
}
