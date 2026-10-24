//! Viewport media queries. Unknown features remain nonmatching instead of
//! making their enclosed declarations apply unconditionally.
use lightningcss::media_query::{
    MediaCondition, MediaFeature, MediaFeatureComparison, MediaFeatureId, MediaFeatureName,
    MediaFeatureValue, MediaList, MediaType, Operator, Qualifier, QueryFeature,
};
use lightningcss::values::length::{Length, LengthValue};

#[derive(Clone, Debug)]
pub(crate) enum Condition {
    Always,
    Never,
    Not(Box<Self>),
    All(Vec<Self>),
    Any(Vec<Self>),
    Dimension {
        width: bool,
        operator: MediaFeatureComparison,
        value: f32,
    },
    Landscape(bool),
}
impl Condition {
    pub fn from_list(list: &MediaList<'_>) -> Self {
        if list.media_queries.is_empty() {
            return Self::Always;
        }
        Self::Any(
            list.media_queries
                .iter()
                .map(|q| {
                    let mut value = if matches!(q.media_type, MediaType::All | MediaType::Screen) {
                        q.condition
                            .as_ref()
                            .map_or(Self::Always, Self::from_condition)
                    } else {
                        Self::Never
                    };
                    if q.qualifier == Some(Qualifier::Not) {
                        value = Self::Not(Box::new(value));
                    }
                    value
                })
                .collect(),
        )
    }
    fn from_condition(condition: &MediaCondition<'_>) -> Self {
        match condition {
            MediaCondition::Feature(f) => Self::from_feature(f),
            MediaCondition::Not(c) => Self::Not(Box::new(Self::from_condition(c))),
            MediaCondition::Operation {
                operator,
                conditions,
            } => {
                let children = conditions.iter().map(Self::from_condition).collect();
                if *operator == Operator::And {
                    Self::All(children)
                } else {
                    Self::Any(children)
                }
            }
            _ => Self::Never,
        }
    }
    fn from_feature(feature: &MediaFeature<'_>) -> Self {
        use QueryFeature::*;
        match feature {
            Plain { name, value } => Self::comparison(name, MediaFeatureComparison::Equal, value),
            Range {
                name,
                operator,
                value,
            } => Self::comparison(name, *operator, value),
            Interval {
                name,
                start,
                start_operator,
                end,
                end_operator,
            } => Self::All(vec![
                Self::comparison(name, reverse(*start_operator), start),
                Self::comparison(name, *end_operator, end),
            ]),
            Boolean {
                name: MediaFeatureName::Standard(MediaFeatureId::Width | MediaFeatureId::Height),
            } => Self::Always,
            _ => Self::Never,
        }
    }
    fn comparison(
        name: &MediaFeatureName<'_, MediaFeatureId>,
        operator: MediaFeatureComparison,
        value: &MediaFeatureValue<'_>,
    ) -> Self {
        match (name, value) {
            (
                MediaFeatureName::Standard(id @ (MediaFeatureId::Width | MediaFeatureId::Height)),
                MediaFeatureValue::Length(length),
            ) => {
                let px = match length {
                    Length::Value(LengthValue::Em(v) | LengthValue::Rem(v)) => Some(v * 16.0),
                    _ => length.to_px(),
                };
                px.map_or(Self::Never, |value| Self::Dimension {
                    width: *id == MediaFeatureId::Width,
                    operator,
                    value,
                })
            }
            (
                MediaFeatureName::Standard(MediaFeatureId::Orientation),
                MediaFeatureValue::Ident(value),
            ) if operator == MediaFeatureComparison::Equal => match value.as_ref() {
                "landscape" => Self::Landscape(true),
                "portrait" => Self::Landscape(false),
                _ => Self::Never,
            },
            _ => Self::Never,
        }
    }
    pub fn matches(&self, width: usize, height: usize) -> bool {
        match self {
            Self::Always => true,
            Self::Never => false,
            Self::Not(c) => !c.matches(width, height),
            Self::All(cs) => cs.iter().all(|c| c.matches(width, height)),
            Self::Any(cs) => cs.iter().any(|c| c.matches(width, height)),
            Self::Landscape(value) => (width > height) == *value,
            Self::Dimension {
                width: is_width,
                operator,
                value,
            } => {
                let actual = if *is_width { width } else { height } as f32;
                match operator {
                    MediaFeatureComparison::Equal => actual == *value,
                    MediaFeatureComparison::GreaterThan => actual > *value,
                    MediaFeatureComparison::GreaterThanEqual => actual >= *value,
                    MediaFeatureComparison::LessThan => actual < *value,
                    MediaFeatureComparison::LessThanEqual => actual <= *value,
                }
            }
        }
    }
}
fn reverse(op: MediaFeatureComparison) -> MediaFeatureComparison {
    use MediaFeatureComparison::*;
    match op {
        Equal => Equal,
        GreaterThan => LessThan,
        GreaterThanEqual => LessThanEqual,
        LessThan => GreaterThan,
        LessThanEqual => GreaterThanEqual,
    }
}

#[cfg(test)]
mod tests {
    use crate::{parse_stylesheet, parse_stylesheet_lossy};
    #[test]
    fn viewport_queries_filter_width_height_ranges_and_nested_conditions() {
        let css = ".card { width: 200px; } @media (max-width: 600px) { .card { width: 100px; } @media (min-height: 700px) { .card { height: 80px; } } }";
        let sheet = parse_stylesheet(css).unwrap();
        assert_eq!(sheet.for_viewport(500, 800).rules.len(), 3);
        assert_eq!(sheet.for_viewport(500, 600).rules.len(), 2);
        assert_eq!(sheet.for_viewport(900, 800).rules.len(), 1);
        let sheet=parse_stylesheet("@media (400px < width <= 800px) and (orientation: landscape) { .card { width: 100px; } }").unwrap();
        assert_eq!(sheet.for_viewport(600, 400).rules.len(), 1);
        assert_eq!(sheet.for_viewport(900, 400).rules.len(), 0);
        assert_eq!(sheet.for_viewport(600, 800).rules.len(), 0);
    }
    #[test]
    fn screen_print_negation_and_unknown_features_are_not_flattened() {
        let sheet=parse_stylesheet("@media screen { .a { width: 1px; } } @media print { .b { width: 1px; } } @media not screen { .c { width: 1px; } } @media (unknown-feature: yes) { .d { width: 1px; } }").unwrap();
        assert_eq!(sheet.for_viewport(600, 400).rules.len(), 1);
        let sheet =
            parse_stylesheet_lossy("@media (max-width: 600px) { .a { width: 1px; } }").unwrap();
        assert_eq!(sheet.for_viewport(500, 400).rules.len(), 1);
        assert_eq!(sheet.for_viewport(900, 400).rules.len(), 0);
    }
}
