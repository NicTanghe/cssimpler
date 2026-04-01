use cssimpler_core::{
    AnglePercentageValue, GradientInterpolation, GradientStop, LengthPercentageValue, LinearRgba,
};

#[derive(Clone, Copy)]
pub(crate) struct ResolvedGradientStop {
    pub(crate) color: LinearRgba,
    pub(crate) position: f32,
}

pub(crate) fn resolve_length_stops(
    stops: &[GradientStop<LengthPercentageValue>],
    total: f32,
    origin: f32,
) -> Vec<ResolvedGradientStop> {
    let mut resolved: Vec<_> = stops
        .iter()
        .map(|stop| ResolvedGradientStop {
            color: stop.color.to_linear_rgba(),
            position: origin + stop.position.resolve(total),
        })
        .collect();
    clamp_resolved_stop_positions(&mut resolved);
    resolved
}

pub(crate) fn resolve_angle_stops(
    stops: &[GradientStop<AnglePercentageValue>],
) -> Vec<ResolvedGradientStop> {
    let mut resolved: Vec<_> = stops
        .iter()
        .map(|stop| ResolvedGradientStop {
            color: stop.color.to_linear_rgba(),
            position: stop.position.resolve_degrees(),
        })
        .collect();
    clamp_resolved_stop_positions(&mut resolved);
    resolved
}

pub(crate) fn sample_gradient(
    stops: &[ResolvedGradientStop],
    position: f32,
    repeating: bool,
    interpolation: GradientInterpolation,
) -> LinearRgba {
    sample_gradient_color(
        stops,
        normalize_gradient_t(position, stops, repeating),
        interpolation,
    )
}

fn clamp_resolved_stop_positions(stops: &mut [ResolvedGradientStop]) {
    let mut last_position = f32::NEG_INFINITY;
    for stop in stops {
        if stop.position < last_position {
            stop.position = last_position;
        } else {
            last_position = stop.position;
        }
    }
}

fn normalize_gradient_t(t: f32, stops: &[ResolvedGradientStop], repeating: bool) -> f32 {
    let start = stops.first().map(|stop| stop.position).unwrap_or(0.0);
    let end = stops.last().map(|stop| stop.position).unwrap_or(start);
    if !repeating {
        return t;
    }

    let period = end - start;
    if period.abs() <= f32::EPSILON {
        start
    } else {
        start + (t - start).rem_euclid(period)
    }
}

fn sample_gradient_color(
    stops: &[ResolvedGradientStop],
    t: f32,
    interpolation: GradientInterpolation,
) -> LinearRgba {
    let Some(first) = stops.first().copied() else {
        return LinearRgba::TRANSPARENT;
    };
    if t <= first.position {
        return first.color;
    }

    for pair in stops.windows(2) {
        let start = pair[0];
        let end = pair[1];
        if t > end.position {
            continue;
        }

        let span = end.position - start.position;
        if span.abs() <= f32::EPSILON {
            return end.color;
        }

        return start
            .color
            .interpolate(end.color, (t - start.position) / span, interpolation);
    }

    stops
        .last()
        .copied()
        .map(|stop| stop.color)
        .unwrap_or(LinearRgba::TRANSPARENT)
}
