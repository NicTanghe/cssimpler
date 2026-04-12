use cssimpler_core::{Color, OverflowMode};
use taffy::prelude::{
    AlignContent as TaffyAlignContent, AlignItems as TaffyAlignItems, AlignSelf as TaffyAlignSelf,
    Dimension, Display as TaffyDisplay, FlexDirection, FlexWrap,
    JustifyContent as TaffyJustifyContent, LengthPercentage as TaffyLengthPercentage,
    LengthPercentageAuto as TaffyLengthPercentageAuto, Position as TaffyPosition,
};

use crate::{
    Declaration, PseudoElementKind, Selector, SelectorCombinator, SimpleSelector, StyleRule,
    Stylesheet,
    selectors::{AncestorSelector, CompoundSelector},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StaticSimpleSelectorDesc {
    Class(&'static str),
    Id(&'static str),
    Tag(&'static str),
    AttributeExists(&'static str),
    AttributeEquals {
        name: &'static str,
        value: &'static str,
    },
    Hover,
    Active,
}

impl StaticSimpleSelectorDesc {
    fn to_selector(self) -> SimpleSelector {
        match self {
            Self::Class(name) => SimpleSelector::Class(name.to_string()),
            Self::Id(name) => SimpleSelector::Id(name.to_string()),
            Self::Tag(name) => SimpleSelector::Tag(name.to_string()),
            Self::AttributeExists(name) => SimpleSelector::AttributeExists(name.to_string()),
            Self::AttributeEquals { name, value } => SimpleSelector::AttributeEquals {
                name: name.to_string(),
                value: value.to_string(),
            },
            Self::Hover => SimpleSelector::Hover,
            Self::Active => SimpleSelector::Active,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StaticAncestorSelectorDesc {
    pub combinator: SelectorCombinator,
    pub simple_selectors: &'static [StaticSimpleSelectorDesc],
}

impl StaticAncestorSelectorDesc {
    pub const fn new(
        combinator: SelectorCombinator,
        simple_selectors: &'static [StaticSimpleSelectorDesc],
    ) -> Self {
        Self {
            combinator,
            simple_selectors,
        }
    }

    fn to_selector(self) -> AncestorSelector {
        AncestorSelector::new(
            self.combinator,
            CompoundSelector::new(
                self.simple_selectors
                    .iter()
                    .copied()
                    .map(StaticSimpleSelectorDesc::to_selector)
                    .collect(),
            ),
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StaticSelectorDesc {
    pub rightmost: &'static [StaticSimpleSelectorDesc],
    pub ancestors: &'static [StaticAncestorSelectorDesc],
    pub pseudo_element: Option<PseudoElementKind>,
}

impl StaticSelectorDesc {
    pub const fn new(
        rightmost: &'static [StaticSimpleSelectorDesc],
        ancestors: &'static [StaticAncestorSelectorDesc],
        pseudo_element: Option<PseudoElementKind>,
    ) -> Self {
        Self {
            rightmost,
            ancestors,
            pseudo_element,
        }
    }

    pub fn to_selector(self) -> Selector {
        Selector::complex(
            CompoundSelector::new(
                self.rightmost
                    .iter()
                    .copied()
                    .map(StaticSimpleSelectorDesc::to_selector)
                    .collect(),
            ),
            self.ancestors
                .iter()
                .copied()
                .map(StaticAncestorSelectorDesc::to_selector)
                .collect(),
            self.pseudo_element,
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StaticDisplay {
    None,
    Flex,
    Grid,
    Block,
}

impl StaticDisplay {
    fn to_runtime(self) -> TaffyDisplay {
        match self {
            Self::None => TaffyDisplay::None,
            Self::Flex => TaffyDisplay::Flex,
            Self::Grid => TaffyDisplay::Grid,
            Self::Block => TaffyDisplay::Block,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StaticPosition {
    Relative,
    Absolute,
}

impl StaticPosition {
    fn to_runtime(self) -> TaffyPosition {
        match self {
            Self::Relative => TaffyPosition::Relative,
            Self::Absolute => TaffyPosition::Absolute,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StaticDimension {
    Auto,
    Length(f32),
    Percent(f32),
}

impl StaticDimension {
    fn to_runtime(self) -> Dimension {
        match self {
            Self::Auto => Dimension::Auto,
            Self::Length(value) => Dimension::Length(value),
            Self::Percent(value) => Dimension::Percent(value),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StaticLengthPercentage {
    Length(f32),
    Percent(f32),
}

impl StaticLengthPercentage {
    fn to_runtime(self) -> TaffyLengthPercentage {
        match self {
            Self::Length(value) => TaffyLengthPercentage::Length(value),
            Self::Percent(value) => TaffyLengthPercentage::Percent(value),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StaticLengthPercentageAuto {
    Auto,
    Length(f32),
    Percent(f32),
}

impl StaticLengthPercentageAuto {
    fn to_runtime(self) -> TaffyLengthPercentageAuto {
        match self {
            Self::Auto => TaffyLengthPercentageAuto::Auto,
            Self::Length(value) => TaffyLengthPercentageAuto::Length(value),
            Self::Percent(value) => TaffyLengthPercentageAuto::Percent(value),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StaticFlexDirection {
    Row,
    RowReverse,
    Column,
    ColumnReverse,
}

impl StaticFlexDirection {
    fn to_runtime(self) -> FlexDirection {
        match self {
            Self::Row => FlexDirection::Row,
            Self::RowReverse => FlexDirection::RowReverse,
            Self::Column => FlexDirection::Column,
            Self::ColumnReverse => FlexDirection::ColumnReverse,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StaticFlexWrap {
    NoWrap,
    Wrap,
    WrapReverse,
}

impl StaticFlexWrap {
    fn to_runtime(self) -> FlexWrap {
        match self {
            Self::NoWrap => FlexWrap::NoWrap,
            Self::Wrap => FlexWrap::Wrap,
            Self::WrapReverse => FlexWrap::WrapReverse,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StaticJustifyContent {
    Start,
    End,
    FlexStart,
    FlexEnd,
    Center,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
    Stretch,
}

impl StaticJustifyContent {
    fn to_runtime(self) -> TaffyJustifyContent {
        match self {
            Self::Start => TaffyJustifyContent::Start,
            Self::End => TaffyJustifyContent::End,
            Self::FlexStart => TaffyJustifyContent::FlexStart,
            Self::FlexEnd => TaffyJustifyContent::FlexEnd,
            Self::Center => TaffyJustifyContent::Center,
            Self::SpaceBetween => TaffyJustifyContent::SpaceBetween,
            Self::SpaceAround => TaffyJustifyContent::SpaceAround,
            Self::SpaceEvenly => TaffyJustifyContent::SpaceEvenly,
            Self::Stretch => TaffyJustifyContent::Stretch,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StaticAlignContent {
    Start,
    End,
    FlexStart,
    FlexEnd,
    Center,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
    Stretch,
}

impl StaticAlignContent {
    fn to_runtime(self) -> TaffyAlignContent {
        match self {
            Self::Start => TaffyAlignContent::Start,
            Self::End => TaffyAlignContent::End,
            Self::FlexStart => TaffyAlignContent::FlexStart,
            Self::FlexEnd => TaffyAlignContent::FlexEnd,
            Self::Center => TaffyAlignContent::Center,
            Self::SpaceBetween => TaffyAlignContent::SpaceBetween,
            Self::SpaceAround => TaffyAlignContent::SpaceAround,
            Self::SpaceEvenly => TaffyAlignContent::SpaceEvenly,
            Self::Stretch => TaffyAlignContent::Stretch,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StaticAlignItems {
    Start,
    End,
    FlexStart,
    FlexEnd,
    Center,
    Stretch,
    Baseline,
}

impl StaticAlignItems {
    fn to_runtime(self) -> TaffyAlignItems {
        match self {
            Self::Start => TaffyAlignItems::Start,
            Self::End => TaffyAlignItems::End,
            Self::FlexStart => TaffyAlignItems::FlexStart,
            Self::FlexEnd => TaffyAlignItems::FlexEnd,
            Self::Center => TaffyAlignItems::Center,
            Self::Stretch => TaffyAlignItems::Stretch,
            Self::Baseline => TaffyAlignItems::Baseline,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StaticAlignSelf {
    Start,
    End,
    FlexStart,
    FlexEnd,
    Center,
    Stretch,
    Baseline,
}

impl StaticAlignSelf {
    fn to_runtime(self) -> TaffyAlignSelf {
        match self {
            Self::Start => TaffyAlignSelf::Start,
            Self::End => TaffyAlignSelf::End,
            Self::FlexStart => TaffyAlignSelf::FlexStart,
            Self::FlexEnd => TaffyAlignSelf::FlexEnd,
            Self::Center => TaffyAlignSelf::Center,
            Self::Stretch => TaffyAlignSelf::Stretch,
            Self::Baseline => TaffyAlignSelf::Baseline,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StaticDeclarationDesc {
    CustomProperty {
        name: &'static str,
        value: &'static str,
    },
    VariableDependentProperty {
        property_name: &'static str,
        value_css: &'static str,
    },
    Background(Color),
    Foreground(Color),
    OverflowX(OverflowMode),
    OverflowY(OverflowMode),
    Display(StaticDisplay),
    Position(StaticPosition),
    InsetTop(StaticLengthPercentageAuto),
    InsetRight(StaticLengthPercentageAuto),
    InsetBottom(StaticLengthPercentageAuto),
    InsetLeft(StaticLengthPercentageAuto),
    Width(StaticDimension),
    Height(StaticDimension),
    MarginTop(StaticLengthPercentageAuto),
    MarginRight(StaticLengthPercentageAuto),
    MarginBottom(StaticLengthPercentageAuto),
    MarginLeft(StaticLengthPercentageAuto),
    PaddingTop(StaticLengthPercentage),
    PaddingRight(StaticLengthPercentage),
    PaddingBottom(StaticLengthPercentage),
    PaddingLeft(StaticLengthPercentage),
    FlexDirection(StaticFlexDirection),
    FlexWrap(StaticFlexWrap),
    JustifyContent(Option<StaticJustifyContent>),
    AlignItems(Option<StaticAlignItems>),
    AlignSelf(Option<StaticAlignSelf>),
    AlignContent(Option<StaticAlignContent>),
    GapRow(StaticLengthPercentage),
    GapColumn(StaticLengthPercentage),
    FlexGrow(f32),
    FlexShrink(f32),
    FlexBasis(StaticDimension),
}

impl StaticDeclarationDesc {
    fn to_declaration(self) -> Declaration {
        match self {
            Self::CustomProperty { name, value } => Declaration::CustomProperty {
                name: name.to_string(),
                value: value.to_string(),
            },
            Self::VariableDependentProperty {
                property_name,
                value_css,
            } => Declaration::VariableDependentProperty {
                property_name: property_name.to_string(),
                value_css: value_css.to_string(),
            },
            Self::Background(color) => Declaration::Background(color),
            Self::Foreground(color) => Declaration::Foreground(color),
            Self::OverflowX(mode) => Declaration::OverflowX(mode),
            Self::OverflowY(mode) => Declaration::OverflowY(mode),
            Self::Display(display) => Declaration::Display(display.to_runtime()),
            Self::Position(position) => Declaration::Position(position.to_runtime()),
            Self::InsetTop(value) => Declaration::InsetTop(value.to_runtime()),
            Self::InsetRight(value) => Declaration::InsetRight(value.to_runtime()),
            Self::InsetBottom(value) => Declaration::InsetBottom(value.to_runtime()),
            Self::InsetLeft(value) => Declaration::InsetLeft(value.to_runtime()),
            Self::Width(value) => Declaration::Width(value.to_runtime()),
            Self::Height(value) => Declaration::Height(value.to_runtime()),
            Self::MarginTop(value) => Declaration::MarginTop(value.to_runtime()),
            Self::MarginRight(value) => Declaration::MarginRight(value.to_runtime()),
            Self::MarginBottom(value) => Declaration::MarginBottom(value.to_runtime()),
            Self::MarginLeft(value) => Declaration::MarginLeft(value.to_runtime()),
            Self::PaddingTop(value) => Declaration::PaddingTop(value.to_runtime()),
            Self::PaddingRight(value) => Declaration::PaddingRight(value.to_runtime()),
            Self::PaddingBottom(value) => Declaration::PaddingBottom(value.to_runtime()),
            Self::PaddingLeft(value) => Declaration::PaddingLeft(value.to_runtime()),
            Self::FlexDirection(direction) => Declaration::FlexDirection(direction.to_runtime()),
            Self::FlexWrap(wrap) => Declaration::FlexWrap(wrap.to_runtime()),
            Self::JustifyContent(value) => {
                Declaration::JustifyContent(value.map(StaticJustifyContent::to_runtime))
            }
            Self::AlignItems(value) => {
                Declaration::AlignItems(value.map(StaticAlignItems::to_runtime))
            }
            Self::AlignSelf(value) => {
                Declaration::AlignSelf(value.map(StaticAlignSelf::to_runtime))
            }
            Self::AlignContent(value) => {
                Declaration::AlignContent(value.map(StaticAlignContent::to_runtime))
            }
            Self::GapRow(value) => Declaration::GapRow(value.to_runtime()),
            Self::GapColumn(value) => Declaration::GapColumn(value.to_runtime()),
            Self::FlexGrow(value) => Declaration::FlexGrow(value),
            Self::FlexShrink(value) => Declaration::FlexShrink(value),
            Self::FlexBasis(value) => Declaration::FlexBasis(value.to_runtime()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StaticStyleRuleDesc {
    pub selector: StaticSelectorDesc,
    pub declarations: &'static [StaticDeclarationDesc],
}

impl StaticStyleRuleDesc {
    pub const fn new(
        selector: StaticSelectorDesc,
        declarations: &'static [StaticDeclarationDesc],
    ) -> Self {
        Self {
            selector,
            declarations,
        }
    }

    pub fn to_rule(self) -> StyleRule {
        StyleRule::new(
            self.selector.to_selector(),
            self.declarations
                .iter()
                .copied()
                .map(StaticDeclarationDesc::to_declaration)
                .collect(),
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StaticStylesheetDesc {
    pub rules: &'static [StaticStyleRuleDesc],
}

impl StaticStylesheetDesc {
    pub const fn new(rules: &'static [StaticStyleRuleDesc]) -> Self {
        Self { rules }
    }

    pub fn to_stylesheet(self) -> Stylesheet {
        let mut stylesheet = Stylesheet::default();
        for rule in self.rules {
            stylesheet.push(rule.to_rule());
        }
        stylesheet
    }
}

#[cfg(test)]
mod tests {
    use cssimpler_core::{ElementNode, ElementPath};
    use taffy::prelude::{Dimension, Display as TaffyDisplay, LengthPercentage as TaffyLength};

    use crate::{ElementInteractionState, resolve_style_with_interaction};

    use super::{
        StaticDeclarationDesc, StaticDisplay, StaticLengthPercentage, StaticSelectorDesc,
        StaticSimpleSelectorDesc, StaticStyleRuleDesc, StaticStylesheetDesc,
    };

    static ROOT_SELECTORS: [StaticSimpleSelectorDesc; 1] =
        [StaticSimpleSelectorDesc::Class("card")];
    static ROOT_DECLS: [StaticDeclarationDesc; 4] = [
        StaticDeclarationDesc::Display(StaticDisplay::Flex),
        StaticDeclarationDesc::Width(super::StaticDimension::Length(180.0)),
        StaticDeclarationDesc::Height(super::StaticDimension::Length(90.0)),
        StaticDeclarationDesc::GapColumn(StaticLengthPercentage::Length(8.0)),
    ];
    static ROOT_RULES: [StaticStyleRuleDesc; 1] = [StaticStyleRuleDesc::new(
        StaticSelectorDesc::new(&ROOT_SELECTORS, &[], None),
        &ROOT_DECLS,
    )];
    static STYLESHEET: StaticStylesheetDesc = StaticStylesheetDesc::new(&ROOT_RULES);

    #[test]
    fn static_stylesheet_descriptors_lower_into_runtime_stylesheet_rules() {
        let stylesheet = STYLESHEET.to_stylesheet();
        let element = ElementNode::new("div").with_class("card");
        let resolved = resolve_style_with_interaction(
            &element,
            &stylesheet,
            &ElementInteractionState::default(),
            &ElementPath::root(0),
        );

        assert_eq!(resolved.layout.taffy.display, TaffyDisplay::Flex);
        assert_eq!(resolved.layout.taffy.size.width, Dimension::Length(180.0));
        assert_eq!(resolved.layout.taffy.size.height, Dimension::Length(90.0));
        assert_eq!(resolved.layout.taffy.gap.width, TaffyLength::Length(8.0));
    }
}
