use cssimpler_core::{
    AnglePercentageValue, BackgroundLayer, CircleRadius, ConicGradient, CornerRadius,
    EllipseRadius, GradientDirection, GradientHorizontal, GradientInterpolation, GradientPoint,
    GradientStop, GradientVertical, LayoutBox, LengthPercentageValue, LinearGradient, LinearRgba,
    RadialGradient, RadialShape, ShapeExtent,
};

use super::shapes::{draw_rounded_rect, pixel_bounds, point_in_rounded_rect};
use super::{blend_linear_pixel, ClipRect};

#[derive(Clone, Copy)]
pub(crate) struct ResolvedGradientStop {
    pub(crate) color: LinearRgba,
    pub(crate) position: f32,
}

#[derive(Clone)]
pub(crate) struct PreparedResolvedGradient {
    interpolation: GradientInterpolation,
    first: Option<PreparedColor>,
    start: f32,
    end: f32,
    segments: Vec<PreparedResolvedGradientSegment>,
}

#[derive(Clone)]
pub(crate) struct PreparedLengthGradient {
    interpolation: GradientInterpolation,
    stops: Vec<PreparedLengthGradientStop>,
}

#[derive(Clone, Copy)]
struct PreparedResolvedGradientSegment {
    start: f32,
    end: f32,
    inverse_span: f32,
    start_color: PreparedColor,
    end_color: PreparedColor,
}

#[derive(Clone, Copy)]
struct PreparedLengthGradientStop {
    color: PreparedColor,
    px: f32,
    fraction: f32,
}

#[derive(Clone, Copy)]
struct PreparedColor {
    linear: LinearRgba,
    oklab: Oklab,
}

#[derive(Clone, Copy)]
struct Oklab {
    l: f32,
    a: f32,
    b: f32,
}

#[derive(Clone, Copy)]
struct ResolvedRadialShape {
    radius_x: f32,
    radius_y: f32,
}

pub(crate) fn draw_background_layer(
    buffer: &mut [u32],
    width: usize,
    height: usize,
    layout: LayoutBox,
    radius: CornerRadius,
    layer: &BackgroundLayer,
    clip: ClipRect,
) {
    match layer {
        BackgroundLayer::LinearGradient(gradient) => {
            draw_linear_gradient(buffer, width, height, layout, radius, gradient, clip);
        }
        BackgroundLayer::RadialGradient(gradient) => {
            draw_radial_gradient(buffer, width, height, layout, radius, gradient, clip);
        }
        BackgroundLayer::ConicGradient(gradient) => {
            draw_conic_gradient(buffer, width, height, layout, radius, gradient, clip);
        }
    }
}

fn draw_linear_gradient(
    buffer: &mut [u32],
    width: usize,
    height: usize,
    layout: LayoutBox,
    radius: CornerRadius,
    gradient: &LinearGradient,
    clip: ClipRect,
) {
    let Some((x0, y0, x1, y1)) = pixel_bounds(layout, clip, width, height) else {
        return;
    };

    let Some(first_stop) = gradient.stops.first() else {
        return;
    };
    let direction = gradient_direction_vector(gradient.direction, layout);
    let center_x = layout.x + layout.width * 0.5;
    let center_y = layout.y + layout.height * 0.5;
    let (min_projection, max_projection) =
        gradient_projection_bounds(layout, center_x, center_y, direction);
    let projection_span = max_projection - min_projection;

    if projection_span.abs() <= f32::EPSILON {
        draw_rounded_rect(
            buffer,
            width,
            height,
            layout,
            radius,
            first_stop.color,
            clip,
        );
        return;
    }

    let stops = resolve_length_stops(&gradient.stops, projection_span, min_projection);
    let prepared = prepare_resolved_gradient(&stops, gradient.interpolation);
    let projection_step = direction.0;
    for y in y0..y1 {
        let py = y as f32 + 0.5;
        let mut projection =
            ((x0 as f32 + 0.5 - center_x) * direction.0) + ((py - center_y) * direction.1);
        for x in x0..x1 {
            let px = x as f32 + 0.5;
            if !point_in_rounded_rect(px, py, layout, radius) {
                projection += projection_step;
                continue;
            }

            let color = sample_prepared_gradient(&prepared, projection, gradient.repeating);
            blend_linear_pixel(buffer, width, height, x, y, color);
            projection += projection_step;
        }
    }
}

fn draw_radial_gradient(
    buffer: &mut [u32],
    width: usize,
    height: usize,
    layout: LayoutBox,
    radius: CornerRadius,
    gradient: &RadialGradient,
    clip: ClipRect,
) {
    let Some((x0, y0, x1, y1)) = pixel_bounds(layout, clip, width, height) else {
        return;
    };

    let Some(first_stop) = gradient.stops.first() else {
        return;
    };

    let (center_x, center_y) = resolve_gradient_point(gradient.center, layout);
    let resolved_shape = resolve_radial_shape(gradient.shape, layout, center_x, center_y);
    if resolved_shape.radius_x <= f32::EPSILON || resolved_shape.radius_y <= f32::EPSILON {
        draw_rounded_rect(
            buffer,
            width,
            height,
            layout,
            radius,
            first_stop.color,
            clip,
        );
        return;
    }

    let prepared_length_gradient = prepare_length_gradient(&gradient.stops, gradient.interpolation);
    let fraction_only = length_stops_use_fraction_only(&gradient.stops);
    let prepared_unit_gradient = fraction_only.then(|| {
        let unit_stops = resolve_length_stops(&gradient.stops, 1.0, 0.0);
        prepare_resolved_gradient(&unit_stops, gradient.interpolation)
    });
    let inverse_radius_x_squared = 1.0 / (resolved_shape.radius_x * resolved_shape.radius_x);
    let inverse_radius_y_squared = 1.0 / (resolved_shape.radius_y * resolved_shape.radius_y);

    for y in y0..y1 {
        let py = y as f32 + 0.5;
        for x in x0..x1 {
            let px = x as f32 + 0.5;
            if !point_in_rounded_rect(px, py, layout, radius) {
                continue;
            }

            let dx = px - center_x;
            let dy = py - center_y;
            let color = if let Some(prepared_unit_gradient) = &prepared_unit_gradient {
                let normalized_distance = ((dx * dx) * inverse_radius_x_squared
                    + (dy * dy) * inverse_radius_y_squared)
                    .sqrt();
                sample_prepared_gradient(
                    prepared_unit_gradient,
                    normalized_distance,
                    gradient.repeating,
                )
            } else {
                let distance = (dx * dx + dy * dy).sqrt();
                let ray_length = radial_ray_length(dx, dy, resolved_shape);
                sample_prepared_length_gradient(
                    &prepared_length_gradient,
                    ray_length,
                    0.0,
                    distance,
                    gradient.repeating,
                )
            };
            blend_linear_pixel(buffer, width, height, x, y, color);
        }
    }
}

fn draw_conic_gradient(
    buffer: &mut [u32],
    width: usize,
    height: usize,
    layout: LayoutBox,
    radius: CornerRadius,
    gradient: &ConicGradient,
    clip: ClipRect,
) {
    let Some((x0, y0, x1, y1)) = pixel_bounds(layout, clip, width, height) else {
        return;
    };

    let Some(_first_stop) = gradient.stops.first() else {
        return;
    };

    let stops = resolve_angle_stops(&gradient.stops);
    let prepared = prepare_resolved_gradient(&stops, gradient.interpolation);
    let (center_x, center_y) = resolve_gradient_point(gradient.center, layout);

    for y in y0..y1 {
        for x in x0..x1 {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            if !point_in_rounded_rect(px, py, layout, radius) {
                continue;
            }

            let dx = px - center_x;
            let dy = py - center_y;
            let angle = if dx.abs() <= f32::EPSILON && dy.abs() <= f32::EPSILON {
                0.0
            } else {
                dx.atan2(-dy).to_degrees().rem_euclid(360.0)
            };
            let position = (angle - gradient.angle).rem_euclid(360.0);
            let color = sample_prepared_gradient(&prepared, position, gradient.repeating);
            blend_linear_pixel(buffer, width, height, x, y, color);
        }
    }
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

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn sample_gradient(
    stops: &[ResolvedGradientStop],
    position: f32,
    repeating: bool,
    interpolation: GradientInterpolation,
) -> LinearRgba {
    let prepared = prepare_resolved_gradient(stops, interpolation);
    sample_prepared_gradient(&prepared, position, repeating)
}

pub(crate) fn prepare_resolved_gradient(
    stops: &[ResolvedGradientStop],
    interpolation: GradientInterpolation,
) -> PreparedResolvedGradient {
    let first = stops.first().copied();
    let last = stops.last().copied();
    let segments = stops
        .windows(2)
        .map(|pair| {
            let start = pair[0];
            let end = pair[1];
            let span = end.position - start.position;
            PreparedResolvedGradientSegment {
                start: start.position,
                end: end.position,
                inverse_span: if span.abs() <= f32::EPSILON {
                    0.0
                } else {
                    1.0 / span
                },
                start_color: PreparedColor::new(start.color),
                end_color: PreparedColor::new(end.color),
            }
        })
        .collect();

    PreparedResolvedGradient {
        interpolation,
        first: first.map(|stop| PreparedColor::new(stop.color)),
        start: first.map(|stop| stop.position).unwrap_or(0.0),
        end: last.map(|stop| stop.position).unwrap_or(0.0),
        segments,
    }
}

pub(crate) fn sample_prepared_gradient(
    gradient: &PreparedResolvedGradient,
    position: f32,
    repeating: bool,
) -> LinearRgba {
    let Some(first) = gradient.first else {
        return LinearRgba::TRANSPARENT;
    };
    let t = normalize_gradient_position(position, gradient.start, gradient.end, repeating);
    if t <= gradient.start {
        return first.linear;
    }

    for segment in &gradient.segments {
        if t > segment.end {
            continue;
        }
        return segment.sample(t, gradient.interpolation);
    }

    gradient
        .segments
        .last()
        .map(|segment| segment.end_color.linear)
        .unwrap_or(first.linear)
}

pub(crate) fn prepare_length_gradient(
    stops: &[GradientStop<LengthPercentageValue>],
    interpolation: GradientInterpolation,
) -> PreparedLengthGradient {
    PreparedLengthGradient {
        interpolation,
        stops: stops
            .iter()
            .map(|stop| PreparedLengthGradientStop {
                color: PreparedColor::new(stop.color.to_linear_rgba()),
                px: stop.position.px,
                fraction: stop.position.fraction,
            })
            .collect(),
    }
}

pub(crate) fn sample_prepared_length_gradient(
    gradient: &PreparedLengthGradient,
    total: f32,
    origin: f32,
    position: f32,
    repeating: bool,
) -> LinearRgba {
    let Some(first) = gradient.stops.first().copied() else {
        return LinearRgba::TRANSPARENT;
    };

    let mut previous_position = origin + first.px + first.fraction * total;
    let sample_position = if repeating {
        let end = length_gradient_end_position(gradient, total, origin, previous_position);
        normalize_gradient_position(position, previous_position, end, true)
    } else {
        position
    };

    if sample_position <= previous_position {
        return first.color.linear;
    }

    let mut previous_stop = first;
    for stop in gradient.stops.iter().copied().skip(1) {
        let mut resolved_position = origin + stop.px + stop.fraction * total;
        if resolved_position < previous_position {
            resolved_position = previous_position;
        }
        if sample_position <= resolved_position {
            let span = resolved_position - previous_position;
            if span.abs() <= f32::EPSILON {
                return stop.color.linear;
            }
            let local_t = (sample_position - previous_position) / span;
            return interpolate_prepared_color(
                previous_stop.color,
                stop.color,
                local_t,
                gradient.interpolation,
            );
        }

        previous_stop = stop;
        previous_position = resolved_position;
    }

    previous_stop.color.linear
}

pub(crate) fn length_stops_use_fraction_only(
    stops: &[GradientStop<LengthPercentageValue>],
) -> bool {
    stops
        .iter()
        .all(|stop| stop.position.px.abs() <= f32::EPSILON)
}

fn gradient_direction_vector(direction: GradientDirection, layout: LayoutBox) -> (f32, f32) {
    match direction {
        GradientDirection::Angle(degrees) => {
            let radians = degrees.to_radians();
            (radians.sin(), -radians.cos())
        }
        GradientDirection::Horizontal(GradientHorizontal::Left) => (-1.0, 0.0),
        GradientDirection::Horizontal(GradientHorizontal::Right) => (1.0, 0.0),
        GradientDirection::Vertical(GradientVertical::Top) => (0.0, -1.0),
        GradientDirection::Vertical(GradientVertical::Bottom) => (0.0, 1.0),
        GradientDirection::Corner {
            horizontal,
            vertical,
        } => {
            let dx = match horizontal {
                GradientHorizontal::Left => -layout.width.max(1.0),
                GradientHorizontal::Right => layout.width.max(1.0),
            };
            let dy = match vertical {
                GradientVertical::Top => -layout.height.max(1.0),
                GradientVertical::Bottom => layout.height.max(1.0),
            };
            normalize_vector(dx, dy)
        }
    }
}

fn normalize_vector(x: f32, y: f32) -> (f32, f32) {
    let length = (x * x + y * y).sqrt();
    if length <= f32::EPSILON {
        (0.0, 1.0)
    } else {
        (x / length, y / length)
    }
}

fn gradient_projection_bounds(
    layout: LayoutBox,
    center_x: f32,
    center_y: f32,
    direction: (f32, f32),
) -> (f32, f32) {
    let corners = [
        (layout.x, layout.y),
        (layout.x + layout.width, layout.y),
        (layout.x, layout.y + layout.height),
        (layout.x + layout.width, layout.y + layout.height),
    ];
    let mut min_projection = f32::INFINITY;
    let mut max_projection = f32::NEG_INFINITY;

    for (x, y) in corners {
        let projection = ((x - center_x) * direction.0) + ((y - center_y) * direction.1);
        min_projection = min_projection.min(projection);
        max_projection = max_projection.max(projection);
    }

    (min_projection, max_projection)
}

fn resolve_gradient_point(point: GradientPoint, layout: LayoutBox) -> (f32, f32) {
    (
        layout.x + point.x.resolve(layout.width),
        layout.y + point.y.resolve(layout.height),
    )
}

fn resolve_radial_shape(
    shape: RadialShape,
    layout: LayoutBox,
    center_x: f32,
    center_y: f32,
) -> ResolvedRadialShape {
    match shape {
        RadialShape::Circle(radius) => {
            let radius = match radius {
                CircleRadius::Explicit(radius) => radius.max(0.0),
                CircleRadius::Extent(extent) => {
                    resolve_circle_extent(extent, layout, center_x, center_y)
                }
            };
            ResolvedRadialShape {
                radius_x: radius,
                radius_y: radius,
            }
        }
        RadialShape::Ellipse(radius) => match radius {
            EllipseRadius::Explicit { x, y } => ResolvedRadialShape {
                radius_x: x.resolve(layout.width).max(0.0),
                radius_y: y.resolve(layout.height).max(0.0),
            },
            EllipseRadius::Extent(extent) => {
                resolve_ellipse_extent(extent, layout, center_x, center_y)
            }
        },
    }
}

fn resolve_circle_extent(
    extent: ShapeExtent,
    layout: LayoutBox,
    center_x: f32,
    center_y: f32,
) -> f32 {
    let (left, right, top, bottom) = side_distances(layout, center_x, center_y);
    let corners = corner_offsets(left, right, top, bottom);

    match extent {
        ShapeExtent::ClosestSide => left.min(right).min(top).min(bottom),
        ShapeExtent::FarthestSide => left.max(right).max(top).max(bottom),
        ShapeExtent::ClosestCorner => corners
            .iter()
            .map(|(dx, dy)| (dx * dx + dy * dy).sqrt())
            .fold(f32::INFINITY, f32::min),
        ShapeExtent::FarthestCorner => corners
            .iter()
            .map(|(dx, dy)| (dx * dx + dy * dy).sqrt())
            .fold(0.0, f32::max),
    }
}

fn resolve_ellipse_extent(
    extent: ShapeExtent,
    layout: LayoutBox,
    center_x: f32,
    center_y: f32,
) -> ResolvedRadialShape {
    let (left, right, top, bottom) = side_distances(layout, center_x, center_y);
    let corners = corner_offsets(left, right, top, bottom);

    match extent {
        ShapeExtent::ClosestSide => ResolvedRadialShape {
            radius_x: left.min(right),
            radius_y: top.min(bottom),
        },
        ShapeExtent::FarthestSide => ResolvedRadialShape {
            radius_x: left.max(right),
            radius_y: top.max(bottom),
        },
        ShapeExtent::ClosestCorner => {
            scale_ellipse_to_corner(left.min(right), top.min(bottom), &corners, false)
        }
        ShapeExtent::FarthestCorner => {
            scale_ellipse_to_corner(left.max(right), top.max(bottom), &corners, true)
        }
    }
}

fn scale_ellipse_to_corner(
    base_radius_x: f32,
    base_radius_y: f32,
    corners: &[(f32, f32); 4],
    farthest: bool,
) -> ResolvedRadialShape {
    if base_radius_x <= f32::EPSILON || base_radius_y <= f32::EPSILON {
        return ResolvedRadialShape {
            radius_x: 0.0,
            radius_y: 0.0,
        };
    }

    let mut scale = if farthest { 0.0 } else { f32::INFINITY };
    for &(dx, dy) in corners {
        let factor = ((dx / base_radius_x).powi(2) + (dy / base_radius_y).powi(2)).sqrt();
        if farthest {
            scale = scale.max(factor);
        } else {
            scale = scale.min(factor);
        }
    }

    ResolvedRadialShape {
        radius_x: base_radius_x * scale,
        radius_y: base_radius_y * scale,
    }
}

fn side_distances(layout: LayoutBox, center_x: f32, center_y: f32) -> (f32, f32, f32, f32) {
    (
        (center_x - layout.x).abs(),
        (layout.x + layout.width - center_x).abs(),
        (center_y - layout.y).abs(),
        (layout.y + layout.height - center_y).abs(),
    )
}

fn corner_offsets(left: f32, right: f32, top: f32, bottom: f32) -> [(f32, f32); 4] {
    [(left, top), (right, top), (left, bottom), (right, bottom)]
}

fn radial_ray_length(dx: f32, dy: f32, shape: ResolvedRadialShape) -> f32 {
    if dx.abs() <= f32::EPSILON && dy.abs() <= f32::EPSILON {
        return 0.0;
    }

    let radius_x = shape.radius_x.max(f32::EPSILON);
    let radius_y = shape.radius_y.max(f32::EPSILON);
    let denominator =
        ((dx * dx) / (radius_x * radius_x) + (dy * dy) / (radius_y * radius_y)).sqrt();
    if denominator <= f32::EPSILON {
        0.0
    } else {
        (dx * dx + dy * dy).sqrt() / denominator
    }
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

#[allow(dead_code)]
fn normalize_gradient_t(t: f32, stops: &[ResolvedGradientStop], repeating: bool) -> f32 {
    let start = stops.first().map(|stop| stop.position).unwrap_or(0.0);
    let end = stops.last().map(|stop| stop.position).unwrap_or(start);
    normalize_gradient_position(t, start, end, repeating)
}

#[allow(dead_code)]
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

fn normalize_gradient_position(t: f32, start: f32, end: f32, repeating: bool) -> f32 {
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

fn length_gradient_end_position(
    gradient: &PreparedLengthGradient,
    total: f32,
    origin: f32,
    first_position: f32,
) -> f32 {
    let mut last_position = first_position;
    for stop in gradient.stops.iter().skip(1) {
        let mut resolved_position = origin + stop.px + stop.fraction * total;
        if resolved_position < last_position {
            resolved_position = last_position;
        }
        last_position = resolved_position;
    }
    last_position
}

impl PreparedResolvedGradientSegment {
    fn sample(self, position: f32, interpolation: GradientInterpolation) -> LinearRgba {
        if self.inverse_span == 0.0 {
            return self.end_color.linear;
        }
        let t = ((position - self.start) * self.inverse_span).clamp(0.0, 1.0);
        interpolate_prepared_color(self.start_color, self.end_color, t, interpolation)
    }
}

impl PreparedColor {
    fn new(linear: LinearRgba) -> Self {
        Self {
            linear,
            oklab: linear_to_oklab(linear),
        }
    }
}

fn interpolate_prepared_color(
    start: PreparedColor,
    end: PreparedColor,
    t: f32,
    interpolation: GradientInterpolation,
) -> LinearRgba {
    let t = t.clamp(0.0, 1.0);
    if matches!(interpolation, GradientInterpolation::LinearSrgb) {
        return start.linear.lerp(end.linear, t);
    }

    linear_from_oklab(
        start.oklab.lerp(end.oklab, t),
        mix(start.linear.a, end.linear.a, t),
    )
}

impl Oklab {
    fn lerp(self, other: Self, t: f32) -> Self {
        Self {
            l: mix(self.l, other.l, t),
            a: mix(self.a, other.a, t),
            b: mix(self.b, other.b, t),
        }
    }
}

fn linear_to_oklab(color: LinearRgba) -> Oklab {
    let l = 0.412_221_470_8 * color.r + 0.536_332_536_3 * color.g + 0.051_445_992_9 * color.b;
    let m = 0.211_903_498_2 * color.r + 0.680_699_545_1 * color.g + 0.107_396_956_6 * color.b;
    let s = 0.088_302_461_9 * color.r + 0.281_718_837_6 * color.g + 0.629_978_700_5 * color.b;

    let l_prime = l.cbrt();
    let m_prime = m.cbrt();
    let s_prime = s.cbrt();

    Oklab {
        l: 0.210_454_255_3 * l_prime + 0.793_617_785 * m_prime - 0.004_072_046_8 * s_prime,
        a: 1.977_998_495_1 * l_prime - 2.428_592_205 * m_prime + 0.450_593_709_9 * s_prime,
        b: 0.025_904_037_1 * l_prime + 0.782_771_766_2 * m_prime - 0.808_675_766 * s_prime,
    }
}

fn linear_from_oklab(color: Oklab, alpha: f32) -> LinearRgba {
    let l_prime = color.l + 0.396_337_777_4 * color.a + 0.215_803_757_3 * color.b;
    let m_prime = color.l - 0.105_561_345_8 * color.a - 0.063_854_172_8 * color.b;
    let s_prime = color.l - 0.089_484_177_5 * color.a - 1.291_485_548 * color.b;

    let l = l_prime * l_prime * l_prime;
    let m = m_prime * m_prime * m_prime;
    let s = s_prime * s_prime * s_prime;

    LinearRgba {
        r: 4.076_741_662_1 * l - 3.307_711_591_3 * m + 0.230_969_929_2 * s,
        g: -1.268_438_004_6 * l + 2.609_757_401_1 * m - 0.341_319_396_5 * s,
        b: -0.004_196_086_3 * l - 0.703_418_614_7 * m + 1.707_614_701 * s,
        a: alpha.clamp(0.0, 1.0),
    }
}

fn mix(start: f32, end: f32, t: f32) -> f32 {
    start + (end - start) * t
}

#[cfg(test)]
mod tests {
    use cssimpler_core::{Color, GradientInterpolation};

    use super::{
        length_stops_use_fraction_only, prepare_length_gradient, prepare_resolved_gradient,
        resolve_length_stops, sample_gradient, sample_prepared_gradient,
        sample_prepared_length_gradient, PreparedLengthGradient, ResolvedGradientStop,
    };

    fn assert_close(left: cssimpler_core::LinearRgba, right: cssimpler_core::LinearRgba) {
        let epsilon = 0.0005;
        assert!((left.r - right.r).abs() <= epsilon);
        assert!((left.g - right.g).abs() <= epsilon);
        assert!((left.b - right.b).abs() <= epsilon);
        assert!((left.a - right.a).abs() <= epsilon);
    }

    #[test]
    fn prepared_gradient_matches_direct_sampling_in_oklab_mode() {
        let stops = [
            ResolvedGradientStop {
                color: Color::rgb(255, 0, 0).to_linear_rgba(),
                position: 0.0,
            },
            ResolvedGradientStop {
                color: Color::rgb(0, 255, 255).to_linear_rgba(),
                position: 0.45,
            },
            ResolvedGradientStop {
                color: Color::rgb(255, 255, 0).to_linear_rgba(),
                position: 1.0,
            },
        ];
        let prepared = prepare_resolved_gradient(&stops, GradientInterpolation::Oklab);

        for sample in [-0.2_f32, 0.0, 0.1, 0.45, 0.7, 1.0, 1.4] {
            assert_close(
                sample_gradient(&stops, sample, false, GradientInterpolation::Oklab),
                sample_prepared_gradient(&prepared, sample, false),
            );
        }
    }

    #[test]
    fn prepared_length_gradient_matches_resolved_sampling_for_percentage_stops() {
        let stops = vec![
            cssimpler_core::GradientStop {
                color: Color::rgb(255, 0, 0),
                position: cssimpler_core::LengthPercentageValue::from_fraction(0.0),
            },
            cssimpler_core::GradientStop {
                color: Color::rgb(0, 255, 0),
                position: cssimpler_core::LengthPercentageValue::from_fraction(0.5),
            },
            cssimpler_core::GradientStop {
                color: Color::rgb(0, 0, 255),
                position: cssimpler_core::LengthPercentageValue::from_fraction(1.0),
            },
        ];
        let prepared: PreparedLengthGradient =
            prepare_length_gradient(&stops, GradientInterpolation::LinearSrgb);
        let resolved = resolve_length_stops(&stops, 200.0, 0.0);

        assert!(length_stops_use_fraction_only(&stops));
        for sample in [0.0_f32, 25.0, 80.0, 100.0, 160.0, 220.0] {
            assert_close(
                sample_gradient(&resolved, sample, false, GradientInterpolation::LinearSrgb),
                sample_prepared_length_gradient(&prepared, 200.0, 0.0, sample, false),
            );
        }
    }
}
