use cssimpler_core::Style;
use cssimpler_core::fonts::{
    FontFamily, FontStyle, GenericFontFamily, LineHeight as CoreLineHeight,
    TextTransform as CoreTextTransform,
};
use lightningcss::printer::PrinterOptions;
use lightningcss::properties::Property;
use lightningcss::properties::font::{
    AbsoluteFontSize, AbsoluteFontWeight, FontFamily as CssFontFamily, FontSize as CssFontSize,
    FontStyle as CssFontStyle, FontWeight as CssFontWeight,
    GenericFontFamily as CssGenericFontFamily, LineHeight as CssLineHeight,
    RelativeFontSize as CssRelativeFontSize,
};
use lightningcss::properties::text::{
    Spacing, TextTransform as CssTextTransform, TextTransformCase as CssTextTransformCase,
};
use lightningcss::traits::ToCss;
use lightningcss::values::length::{Length, LengthPercentage, LengthValue};

use crate::{Declaration, StyleError};

#[derive(Clone, Debug, PartialEq)]
pub enum FontSizeDeclaration {
    Px(f32),
    Scale(f32),
}

impl FontSizeDeclaration {
    fn resolve(&self, current_size_px: f32) -> f32 {
        match self {
            Self::Px(px) => *px,
            Self::Scale(scale) => current_size_px * *scale,
        }
        .max(1.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontWeightDeclaration {
    Absolute(u16),
    Bolder,
    Lighter,
}

impl FontWeightDeclaration {
    fn resolve(self, current_weight: u16) -> u16 {
        match self {
            Self::Absolute(weight) => weight,
            Self::Bolder => match current_weight {
                0..=349 => 400,
                350..=549 => 700,
                550..=899 => 900,
                _ => current_weight,
            },
            Self::Lighter => match current_weight {
                0..=199 => 100,
                200..=549 => 400,
                550..=749 => 700,
                _ => 700,
            },
        }
    }
}

pub type LineHeightDeclaration = CoreLineHeight;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LetterSpacingDeclaration {
    Px(f32),
    Em(f32),
}

impl LetterSpacingDeclaration {
    fn resolve_px(self, font_size_px: f32) -> f32 {
        match self {
            Self::Px(px) => px,
            Self::Em(scale) => font_size_px * scale,
        }
    }

    fn em_scale(self) -> Option<f32> {
        match self {
            Self::Em(scale) => Some(scale),
            Self::Px(_) => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct FontDeclarationState {
    letter_spacing_em: Option<f32>,
}

pub(crate) fn extract_property(
    property: &Property<'_>,
) -> Option<Result<Vec<Declaration>, StyleError>> {
    match property {
        Property::FontFamily(value) => {
            let families = font_families_from_css(value);
            (!families.is_empty()).then_some(Ok(vec![Declaration::FontFamilies(families)]))
        }
        Property::FontSize(value) => {
            Some(font_size_from_css(value).map(|value| vec![Declaration::FontSize(value)]))
        }
        Property::FontWeight(value) => Some(Ok(vec![Declaration::FontWeight(
            font_weight_from_css(value),
        )])),
        Property::FontStyle(value) => {
            Some(Ok(vec![Declaration::FontStyle(font_style_from_css(value))]))
        }
        Property::LineHeight(value) => {
            Some(line_height_from_css(value).map(|value| vec![Declaration::LineHeight(value)]))
        }
        Property::LetterSpacing(value) => Some(
            letter_spacing_from_css(value).map(|value| vec![Declaration::LetterSpacing(value)]),
        ),
        Property::TextTransform(value) => Some(
            text_transform_from_css(value).map(|value| vec![Declaration::TextTransform(value)]),
        ),
        _ => None,
    }
}

pub(crate) fn apply_font_declaration(
    style: &mut Style,
    declaration: &Declaration,
    state: &mut FontDeclarationState,
) -> bool {
    match declaration {
        Declaration::FontFamilies(value) => {
            style.visual.text.families = value.clone();
            true
        }
        Declaration::FontSize(value) => {
            style.visual.text.size_px = value.resolve(style.visual.text.size_px);
            if let Some(scale) = state.letter_spacing_em {
                style.visual.text.letter_spacing_px = style.visual.text.size_px * scale;
            }
            true
        }
        Declaration::FontWeight(value) => {
            style.visual.text.weight = value.resolve(style.visual.text.weight);
            true
        }
        Declaration::FontStyle(value) => {
            style.visual.text.style = *value;
            true
        }
        Declaration::LineHeight(value) => {
            style.visual.text.line_height = value.clone();
            true
        }
        Declaration::LetterSpacing(value) => {
            state.letter_spacing_em = value.em_scale();
            style.visual.text.letter_spacing_px = value.resolve_px(style.visual.text.size_px);
            true
        }
        Declaration::TextTransform(value) => {
            style.visual.text.text_transform = *value;
            true
        }
        _ => false,
    }
}

fn font_families_from_css(value: &[CssFontFamily<'_>]) -> Vec<FontFamily> {
    value.iter().filter_map(font_family_from_css).collect()
}

fn font_family_from_css(value: &CssFontFamily<'_>) -> Option<FontFamily> {
    match value {
        CssFontFamily::FamilyName(name) => {
            let family = name
                .to_css_string(PrinterOptions::default())
                .unwrap_or_default();
            let family = family
                .trim_matches('"')
                .trim_matches('\'')
                .trim()
                .to_string();
            (!family.is_empty()).then_some(FontFamily::Named(family))
        }
        CssFontFamily::Generic(generic) => match generic {
            CssGenericFontFamily::Serif => Some(FontFamily::Generic(GenericFontFamily::Serif)),
            CssGenericFontFamily::SansSerif => {
                Some(FontFamily::Generic(GenericFontFamily::SansSerif))
            }
            CssGenericFontFamily::Cursive => Some(FontFamily::Generic(GenericFontFamily::Cursive)),
            CssGenericFontFamily::Fantasy => Some(FontFamily::Generic(GenericFontFamily::Fantasy)),
            CssGenericFontFamily::Monospace => {
                Some(FontFamily::Generic(GenericFontFamily::Monospace))
            }
            CssGenericFontFamily::SystemUI => {
                Some(FontFamily::Generic(GenericFontFamily::SystemUi))
            }
            CssGenericFontFamily::Emoji => Some(FontFamily::Generic(GenericFontFamily::Emoji)),
            CssGenericFontFamily::Math => Some(FontFamily::Generic(GenericFontFamily::Math)),
            CssGenericFontFamily::FangSong => {
                Some(FontFamily::Generic(GenericFontFamily::FangSong))
            }
            CssGenericFontFamily::UISerif => Some(FontFamily::Generic(GenericFontFamily::UiSerif)),
            CssGenericFontFamily::UISansSerif => {
                Some(FontFamily::Generic(GenericFontFamily::UiSansSerif))
            }
            CssGenericFontFamily::UIMonospace => {
                Some(FontFamily::Generic(GenericFontFamily::UiMonospace))
            }
            CssGenericFontFamily::UIRounded => {
                Some(FontFamily::Generic(GenericFontFamily::UiRounded))
            }
            CssGenericFontFamily::Initial
            | CssGenericFontFamily::Inherit
            | CssGenericFontFamily::Unset
            | CssGenericFontFamily::Default
            | CssGenericFontFamily::Revert
            | CssGenericFontFamily::RevertLayer => None,
        },
    }
}

fn font_size_from_css(value: &CssFontSize) -> Result<FontSizeDeclaration, StyleError> {
    match value {
        CssFontSize::Length(length) => font_size_from_length(length),
        CssFontSize::Absolute(keyword) => Ok(FontSizeDeclaration::Px(match keyword {
            AbsoluteFontSize::XXSmall => 9.0,
            AbsoluteFontSize::XSmall => 10.0,
            AbsoluteFontSize::Small => 13.0,
            AbsoluteFontSize::Medium => 16.0,
            AbsoluteFontSize::Large => 18.0,
            AbsoluteFontSize::XLarge => 24.0,
            AbsoluteFontSize::XXLarge => 32.0,
            AbsoluteFontSize::XXXLarge => 48.0,
        })),
        CssFontSize::Relative(keyword) => Ok(FontSizeDeclaration::Scale(match keyword {
            CssRelativeFontSize::Smaller => 0.8,
            CssRelativeFontSize::Larger => 1.2,
        })),
    }
}

fn font_size_from_length(value: &LengthPercentage) -> Result<FontSizeDeclaration, StyleError> {
    match value {
        LengthPercentage::Dimension(length) => match length_value_to_typography_measure(length)? {
            TypographyLengthMeasure::Px(px) => Ok(FontSizeDeclaration::Px(px)),
            TypographyLengthMeasure::Em(scale) => Ok(FontSizeDeclaration::Scale(scale)),
        },
        LengthPercentage::Percentage(percentage) => Ok(FontSizeDeclaration::Scale(percentage.0)),
        _ => Err(StyleError::UnsupportedValue(format!("{value:?}"))),
    }
}

fn font_weight_from_css(value: &CssFontWeight) -> FontWeightDeclaration {
    match value {
        CssFontWeight::Absolute(absolute) => FontWeightDeclaration::Absolute(match absolute {
            AbsoluteFontWeight::Weight(value) => value.round().clamp(1.0, 1_000.0) as u16,
            AbsoluteFontWeight::Normal => 400,
            AbsoluteFontWeight::Bold => 700,
        }),
        CssFontWeight::Bolder => FontWeightDeclaration::Bolder,
        CssFontWeight::Lighter => FontWeightDeclaration::Lighter,
    }
}

fn font_style_from_css(value: &CssFontStyle) -> FontStyle {
    match value {
        CssFontStyle::Normal => FontStyle::Normal,
        CssFontStyle::Italic => FontStyle::Italic,
        CssFontStyle::Oblique(_) => FontStyle::Oblique,
    }
}

fn line_height_from_css(value: &CssLineHeight) -> Result<LineHeightDeclaration, StyleError> {
    match value {
        CssLineHeight::Normal => Ok(CoreLineHeight::Normal),
        CssLineHeight::Number(value) => Ok(CoreLineHeight::Scale(*value)),
        CssLineHeight::Length(length) => match length {
            LengthPercentage::Dimension(length) => {
                match length_value_to_typography_measure(length)? {
                    TypographyLengthMeasure::Px(px) => Ok(CoreLineHeight::Px(px)),
                    TypographyLengthMeasure::Em(scale) => Ok(CoreLineHeight::Scale(scale)),
                }
            }
            LengthPercentage::Percentage(percentage) => Ok(CoreLineHeight::Scale(percentage.0)),
            _ => Err(StyleError::UnsupportedValue(format!("{value:?}"))),
        },
    }
}

fn letter_spacing_from_css(value: &Spacing) -> Result<LetterSpacingDeclaration, StyleError> {
    match value {
        Spacing::Normal => Ok(LetterSpacingDeclaration::Px(0.0)),
        Spacing::Length(length) => match length_to_typography_measure(length)? {
            TypographyLengthMeasure::Px(px) => Ok(LetterSpacingDeclaration::Px(px)),
            TypographyLengthMeasure::Em(scale) => Ok(LetterSpacingDeclaration::Em(scale)),
        },
    }
}

fn text_transform_from_css(value: &CssTextTransform) -> Result<CoreTextTransform, StyleError> {
    if !value.other.is_empty() {
        return Err(StyleError::UnsupportedValue(
            value
                .to_css_string(PrinterOptions::default())
                .unwrap_or_else(|_| format!("{value:?}")),
        ));
    }

    Ok(match value.case {
        CssTextTransformCase::None => CoreTextTransform::None,
        CssTextTransformCase::Uppercase => CoreTextTransform::Uppercase,
        CssTextTransformCase::Lowercase => CoreTextTransform::Lowercase,
        CssTextTransformCase::Capitalize => CoreTextTransform::Capitalize,
    })
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum TypographyLengthMeasure {
    Px(f32),
    Em(f32),
}

fn length_to_typography_measure(value: &Length) -> Result<TypographyLengthMeasure, StyleError> {
    match value {
        Length::Value(length) => length_value_to_typography_measure(length),
        _ => Err(StyleError::UnsupportedValue(format!("{value:?}"))),
    }
}

fn length_value_to_typography_measure(
    value: &LengthValue,
) -> Result<TypographyLengthMeasure, StyleError> {
    if let Some(px) = value.to_px() {
        return Ok(TypographyLengthMeasure::Px(px as f32));
    }

    match value {
        LengthValue::Em(scale) => Ok(TypographyLengthMeasure::Em(*scale)),
        _ => Err(StyleError::UnsupportedValue(format!("{value:?}"))),
    }
}

#[cfg(test)]
mod tests {
    use cssimpler_core::fonts::{
        FontFamily, GenericFontFamily, LineHeight, TextStyle, TextTransform,
    };

    use super::{
        FontDeclarationState, FontSizeDeclaration, FontWeightDeclaration, LetterSpacingDeclaration,
        apply_font_declaration,
    };
    use crate::Declaration;

    #[test]
    fn relative_font_size_scales_from_current_value() {
        let mut style = cssimpler_core::Style::default();
        style.visual.text.size_px = 20.0;
        let mut declaration_state = FontDeclarationState::default();

        assert!(apply_font_declaration(
            &mut style,
            &Declaration::FontSize(FontSizeDeclaration::Scale(1.5)),
            &mut declaration_state,
        ));

        assert_eq!(style.visual.text.size_px, 30.0);
    }

    #[test]
    fn relative_font_weight_steps_up_from_regular_weight() {
        let mut style = cssimpler_core::Style::default();
        style.visual.text.weight = 400;
        let mut declaration_state = FontDeclarationState::default();

        assert!(apply_font_declaration(
            &mut style,
            &Declaration::FontWeight(FontWeightDeclaration::Bolder),
            &mut declaration_state,
        ));

        assert_eq!(style.visual.text.weight, 700);
    }

    #[test]
    fn line_height_assignment_replaces_the_current_value() {
        let mut style = cssimpler_core::Style::default();
        let mut declaration_state = FontDeclarationState::default();

        assert!(apply_font_declaration(
            &mut style,
            &Declaration::LineHeight(LineHeight::Scale(1.4)),
            &mut declaration_state,
        ));

        assert_eq!(style.visual.text.line_height, LineHeight::Scale(1.4));
    }

    #[test]
    fn explicit_font_family_replaces_the_family_stack() {
        let mut style = cssimpler_core::Style::default();
        let family = FontFamily::Generic(GenericFontFamily::Monospace);
        let mut declaration_state = FontDeclarationState::default();

        assert!(apply_font_declaration(
            &mut style,
            &Declaration::FontFamilies(vec![family.clone()]),
            &mut declaration_state,
        ));

        assert_eq!(style.visual.text, TextStyle::default().with_family(family));
    }

    #[test]
    fn letter_spacing_assignment_updates_the_text_style() {
        let mut style = cssimpler_core::Style::default();
        let mut declaration_state = FontDeclarationState::default();

        assert!(apply_font_declaration(
            &mut style,
            &Declaration::LetterSpacing(LetterSpacingDeclaration::Px(2.5)),
            &mut declaration_state,
        ));

        assert_eq!(style.visual.text.letter_spacing_px, 2.5);
    }

    #[test]
    fn text_transform_assignment_updates_the_text_style() {
        let mut style = cssimpler_core::Style::default();
        let mut declaration_state = FontDeclarationState::default();

        assert!(apply_font_declaration(
            &mut style,
            &Declaration::TextTransform(TextTransform::Uppercase),
            &mut declaration_state,
        ));

        assert_eq!(style.visual.text.text_transform, TextTransform::Uppercase);
    }

    #[test]
    fn em_letter_spacing_recomputes_when_font_size_changes() {
        let mut style = cssimpler_core::Style::default();
        let mut declaration_state = FontDeclarationState::default();

        assert!(apply_font_declaration(
            &mut style,
            &Declaration::LetterSpacing(LetterSpacingDeclaration::Em(0.05)),
            &mut declaration_state,
        ));
        assert!((style.visual.text.letter_spacing_px - 0.8).abs() < 0.001);

        assert!(apply_font_declaration(
            &mut style,
            &Declaration::FontSize(FontSizeDeclaration::Px(20.0)),
            &mut declaration_state,
        ));
        assert!((style.visual.text.letter_spacing_px - 1.0).abs() < 0.001);
    }
}
