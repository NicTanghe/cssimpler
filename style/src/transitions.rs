use cssimpler_core::{Style, TransitionPropertyName, TransitionTimingFunction};
use lightningcss::properties::{Property, PropertyId};
use lightningcss::values::{easing::EasingFunction as CssEasingFunction, time::Time as CssTime};

use crate::{Declaration, StyleError};

pub(crate) fn extract_property(
    property: &Property<'_>,
) -> Option<Result<Vec<Declaration>, StyleError>> {
    match property {
        Property::TransitionProperty(properties, _) => Some(Ok(vec![
            Declaration::TransitionProperties(transition_properties_from_css(properties)),
        ])),
        Property::TransitionDuration(durations, _) => Some(Ok(vec![
            Declaration::TransitionDurations(transition_times_from_css(durations)),
        ])),
        Property::TransitionDelay(delays, _) => Some(Ok(vec![
            Declaration::TransitionDelays(transition_times_from_css(delays)),
        ])),
        Property::TransitionTimingFunction(timings, _) => Some(Ok(vec![
            Declaration::TransitionTimingFunctions(transition_timings_from_css(timings)),
        ])),
        Property::Transition(transitions, _) => Some(Ok(vec![
            Declaration::TransitionProperties(
                transitions
                    .iter()
                    .map(|transition| transition_property_name(&transition.property))
                    .collect(),
            ),
            Declaration::TransitionDurations(
                transitions
                    .iter()
                    .map(|transition| transition_time_to_seconds(&transition.duration))
                    .collect(),
            ),
            Declaration::TransitionDelays(
                transitions
                    .iter()
                    .map(|transition| transition_time_to_seconds(&transition.delay))
                    .collect(),
            ),
            Declaration::TransitionTimingFunctions(
                transitions
                    .iter()
                    .map(|transition| transition_timing_from_css(&transition.timing_function))
                    .collect(),
            ),
        ])),
        _ => None,
    }
}

pub(crate) fn apply_declaration(style: &mut Style, declaration: &Declaration) -> bool {
    match declaration {
        Declaration::TransitionProperties(value) => {
            style.transitions.properties = value.clone();
            true
        }
        Declaration::TransitionDurations(value) => {
            style.transitions.durations_seconds = value.clone();
            true
        }
        Declaration::TransitionDelays(value) => {
            style.transitions.delays_seconds = value.clone();
            true
        }
        Declaration::TransitionTimingFunctions(value) => {
            style.transitions.timing_functions = value.clone();
            true
        }
        _ => false,
    }
}

fn transition_properties_from_css(properties: &[PropertyId<'_>]) -> Vec<TransitionPropertyName> {
    if properties
        .iter()
        .any(|property| property.name().eq_ignore_ascii_case("none"))
    {
        return Vec::new();
    }

    properties.iter().map(transition_property_name).collect()
}

fn transition_property_name(property: &PropertyId<'_>) -> TransitionPropertyName {
    if property.name().eq_ignore_ascii_case("all") {
        TransitionPropertyName::All
    } else {
        TransitionPropertyName::Property(property.name().to_string())
    }
}

fn transition_times_from_css(times: &[CssTime]) -> Vec<f32> {
    times.iter().map(transition_time_to_seconds).collect()
}

fn transition_time_to_seconds(time: &CssTime) -> f32 {
    time.to_ms() as f32 / 1000.0
}

fn transition_timings_from_css(timings: &[CssEasingFunction]) -> Vec<TransitionTimingFunction> {
    timings.iter().map(transition_timing_from_css).collect()
}

fn transition_timing_from_css(timing: &CssEasingFunction) -> TransitionTimingFunction {
    match timing {
        CssEasingFunction::Linear => TransitionTimingFunction::Linear,
        CssEasingFunction::Ease => TransitionTimingFunction::Ease,
        CssEasingFunction::EaseIn => TransitionTimingFunction::EaseIn,
        CssEasingFunction::EaseOut => TransitionTimingFunction::EaseOut,
        CssEasingFunction::EaseInOut => TransitionTimingFunction::EaseInOut,
        CssEasingFunction::CubicBezier { x1, y1, x2, y2 } => match (*x1, *y1, *x2, *y2) {
            (0.25, 0.1, 0.25, 1.0) => TransitionTimingFunction::Ease,
            (0.42, 0.0, 1.0, 1.0) => TransitionTimingFunction::EaseIn,
            (0.0, 0.0, 0.58, 1.0) => TransitionTimingFunction::EaseOut,
            (0.42, 0.0, 0.58, 1.0) => TransitionTimingFunction::EaseInOut,
            _ => TransitionTimingFunction::Unsupported,
        },
        _ => TransitionTimingFunction::Unsupported,
    }
}
