use cssimpler_core::MixBlendMode;
use lightningcss::{
    printer::PrinterOptions,
    properties::{Property, effects::BlendMode},
};

use crate::{Declaration, StyleError};

pub(super) fn extract(mode: BlendMode) -> MixBlendMode {
    match mode {
        BlendMode::Normal => MixBlendMode::Normal,
        BlendMode::Multiply => MixBlendMode::Multiply,
        BlendMode::Screen => MixBlendMode::Screen,
        BlendMode::Overlay => MixBlendMode::Overlay,
        BlendMode::Darken => MixBlendMode::Darken,
        BlendMode::Lighten => MixBlendMode::Lighten,
        BlendMode::ColorDodge => MixBlendMode::ColorDodge,
        BlendMode::ColorBurn => MixBlendMode::ColorBurn,
        BlendMode::HardLight => MixBlendMode::HardLight,
        BlendMode::SoftLight => MixBlendMode::SoftLight,
        BlendMode::Difference => MixBlendMode::Difference,
        BlendMode::Exclusion => MixBlendMode::Exclusion,
        BlendMode::Hue => MixBlendMode::Hue,
        BlendMode::Saturation => MixBlendMode::Saturation,
        BlendMode::Color => MixBlendMode::Color,
        BlendMode::Luminosity => MixBlendMode::Luminosity,
        BlendMode::PlusDarker => MixBlendMode::PlusDarker,
        BlendMode::PlusLighter => MixBlendMode::PlusLighter,
    }
}

pub(super) fn isolation(property: &Property<'_>) -> Result<Vec<Declaration>, StyleError> {
    let value = property
        .value_to_css_string(PrinterOptions::default())
        .map_err(|error| StyleError::UnsupportedValue(error.to_string()))?;
    if value.trim().eq_ignore_ascii_case("isolate") {
        Ok(vec![Declaration::Isolation(true)])
    } else if value.trim().eq_ignore_ascii_case("auto") {
        Ok(vec![Declaration::Isolation(false)])
    } else {
        Err(StyleError::UnsupportedValue(value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cssimpler_core::ElementNode;

    #[test]
    fn parsed_blend_mode_and_isolate_reach_visual_style() {
        let stylesheet = crate::parse_stylesheet(
            "panel { --mode: multiply; mix-blend-mode: var(--mode); isolation: isolate; }",
        )
        .unwrap();
        let style = crate::resolve_style(&ElementNode::new("panel"), &stylesheet);
        assert_eq!(style.visual.mix_blend_mode, MixBlendMode::Multiply);
        assert!(style.visual.isolate_blending);
    }
}
