//! Compile-time finalizers for baked style descriptors.
//!
//! These helpers intentionally operate on already-structured
//! [`crate::StaticDeclarationDesc`] values and trait-provided associated constants.
//! They are not CSS parsers, and they deliberately avoid depending on newer const
//! trait method surfaces so the API remains useful on toolchains where const-trait
//! support is still incomplete.

use cssimpler_core::Color;

use crate::{
    StaticAlignItems, StaticDeclarationDesc, StaticDimension, StaticDisplay, StaticFlexDirection,
    StaticJustifyContent, StaticLengthPercentage,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StaticStyleFlags {
    pub paint: bool,
    pub layout: bool,
    pub absolute_position: bool,
    pub custom_properties: bool,
    pub variable_dependencies: bool,
}

impl StaticStyleFlags {
    pub const NONE: Self = Self {
        paint: false,
        layout: false,
        absolute_position: false,
        custom_properties: false,
        variable_dependencies: false,
    };

    const fn mark_paint(mut self) -> Self {
        self.paint = true;
        self
    }

    const fn mark_layout(mut self) -> Self {
        self.paint = true;
        self.layout = true;
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StaticLayoutHint {
    pub display: StaticDisplay,
    pub flex_direction: StaticFlexDirection,
    pub width: StaticDimension,
    pub height: StaticDimension,
    pub padding_x: StaticLengthPercentage,
    pub padding_y: StaticLengthPercentage,
    pub gap_x: StaticLengthPercentage,
    pub gap_y: StaticLengthPercentage,
}

impl StaticLayoutHint {
    pub const fn row_box(
        width: StaticDimension,
        height: StaticDimension,
        padding_x: StaticLengthPercentage,
        padding_y: StaticLengthPercentage,
        gap_x: StaticLengthPercentage,
        gap_y: StaticLengthPercentage,
    ) -> Self {
        Self {
            display: StaticDisplay::Flex,
            flex_direction: StaticFlexDirection::Row,
            width,
            height,
            padding_x,
            padding_y,
            gap_x,
            gap_y,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FinalizedStaticStyle<const N: usize> {
    pub declarations: [StaticDeclarationDesc; N],
    pub flags: StaticStyleFlags,
    pub layout_hint: StaticLayoutHint,
}

pub type FinalizedStaticBoxStyle = FinalizedStaticStyle<14>;

/// A const-friendly theme contract for finalizing compact baked style tables.
///
/// This is intentionally based on associated constants instead of const trait
/// methods so compile-time specialization keeps working even when the newest
/// const-trait method surface is unavailable.
pub trait StaticThemeSpec {
    const BACKGROUND: Color;
    const FOREGROUND: Color;
    const WIDTH: StaticDimension;
    const HEIGHT: StaticDimension;
    const PADDING_X: StaticLengthPercentage;
    const PADDING_Y: StaticLengthPercentage;
    const GAP_X: StaticLengthPercentage;
    const GAP_Y: StaticLengthPercentage;
    const FLEX_DIRECTION: StaticFlexDirection;
    const JUSTIFY_CONTENT: Option<StaticJustifyContent>;
    const ALIGN_ITEMS: Option<StaticAlignItems>;
}

/// Finalize a small themed box style at compile time.
///
/// This function specializes existing baked descriptors from type-provided
/// constants. It does not parse CSS text and it does not perform runtime-only
/// work such as heap allocation or dynamic selector matching.
pub const fn finalize_box_style<T: StaticThemeSpec>() -> FinalizedStaticBoxStyle {
    let declarations = [
        StaticDeclarationDesc::Display(StaticDisplay::Flex),
        StaticDeclarationDesc::FlexDirection(T::FLEX_DIRECTION),
        StaticDeclarationDesc::JustifyContent(T::JUSTIFY_CONTENT),
        StaticDeclarationDesc::AlignItems(T::ALIGN_ITEMS),
        StaticDeclarationDesc::Width(T::WIDTH),
        StaticDeclarationDesc::Height(T::HEIGHT),
        StaticDeclarationDesc::PaddingTop(T::PADDING_Y),
        StaticDeclarationDesc::PaddingRight(T::PADDING_X),
        StaticDeclarationDesc::PaddingBottom(T::PADDING_Y),
        StaticDeclarationDesc::PaddingLeft(T::PADDING_X),
        StaticDeclarationDesc::GapRow(T::GAP_Y),
        StaticDeclarationDesc::GapColumn(T::GAP_X),
        StaticDeclarationDesc::Background(T::BACKGROUND),
        StaticDeclarationDesc::Foreground(T::FOREGROUND),
    ];
    FinalizedStaticStyle {
        declarations,
        flags: summarize_static_declarations(&declarations),
        layout_hint: StaticLayoutHint {
            display: StaticDisplay::Flex,
            flex_direction: T::FLEX_DIRECTION,
            width: T::WIDTH,
            height: T::HEIGHT,
            padding_x: T::PADDING_X,
            padding_y: T::PADDING_Y,
            gap_x: T::GAP_X,
            gap_y: T::GAP_Y,
        },
    }
}

/// Summarize already-generated declarations into compile-time flags.
///
/// This is a finalizer over structured descriptor data and is meant for compact
/// lookup-style metadata, not for parsing raw CSS source.
pub const fn summarize_static_declarations<const N: usize>(
    declarations: &[StaticDeclarationDesc; N],
) -> StaticStyleFlags {
    let mut flags = StaticStyleFlags::NONE;
    let mut index = 0;
    while index < N {
        flags = match declarations[index] {
            StaticDeclarationDesc::CustomProperty { .. } => {
                let mut next = flags.mark_paint();
                next.custom_properties = true;
                next
            }
            StaticDeclarationDesc::VariableDependentProperty { .. } => {
                let mut next = flags.mark_paint();
                next.variable_dependencies = true;
                next
            }
            StaticDeclarationDesc::Background(_)
            | StaticDeclarationDesc::Foreground(_)
            | StaticDeclarationDesc::OverflowX(_)
            | StaticDeclarationDesc::OverflowY(_) => flags.mark_paint(),
            StaticDeclarationDesc::Display(_)
            | StaticDeclarationDesc::Width(_)
            | StaticDeclarationDesc::Height(_)
            | StaticDeclarationDesc::MarginTop(_)
            | StaticDeclarationDesc::MarginRight(_)
            | StaticDeclarationDesc::MarginBottom(_)
            | StaticDeclarationDesc::MarginLeft(_)
            | StaticDeclarationDesc::PaddingTop(_)
            | StaticDeclarationDesc::PaddingRight(_)
            | StaticDeclarationDesc::PaddingBottom(_)
            | StaticDeclarationDesc::PaddingLeft(_)
            | StaticDeclarationDesc::FlexDirection(_)
            | StaticDeclarationDesc::FlexWrap(_)
            | StaticDeclarationDesc::JustifyContent(_)
            | StaticDeclarationDesc::AlignItems(_)
            | StaticDeclarationDesc::AlignSelf(_)
            | StaticDeclarationDesc::AlignContent(_)
            | StaticDeclarationDesc::GapRow(_)
            | StaticDeclarationDesc::GapColumn(_)
            | StaticDeclarationDesc::FlexGrow(_)
            | StaticDeclarationDesc::FlexShrink(_)
            | StaticDeclarationDesc::FlexBasis(_) => flags.mark_layout(),
            StaticDeclarationDesc::Position(position) => {
                let mut next = flags.mark_layout();
                if matches!(position, crate::StaticPosition::Absolute) {
                    next.absolute_position = true;
                }
                next
            }
            StaticDeclarationDesc::InsetTop(_)
            | StaticDeclarationDesc::InsetRight(_)
            | StaticDeclarationDesc::InsetBottom(_)
            | StaticDeclarationDesc::InsetLeft(_) => {
                let mut next = flags.mark_layout();
                next.absolute_position = true;
                next
            }
        };
        index += 1;
    }
    flags
}

#[cfg(test)]
mod tests {
    use cssimpler_core::{Color, ElementNode, ElementPath};
    use taffy::prelude::{
        Dimension, Display as TaffyDisplay, FlexDirection, JustifyContent as TaffyJustifyContent,
        LengthPercentage as TaffyLength,
    };

    use crate::{
        ElementInteractionState, StaticSelectorDesc, StaticSimpleSelectorDesc, StaticStyleRuleDesc,
        StaticStylesheetDesc, resolve_style_with_interaction,
    };

    use super::{
        FinalizedStaticBoxStyle, StaticLayoutHint, StaticStyleFlags, StaticThemeSpec,
        finalize_box_style, summarize_static_declarations,
    };

    struct Primary;

    impl StaticThemeSpec for Primary {
        const BACKGROUND: Color = Color::rgb(0x25, 0x63, 0xeb);
        const FOREGROUND: Color = Color::WHITE;
        const WIDTH: crate::StaticDimension = crate::StaticDimension::Length(180.0);
        const HEIGHT: crate::StaticDimension = crate::StaticDimension::Length(48.0);
        const PADDING_X: crate::StaticLengthPercentage =
            crate::StaticLengthPercentage::Length(16.0);
        const PADDING_Y: crate::StaticLengthPercentage =
            crate::StaticLengthPercentage::Length(12.0);
        const GAP_X: crate::StaticLengthPercentage = crate::StaticLengthPercentage::Length(8.0);
        const GAP_Y: crate::StaticLengthPercentage = crate::StaticLengthPercentage::Length(6.0);
        const FLEX_DIRECTION: crate::StaticFlexDirection = crate::StaticFlexDirection::Row;
        const JUSTIFY_CONTENT: Option<crate::StaticJustifyContent> =
            Some(crate::StaticJustifyContent::Center);
        const ALIGN_ITEMS: Option<crate::StaticAlignItems> = Some(crate::StaticAlignItems::Center);
    }

    const PRIMARY_STYLE: FinalizedStaticBoxStyle = finalize_box_style::<Primary>();
    const PRIMARY_FLAGS: StaticStyleFlags =
        summarize_static_declarations(&PRIMARY_STYLE.declarations);
    static ROOT_SELECTORS: [StaticSimpleSelectorDesc; 1] =
        [StaticSimpleSelectorDesc::Class("button")];
    static ROOT_DECLS: [crate::StaticDeclarationDesc; 14] = PRIMARY_STYLE.declarations;
    static ROOT_RULES: [StaticStyleRuleDesc; 1] = [StaticStyleRuleDesc::new(
        StaticSelectorDesc::new(&ROOT_SELECTORS, &[], None),
        &ROOT_DECLS,
    )];
    static STYLESHEET: StaticStylesheetDesc = StaticStylesheetDesc::new(&ROOT_RULES);

    #[test]
    fn const_theme_finalizer_specializes_baked_styles_from_type_level_constants() {
        let stylesheet = STYLESHEET.to_stylesheet();
        let element = ElementNode::new("button").with_class("button");
        let resolved = resolve_style_with_interaction(
            &element,
            &stylesheet,
            &ElementInteractionState::default(),
            &ElementPath::root(0),
        );

        assert_eq!(
            PRIMARY_STYLE.layout_hint.width,
            crate::StaticDimension::Length(180.0)
        );
        assert_eq!(
            PRIMARY_STYLE.layout_hint,
            StaticLayoutHint::row_box(
                crate::StaticDimension::Length(180.0),
                crate::StaticDimension::Length(48.0),
                crate::StaticLengthPercentage::Length(16.0),
                crate::StaticLengthPercentage::Length(12.0),
                crate::StaticLengthPercentage::Length(8.0),
                crate::StaticLengthPercentage::Length(6.0),
            )
        );
        assert_eq!(resolved.layout.taffy.display, TaffyDisplay::Flex);
        assert_eq!(resolved.layout.taffy.flex_direction, FlexDirection::Row);
        assert_eq!(
            resolved.layout.taffy.justify_content,
            Some(TaffyJustifyContent::Center)
        );
        assert_eq!(resolved.layout.taffy.size.width, Dimension::Length(180.0));
        assert_eq!(resolved.layout.taffy.size.height, Dimension::Length(48.0));
        assert_eq!(
            resolved.layout.taffy.padding.left,
            TaffyLength::Length(16.0)
        );
        assert_eq!(resolved.layout.taffy.gap.width, TaffyLength::Length(8.0));
        assert_eq!(
            resolved.visual.background,
            Some(Color::rgb(0x25, 0x63, 0xeb))
        );
        assert_eq!(resolved.visual.foreground, Color::WHITE);
    }

    #[test]
    fn const_declaration_summary_reports_flags_without_runtime_parsing() {
        assert_eq!(
            PRIMARY_FLAGS,
            StaticStyleFlags {
                paint: true,
                layout: true,
                absolute_position: false,
                custom_properties: false,
                variable_dependencies: false,
            }
        );
    }
}
