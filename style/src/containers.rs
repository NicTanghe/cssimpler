use cssimpler_core::{ElementInteractionState, ElementPath};
use lightningcss::properties::{Property, contain::ContainerNameList};

use crate::{Declaration, ElementRef, StyleRule, Stylesheet};

pub(crate) fn extract_names(property: &Property<'_>) -> Option<Declaration> {
    let list = match property {
        Property::ContainerName(names) => names,
        Property::Container(container) => &container.name,
        _ => return None,
    };
    let names = match list {
        ContainerNameList::None => Vec::new(),
        ContainerNameList::Names(names) => names.iter().map(|name| name.0.0.to_string()).collect(),
    };
    Some(Declaration::ContainerNames(names))
}

pub(crate) fn required_names_match(rule: &StyleRule, names: &[&str]) -> bool {
    rule.container_names
        .iter()
        .all(|name| names.contains(&name.as_str()))
}

impl Stylesheet {
    pub(crate) fn ancestor_container_names<'a>(
        &'a self,
        ancestors: &[ElementRef<'_>],
        path: &ElementPath,
        interaction: &ElementInteractionState,
    ) -> Vec<&'a str> {
        // Generated pseudo-elements may include their originating element at
        // the front. Container queries use the originating element's ancestors.
        let ancestors = &ancestors[ancestors.len().saturating_sub(path.children.len())..];
        let mut names = Vec::new();
        for (index, &ancestor) in ancestors.iter().enumerate().rev() {
            let Some(ancestor_path) = path.ancestor(index + 1) else {
                continue;
            };
            let winning_names =
                self.index
                    .container_name_rules
                    .iter()
                    .rev()
                    .find_map(|&rule_index| {
                        let Some(rule) = self.rules.get(rule_index) else {
                            return None;
                        };
                        let declared_names = rule.declarations.iter().rev().find_map(
                            |declaration| match declaration {
                                Declaration::ContainerNames(names) => Some(names),
                                _ => None,
                            },
                        )?;
                        (required_names_match(rule, &names)
                            && rule.selector.matches_with_ancestors_interaction_and_pseudo(
                                ancestor,
                                &ancestors[index + 1..],
                                &ancestor_path,
                                interaction,
                                None,
                            ))
                        .then_some(declared_names)
                    });
            if let Some(winning_names) = winning_names {
                names.extend(winning_names.iter().map(String::as_str));
            }
        }
        names
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{parse_stylesheet, parse_stylesheet_lossy};
    use cssimpler_core::ElementNode;

    #[test]
    fn named_queries_require_a_matching_ancestor_and_skip_unknown_conditions() {
        let source = "
            panel { container: workspace / normal; }
            group { container-name: details alternate; }
            .reset { container-name: none; }
            @container workspace {
                label { color: red; }
                @container details { label { background: blue; } }
            }
            @container details scroll-state(scrollable: bottom) {
                label { color: green; }
            }
        ";
        for sheet in [
            parse_stylesheet(source).unwrap(),
            parse_stylesheet_lossy(source).unwrap(),
        ] {
            let panel = ElementNode::new("panel");
            let group = ElementNode::new("group");
            let label = ElementNode::new("label");
            let path = ElementPath::root(0).with_child(0).with_child(0);
            let interaction = ElementInteractionState::default();
            let matches = |ancestors: &[ElementRef<'_>]| {
                sheet
                    .matching_rules_with_context(
                        ElementRef::from(&label),
                        ancestors,
                        &path,
                        &interaction,
                    )
                    .count()
            };
            assert_eq!(matches(&[]), 0);
            assert_eq!(matches(&[ElementRef::from(&panel)]), 1);
            assert_eq!(
                matches(&[ElementRef::from(&group), ElementRef::from(&panel)]),
                2
            );
            let group = group.with_class("reset");
            assert_eq!(
                matches(&[ElementRef::from(&group), ElementRef::from(&panel)]),
                1
            );
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Scrollable {
    None,
    Any,
    Left,
    Right,
    Top,
    Bottom,
    X,
    Y,
}

impl Scrollable {
    fn matches(self, directions: cssimpler_core::ScrollDirections) -> bool {
        let horizontal = directions.left || directions.right;
        let vertical = directions.up || directions.down;
        match self {
            Self::None => !horizontal && !vertical,
            Self::Any => horizontal || vertical,
            Self::Left => directions.left,
            Self::Right => directions.right,
            Self::Top => directions.up,
            Self::Bottom => directions.down,
            Self::X => horizontal,
            Self::Y => vertical,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ScrollQuery {
    name: Option<String>,
    condition: ScrollCondition,
}

#[derive(Clone, Debug)]
enum ScrollCondition {
    Feature(Scrollable),
    Not(Box<Self>),
    All(Vec<Self>),
    Any(Vec<Self>),
}

impl ScrollCondition {
    fn from_container(
        condition: &lightningcss::rules::container::ContainerCondition<'_>,
    ) -> Option<Self> {
        use lightningcss::media_query::Operator;
        use lightningcss::rules::container::ContainerCondition;
        Some(match condition {
            ContainerCondition::ScrollState(query) => return Self::from_scroll_state(query),
            ContainerCondition::Not(inner) => Self::Not(Box::new(Self::from_container(inner)?)),
            ContainerCondition::Operation {
                operator,
                conditions,
            } => {
                let conditions = conditions
                    .iter()
                    .map(Self::from_container)
                    .collect::<Option<Vec<_>>>()?;
                match operator {
                    Operator::And => Self::All(conditions),
                    Operator::Or => Self::Any(conditions),
                }
            }
            _ => return None,
        })
    }

    fn from_scroll_state(
        query: &lightningcss::rules::container::ScrollStateQuery<'_>,
    ) -> Option<Self> {
        use lightningcss::media_query::{
            MediaFeatureName, MediaFeatureValue, Operator, QueryFeature,
        };
        use lightningcss::rules::container::{ScrollStateFeatureId, ScrollStateQuery};
        let feature = match query {
            ScrollStateQuery::Feature(feature) => feature,
            ScrollStateQuery::Not(inner) => {
                return Some(Self::Not(Box::new(Self::from_scroll_state(inner)?)));
            }
            ScrollStateQuery::Operation {
                operator,
                conditions,
            } => {
                let conditions = conditions
                    .iter()
                    .map(Self::from_scroll_state)
                    .collect::<Option<Vec<_>>>()?;
                return Some(match operator {
                    Operator::And => Self::All(conditions),
                    Operator::Or => Self::Any(conditions),
                });
            }
        };
        let scrollable = match feature {
            QueryFeature::Boolean {
                name: MediaFeatureName::Standard(ScrollStateFeatureId::Scrollable),
            } => Scrollable::Any,
            QueryFeature::Plain {
                name: MediaFeatureName::Standard(ScrollStateFeatureId::Scrollable),
                value: MediaFeatureValue::Ident(ident),
            } => match ident.0.to_ascii_lowercase().as_str() {
                "none" => Scrollable::None,
                "left" => Scrollable::Left,
                "right" => Scrollable::Right,
                "top" => Scrollable::Top,
                "bottom" => Scrollable::Bottom,
                "x" | "inline" => Scrollable::X,
                "y" | "block" => Scrollable::Y,
                _ => return None,
            },
            _ => return None,
        };
        Some(Self::Feature(scrollable))
    }

    fn matches(&self, directions: cssimpler_core::ScrollDirections) -> bool {
        match self {
            Self::Feature(feature) => feature.matches(directions),
            Self::Not(condition) => !condition.matches(directions),
            Self::All(conditions) => conditions
                .iter()
                .all(|condition| condition.matches(directions)),
            Self::Any(conditions) => conditions
                .iter()
                .any(|condition| condition.matches(directions)),
        }
    }
}

impl ScrollQuery {
    pub(crate) fn from_rule(
        rule: &lightningcss::rules::container::ContainerRule<'_, impl Clone>,
    ) -> Option<Self> {
        Some(Self {
            name: rule.name.as_ref().map(|name| name.0.0.to_string()),
            condition: ScrollCondition::from_container(rule.condition.as_ref()?)?,
        })
    }

    pub(crate) fn matches(
        &self,
        state: &[cssimpler_core::ScrollState],
        path: &ElementPath,
    ) -> bool {
        state
            .iter()
            .filter(|entry| {
                entry.query_container
                    && entry.element.children.len() < path.children.len()
                    && entry.element.is_prefix_of(path)
                    && self
                        .name
                        .as_ref()
                        .is_none_or(|name| entry.names.contains(name))
            })
            .max_by_key(|entry| entry.element.children.len())
            .is_some_and(|entry| self.condition.matches(entry.scrollable))
    }
}
