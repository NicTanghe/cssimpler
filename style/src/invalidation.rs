use cssimpler_core::ElementInteractionState;

use crate::selectors::InteractionDependencies;
use crate::{Declaration, StyleRule, Stylesheet};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum StyleInvalidation {
    #[default]
    Clean,
    Paint,
    Layout,
    Structure,
}

impl StyleInvalidation {
    const fn merge(self, other: Self) -> Self {
        if (self as u8) >= (other as u8) {
            self
        } else {
            other
        }
    }
}

impl Stylesheet {
    pub fn interaction_invalidation(
        &self,
        previous: &ElementInteractionState,
        next: &ElementInteractionState,
    ) -> StyleInvalidation {
        let changed = changed_interaction(previous, next);
        if changed.is_empty() {
            return StyleInvalidation::Clean;
        }

        self.index
            .collect_interaction_rule_indices(changed)
            .into_iter()
            .fold(StyleInvalidation::Clean, |invalidation, index| {
                let Some(rule) = self.rules.get(index) else {
                    return invalidation;
                };
                invalidation.merge(rule.interaction_invalidation(changed))
            })
    }
}

impl StyleRule {
    fn interaction_invalidation(&self, changed: InteractionDependencies) -> StyleInvalidation {
        if !self.interaction_dependencies().intersects(changed) {
            return StyleInvalidation::Clean;
        }

        self.declarations
            .iter()
            .fold(StyleInvalidation::Clean, |invalidation, declaration| {
                invalidation.merge(declaration_invalidation(declaration))
            })
    }
}

fn changed_interaction(
    previous: &ElementInteractionState,
    next: &ElementInteractionState,
) -> InteractionDependencies {
    InteractionDependencies {
        hover: previous.hovered != next.hovered,
        active: previous.active != next.active,
    }
}

fn declaration_invalidation(declaration: &Declaration) -> StyleInvalidation {
    match declaration {
        Declaration::Content(_) => StyleInvalidation::Structure,
        Declaration::CustomProperty { .. }
        | Declaration::TransitionProperties(_)
        | Declaration::TransitionDurations(_)
        | Declaration::TransitionDelays(_)
        | Declaration::TransitionTimingFunctions(_) => StyleInvalidation::Clean,
        Declaration::VariableDependentProperty { property_name, .. } => {
            variable_property_invalidation(property_name)
        }
        Declaration::Background(_)
        | Declaration::BackgroundLayers(_)
        | Declaration::Foreground(_)
        | Declaration::NativeMaterial(_)
        | Declaration::GlassTint(_)
        | Declaration::SvgFill(_)
        | Declaration::SvgStroke(_)
        | Declaration::SvgStrokeWidth(_)
        | Declaration::CornerTopLeft(_)
        | Declaration::CornerTopRight(_)
        | Declaration::CornerBottomRight(_)
        | Declaration::CornerBottomLeft(_)
        | Declaration::BorderTopWidth(_)
        | Declaration::BorderRightWidth(_)
        | Declaration::BorderBottomWidth(_)
        | Declaration::BorderLeftWidth(_)
        | Declaration::BorderLineStyle(_)
        | Declaration::BorderColor(_)
        | Declaration::BoxShadows(_)
        | Declaration::TextShadows(_)
        | Declaration::FilterDropShadows(_)
        | Declaration::BackdropBlur(_)
        | Declaration::TextStrokeWidth(_)
        | Declaration::TextStrokeColor(_)
        | Declaration::TransformOperations(_)
        | Declaration::TransformOrigin(_)
        | Declaration::Perspective(_)
        | Declaration::TransformStyle(_)
        | Declaration::ScrollbarColors(_, _) => StyleInvalidation::Paint,
        Declaration::FontFamilies(_)
        | Declaration::FontSize(_)
        | Declaration::FontWeight(_)
        | Declaration::FontStyle(_)
        | Declaration::LineHeight(_)
        | Declaration::LetterSpacing(_)
        | Declaration::TextTransform(_)
        | Declaration::OverflowX(_)
        | Declaration::OverflowY(_)
        | Declaration::ScrollbarWidth(_)
        | Declaration::Position(_)
        | Declaration::InsetTop(_)
        | Declaration::InsetTopCalc(_)
        | Declaration::InsetRight(_)
        | Declaration::InsetRightCalc(_)
        | Declaration::InsetBottom(_)
        | Declaration::InsetBottomCalc(_)
        | Declaration::InsetLeft(_)
        | Declaration::InsetLeftCalc(_)
        | Declaration::Width(_)
        | Declaration::WidthCalc(_)
        | Declaration::Height(_)
        | Declaration::HeightCalc(_)
        | Declaration::MinWidth(_)
        | Declaration::MinWidthCalc(_)
        | Declaration::MinHeight(_)
        | Declaration::MinHeightCalc(_)
        | Declaration::MaxWidth(_)
        | Declaration::MaxWidthCalc(_)
        | Declaration::MaxHeight(_)
        | Declaration::MaxHeightCalc(_)
        | Declaration::MarginTop(_)
        | Declaration::MarginTopCalc(_)
        | Declaration::MarginRight(_)
        | Declaration::MarginRightCalc(_)
        | Declaration::MarginBottom(_)
        | Declaration::MarginBottomCalc(_)
        | Declaration::MarginLeft(_)
        | Declaration::MarginLeftCalc(_)
        | Declaration::PaddingTop(_)
        | Declaration::PaddingTopCalc(_)
        | Declaration::PaddingRight(_)
        | Declaration::PaddingRightCalc(_)
        | Declaration::PaddingBottom(_)
        | Declaration::PaddingBottomCalc(_)
        | Declaration::PaddingLeft(_)
        | Declaration::PaddingLeftCalc(_)
        | Declaration::FlexDirection(_)
        | Declaration::FlexWrap(_)
        | Declaration::JustifyContent(_)
        | Declaration::AlignItems(_)
        | Declaration::AlignSelf(_)
        | Declaration::AlignContent(_)
        | Declaration::GapRow(_)
        | Declaration::GapRowCalc(_)
        | Declaration::GapColumn(_)
        | Declaration::GapColumnCalc(_)
        | Declaration::FlexGrow(_)
        | Declaration::FlexShrink(_)
        | Declaration::FlexBasis(_)
        | Declaration::GridTemplateColumns(_)
        | Declaration::GridTemplateRows(_)
        | Declaration::GridColumn(_)
        | Declaration::GridRow(_)
        | Declaration::GridColumnStart(_)
        | Declaration::GridColumnEnd(_)
        | Declaration::GridRowStart(_)
        | Declaration::GridRowEnd(_) => StyleInvalidation::Layout,
        Declaration::Display(_) => StyleInvalidation::Structure,
    }
}

fn variable_property_invalidation(property_name: &str) -> StyleInvalidation {
    match property_name {
        "background"
        | "background-color"
        | "background-image"
        | "color"
        | "fill"
        | "stroke"
        | "stroke-width"
        | "border-color"
        | "border-top-color"
        | "border-right-color"
        | "border-bottom-color"
        | "border-left-color"
        | "box-shadow"
        | "text-shadow"
        | "filter"
        | "backdrop-filter"
        | "native-material"
        | "glass-tint"
        | "-webkit-backdrop-filter"
        | "-webkit-text-stroke"
        | "-webkit-text-stroke-width"
        | "-webkit-text-stroke-color"
        | "scrollbar-color"
        | "border-radius"
        | "border-top-left-radius"
        | "border-top-right-radius"
        | "border-bottom-right-radius"
        | "border-bottom-left-radius"
        | "transform"
        | "transform-origin"
        | "transform-style"
        | "perspective"
        | "translate"
        | "rotate"
        | "scale" => StyleInvalidation::Paint,
        "display" => StyleInvalidation::Structure,
        _ => StyleInvalidation::Layout,
    }
}

#[cfg(test)]
mod tests {
    use cssimpler_core::{ElementInteractionState, ElementPath};

    use crate::{StyleInvalidation, parse_stylesheet};

    #[test]
    fn interaction_invalidation_is_clean_without_interactive_selectors() {
        let stylesheet =
            parse_stylesheet(".button { color: #2563eb; }").expect("stylesheet should parse");

        let invalidation = stylesheet.interaction_invalidation(
            &ElementInteractionState::default(),
            &ElementInteractionState {
                hovered: Some(ElementPath::root(0)),
                active: None,
            },
        );

        assert_eq!(invalidation, StyleInvalidation::Clean);
    }

    #[test]
    fn interaction_invalidation_tracks_hover_layout_rules() {
        let stylesheet =
            parse_stylesheet(".button:hover { width: 120px; }").expect("stylesheet should parse");

        let invalidation = stylesheet.interaction_invalidation(
            &ElementInteractionState::default(),
            &ElementInteractionState {
                hovered: Some(ElementPath::root(0)),
                active: None,
            },
        );

        assert_eq!(invalidation, StyleInvalidation::Layout);
    }

    #[test]
    fn interaction_invalidation_ignores_unchanged_pseudo_classes() {
        let stylesheet =
            parse_stylesheet(".button:active { width: 120px; }").expect("stylesheet should parse");

        let invalidation = stylesheet.interaction_invalidation(
            &ElementInteractionState::default(),
            &ElementInteractionState {
                hovered: Some(ElementPath::root(0)),
                active: None,
            },
        );

        assert_eq!(invalidation, StyleInvalidation::Clean);
    }

    #[test]
    fn interaction_invalidation_sees_ancestor_hover_selectors() {
        let stylesheet = parse_stylesheet(".button:hover .hover-text { color: #2563eb; }")
            .expect("stylesheet should parse");

        let invalidation = stylesheet.interaction_invalidation(
            &ElementInteractionState::default(),
            &ElementInteractionState {
                hovered: Some(ElementPath::root(0).with_child(0)),
                active: None,
            },
        );

        assert_eq!(invalidation, StyleInvalidation::Paint);
    }
}
