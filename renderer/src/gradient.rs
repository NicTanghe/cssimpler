use std::collections::HashMap;
use std::mem::size_of;
use std::sync::{Arc, Mutex, OnceLock};

use cssimpler_core::{
    AnglePercentageValue, BackgroundLayer, CircleRadius, Color, ConicGradient, CornerRadius,
    EllipseRadius, GradientDirection, GradientHorizontal, GradientInterpolation, GradientPoint,
    GradientStop, GradientVertical, LayoutBox, LengthPercentageValue, LinearGradient, LinearRgba,
    RadialGradient, RadialShape, ShapeExtent,
};

use super::shapes::{pixel_bounds, rounded_rect_row_span, transformed_rounded_rect_coverage};
use super::{
    ClipRect, blend_linear_over, clip_pixel_bounds, current_render_buffer_rows, pack_linear_rgb,
    transform::{AffineTransform, ClipState, transform_layout_bounds},
};

const MAX_GRADIENT_LAYER_CACHE_ENTRIES: usize = 16;
const MAX_GRADIENT_LAYER_CACHE_BYTES: usize = 4 * 1024 * 1024;
const MAX_SINGLE_GRADIENT_LAYER_CACHE_BYTES: usize = 256 * 1024;
const MIN_GRADIENT_LAYER_CACHE_PIXELS: usize = 512;
const MIN_GRADIENT_LAYER_CACHE_REUSES: u8 = 2;
const MAX_STATIC_GRADIENT_LAYER_CACHE_ENTRIES: usize = 4;
const MAX_STATIC_GRADIENT_LAYER_CACHE_BYTES: usize = 20 * 1024 * 1024;
const MAX_SINGLE_STATIC_GRADIENT_LAYER_CACHE_BYTES: usize = 6 * 1024 * 1024;
const MIN_STATIC_GRADIENT_LAYER_CACHE_PIXELS: usize = 250_000;
const MIN_STATIC_GRADIENT_LAYER_CACHE_REUSES: u8 = 2;

#[derive(Clone)]
struct CachedGradientLayer {
    width: usize,
    height: usize,
    pixels: CachedGradientPixels,
}

#[derive(Clone)]
enum CachedGradientPixels {
    BinaryAlpha(Vec<u32>),
    Linear(Vec<LinearRgba>),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct GradientLayerCacheKey {
    layout: LayoutCacheKey,
    radius: CornerRadiusCacheKey,
    layer: BackgroundLayerCacheKey,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct LayoutCacheKey {
    x_bits: u32,
    y_bits: u32,
    width_bits: u32,
    height_bits: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct CornerRadiusCacheKey {
    top_left_bits: u32,
    top_right_bits: u32,
    bottom_right_bits: u32,
    bottom_left_bits: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum BackgroundLayerCacheKey {
    Linear(LinearGradientCacheKey),
    Radial(RadialGradientCacheKey),
    Conic(ConicGradientCacheKey),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct LinearGradientCacheKey {
    direction: GradientDirectionCacheKey,
    interpolation: u8,
    repeating: bool,
    stops: Vec<GradientStopCacheKey<LengthPercentageCacheKey>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct RadialGradientCacheKey {
    shape: RadialShapeCacheKey,
    center: GradientPointCacheKey,
    interpolation: u8,
    repeating: bool,
    stops: Vec<GradientStopCacheKey<LengthPercentageCacheKey>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct ConicGradientCacheKey {
    angle_bits: u32,
    center: GradientPointCacheKey,
    interpolation: u8,
    repeating: bool,
    stops: Vec<GradientStopCacheKey<AnglePercentageCacheKey>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum GradientDirectionCacheKey {
    Angle(u32),
    Horizontal(u8),
    Vertical(u8),
    Corner { horizontal: u8, vertical: u8 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct GradientPointCacheKey {
    x: LengthPercentageCacheKey,
    y: LengthPercentageCacheKey,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct LengthPercentageCacheKey {
    px_bits: u32,
    fraction_bits: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct AnglePercentageCacheKey {
    degrees_bits: u32,
    turns_bits: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct GradientStopCacheKey<P> {
    color_rgba: u32,
    position: P,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum RadialShapeCacheKey {
    Circle(CircleRadiusCacheKey),
    Ellipse(EllipseRadiusCacheKey),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum CircleRadiusCacheKey {
    Explicit(u32),
    Extent(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum EllipseRadiusCacheKey {
    Explicit {
        x: LengthPercentageCacheKey,
        y: LengthPercentageCacheKey,
    },
    Extent(u8),
}

#[derive(Default)]
struct GradientLayerCache {
    total_bytes: usize,
    layers: HashMap<GradientLayerCacheKey, Arc<CachedGradientLayer>>,
    seen_counts: HashMap<GradientLayerCacheKey, u8>,
}

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
    if let Some((cached_layer, offset_x, offset_y)) =
        cached_static_gradient_layer(layout, radius, layer)
    {
        draw_cached_gradient_layer(
            buffer,
            width,
            height,
            layout,
            clip,
            cached_layer.as_ref(),
            offset_x,
            offset_y,
        );
        return;
    }

    if let Some((cached_layer, offset_x, offset_y)) = cached_gradient_layer(layout, radius, layer) {
        draw_cached_gradient_layer(
            buffer,
            width,
            height,
            layout,
            clip,
            cached_layer.as_ref(),
            offset_x,
            offset_y,
        );
        return;
    }

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

pub(crate) fn draw_background_layer_transformed(
    buffer: &mut [u32],
    width: usize,
    height: usize,
    layout: LayoutBox,
    radius: CornerRadius,
    layer: &BackgroundLayer,
    matrix: AffineTransform,
    clip_state: &ClipState,
) {
    let Some(inverse) = matrix.invert() else {
        return;
    };
    let Some(bounds) = transform_layout_bounds(layout, matrix)
        .and_then(|bounds| bounds.intersect(clip_state.coarse))
    else {
        return;
    };
    let Some((x0, y0, x1, y1)) = clip_pixel_bounds(bounds, width, height) else {
        return;
    };

    match layer {
        BackgroundLayer::LinearGradient(gradient) => {
            let Some(first_stop) = gradient.stops.first() else {
                return;
            };
            let direction = gradient_direction_vector(gradient.direction, layout);
            let center_x = layout.x + layout.width * 0.5;
            let center_y = layout.y + layout.height * 0.5;
            let (min_projection, max_projection) =
                gradient_projection_bounds(layout, center_x, center_y, direction);
            let projection_span = max_projection - min_projection;
            let prepared = (projection_span.abs() > f32::EPSILON).then(|| {
                let stops = resolve_length_stops(&gradient.stops, projection_span, min_projection);
                prepare_resolved_gradient(&stops, gradient.interpolation)
            });

            for y in y0..y1 {
                for x in x0..x1 {
                    let screen_x = x as f32 + 0.5;
                    let screen_y = y as f32 + 0.5;
                    let (source_x, source_y) = inverse.transform_point(screen_x, screen_y);
                    let coverage = transformed_rounded_rect_coverage(
                        layout, radius, inverse, clip_state, x, y,
                    );
                    if coverage == 0 {
                        continue;
                    }

                    let color = if let Some(prepared) = &prepared {
                        let projection = ((source_x - center_x) * direction.0)
                            + ((source_y - center_y) * direction.1);
                        sample_prepared_gradient(prepared, projection, gradient.repeating)
                    } else {
                        first_stop.color.to_linear_rgba()
                    };
                    blend_gradient_sample_with_coverage(
                        buffer, width, height, x, y, color, coverage,
                    );
                }
            }
        }
        BackgroundLayer::RadialGradient(gradient) => {
            let Some(first_stop) = gradient.stops.first() else {
                return;
            };

            let (center_x, center_y) = resolve_gradient_point(gradient.center, layout);
            let resolved_shape = resolve_radial_shape(gradient.shape, layout, center_x, center_y);
            let prepared_length_gradient =
                prepare_length_gradient(&gradient.stops, gradient.interpolation);
            let fraction_only = length_stops_use_fraction_only(&gradient.stops);
            let prepared_unit_gradient = fraction_only.then(|| {
                let unit_stops = resolve_length_stops(&gradient.stops, 1.0, 0.0);
                prepare_resolved_gradient(&unit_stops, gradient.interpolation)
            });
            let inverse_radius_x_squared = if resolved_shape.radius_x.abs() <= f32::EPSILON {
                0.0
            } else {
                1.0 / (resolved_shape.radius_x * resolved_shape.radius_x)
            };
            let inverse_radius_y_squared = if resolved_shape.radius_y.abs() <= f32::EPSILON {
                0.0
            } else {
                1.0 / (resolved_shape.radius_y * resolved_shape.radius_y)
            };

            for y in y0..y1 {
                for x in x0..x1 {
                    let screen_x = x as f32 + 0.5;
                    let screen_y = y as f32 + 0.5;
                    let (source_x, source_y) = inverse.transform_point(screen_x, screen_y);
                    let coverage = transformed_rounded_rect_coverage(
                        layout, radius, inverse, clip_state, x, y,
                    );
                    if coverage == 0 {
                        continue;
                    }

                    let color = if resolved_shape.radius_x <= f32::EPSILON
                        || resolved_shape.radius_y <= f32::EPSILON
                    {
                        first_stop.color.to_linear_rgba()
                    } else {
                        let dx = source_x - center_x;
                        let dy = source_y - center_y;
                        if let Some(prepared_unit_gradient) = &prepared_unit_gradient {
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
                        }
                    };
                    blend_gradient_sample_with_coverage(
                        buffer, width, height, x, y, color, coverage,
                    );
                }
            }
        }
        BackgroundLayer::ConicGradient(gradient) => {
            let Some(_first_stop) = gradient.stops.first() else {
                return;
            };
            let stops = resolve_angle_stops(&gradient.stops);
            let prepared = prepare_resolved_gradient(&stops, gradient.interpolation);
            let (center_x, center_y) = resolve_gradient_point(gradient.center, layout);
            for y in y0..y1 {
                for x in x0..x1 {
                    let screen_x = x as f32 + 0.5;
                    let screen_y = y as f32 + 0.5;
                    let (source_x, source_y) = inverse.transform_point(screen_x, screen_y);
                    let coverage = transformed_rounded_rect_coverage(
                        layout, radius, inverse, clip_state, x, y,
                    );
                    if coverage == 0 {
                        continue;
                    }

                    let dx = source_x - center_x;
                    let dy = source_y - center_y;
                    let angle = if dx.abs() <= f32::EPSILON && dy.abs() <= f32::EPSILON {
                        0.0
                    } else {
                        dx.atan2(-dy).to_degrees().rem_euclid(360.0)
                    };
                    let position = (angle - gradient.angle).rem_euclid(360.0);
                    let color = sample_prepared_gradient(&prepared, position, gradient.repeating);
                    blend_gradient_sample_with_coverage(
                        buffer, width, height, x, y, color, coverage,
                    );
                }
            }
        }
    }
}

fn gradient_layer_cache() -> &'static Mutex<GradientLayerCache> {
    static CACHE: OnceLock<Mutex<GradientLayerCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(GradientLayerCache::default()))
}

fn static_gradient_layer_cache() -> &'static Mutex<GradientLayerCache> {
    static CACHE: OnceLock<Mutex<GradientLayerCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(GradientLayerCache::default()))
}

#[cfg(test)]
fn reset_gradient_layer_cache(cache: &mut GradientLayerCache) {
    cache.total_bytes = 0;
    cache.layers.clear();
    cache.seen_counts.clear();
}

#[cfg(test)]
pub(crate) fn clear_gradient_layer_cache_for_tests() {
    let mut cache = gradient_layer_cache()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    reset_gradient_layer_cache(&mut cache);
    drop(cache);

    let mut static_cache = static_gradient_layer_cache()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    reset_gradient_layer_cache(&mut static_cache);
}

#[cfg(test)]
pub(crate) fn lock_gradient_cache_for_tests() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
}

fn cached_gradient_layer(
    layout: LayoutBox,
    radius: CornerRadius,
    layer: &BackgroundLayer,
) -> Option<(Arc<CachedGradientLayer>, i32, i32)> {
    cached_gradient_layer_with_policy(
        gradient_layer_cache(),
        layout,
        radius,
        layer,
        MIN_GRADIENT_LAYER_CACHE_REUSES,
        MAX_GRADIENT_LAYER_CACHE_ENTRIES,
        MAX_GRADIENT_LAYER_CACHE_BYTES,
        MAX_SINGLE_GRADIENT_LAYER_CACHE_BYTES,
        should_prerasterize_gradient_layer,
        rasterize_gradient_layer,
    )
}

fn cached_static_gradient_layer(
    layout: LayoutBox,
    radius: CornerRadius,
    layer: &BackgroundLayer,
) -> Option<(Arc<CachedGradientLayer>, i32, i32)> {
    cached_gradient_layer_with_policy(
        static_gradient_layer_cache(),
        layout,
        radius,
        layer,
        MIN_STATIC_GRADIENT_LAYER_CACHE_REUSES,
        MAX_STATIC_GRADIENT_LAYER_CACHE_ENTRIES,
        MAX_STATIC_GRADIENT_LAYER_CACHE_BYTES,
        MAX_SINGLE_STATIC_GRADIENT_LAYER_CACHE_BYTES,
        should_prerasterize_static_gradient_layer,
        rasterize_static_gradient_layer,
    )
}

fn cached_gradient_layer_with_policy(
    cache: &'static Mutex<GradientLayerCache>,
    layout: LayoutBox,
    radius: CornerRadius,
    layer: &BackgroundLayer,
    min_reuses: u8,
    max_entries: usize,
    max_total_bytes: usize,
    max_raster_bytes: usize,
    should_prerasterize: fn(LayoutBox) -> bool,
    rasterize: fn(LayoutBox, CornerRadius, &BackgroundLayer) -> Option<CachedGradientLayer>,
) -> Option<(Arc<CachedGradientLayer>, i32, i32)> {
    let (relative_layout, offset_x, offset_y) = split_layout_for_gradient_cache(layout);
    let key = GradientLayerCacheKey {
        layout: layout_cache_key(relative_layout),
        radius: corner_radius_cache_key(radius),
        layer: background_layer_cache_key(layer),
    };

    if let Some(cached) = cache
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .layers
        .get(&key)
        .cloned()
    {
        return Some((cached, offset_x, offset_y));
    }

    if !should_prerasterize(relative_layout) {
        return None;
    }

    let seen_enough = {
        let mut cache_guard = cache.lock().unwrap_or_else(|poison| poison.into_inner());
        let seen = cache_guard.seen_counts.entry(key.clone()).or_insert(0);
        *seen = seen.saturating_add(1);
        *seen >= min_reuses
    };
    if !seen_enough {
        return None;
    }

    let raster = Arc::new(rasterize(relative_layout, radius, layer)?);
    let raster_bytes = raster.byte_len();
    if raster_bytes > max_raster_bytes {
        return None;
    }
    let mut cache_guard = cache.lock().unwrap_or_else(|poison| poison.into_inner());
    if let Some(cached) = cache_guard.layers.get(&key).cloned() {
        return Some((cached, offset_x, offset_y));
    }

    if cache_guard.layers.len() >= max_entries
        || cache_guard.total_bytes.saturating_add(raster_bytes) > max_total_bytes
    {
        return None;
    }
    cache_guard.total_bytes = cache_guard.total_bytes.saturating_add(raster_bytes);
    cache_guard.layers.insert(key, raster.clone());
    Some((raster, offset_x, offset_y))
}

fn gradient_layer_dimensions(layout: LayoutBox) -> (usize, usize) {
    (
        ((layout.x + layout.width).ceil() - layout.x.floor()).max(0.0) as usize,
        ((layout.y + layout.height).ceil() - layout.y.floor()).max(0.0) as usize,
    )
}

fn should_prerasterize_gradient_layer(layout: LayoutBox) -> bool {
    let (width, height) = gradient_layer_dimensions(layout);
    let pixel_count = width.saturating_mul(height);
    pixel_count >= MIN_GRADIENT_LAYER_CACHE_PIXELS
        && pixel_count.saturating_mul(size_of::<LinearRgba>())
            <= MAX_SINGLE_GRADIENT_LAYER_CACHE_BYTES
}

fn should_prerasterize_static_gradient_layer(layout: LayoutBox) -> bool {
    let (width, height) = gradient_layer_dimensions(layout);
    let pixel_count = width.saturating_mul(height);
    pixel_count >= MIN_STATIC_GRADIENT_LAYER_CACHE_PIXELS
        && pixel_count.saturating_mul(size_of::<LinearRgba>())
            <= MAX_SINGLE_STATIC_GRADIENT_LAYER_CACHE_BYTES
}

fn split_layout_for_gradient_cache(layout: LayoutBox) -> (LayoutBox, i32, i32) {
    let offset_x = layout.x.floor() as i32;
    let offset_y = layout.y.floor() as i32;
    (
        LayoutBox::new(
            layout.x - offset_x as f32,
            layout.y - offset_y as f32,
            layout.width,
            layout.height,
        ),
        offset_x,
        offset_y,
    )
}

fn layout_cache_key(layout: LayoutBox) -> LayoutCacheKey {
    LayoutCacheKey {
        x_bits: layout.x.to_bits(),
        y_bits: layout.y.to_bits(),
        width_bits: layout.width.to_bits(),
        height_bits: layout.height.to_bits(),
    }
}

fn corner_radius_cache_key(radius: CornerRadius) -> CornerRadiusCacheKey {
    CornerRadiusCacheKey {
        top_left_bits: radius.top_left.to_bits(),
        top_right_bits: radius.top_right.to_bits(),
        bottom_right_bits: radius.bottom_right.to_bits(),
        bottom_left_bits: radius.bottom_left.to_bits(),
    }
}

fn background_layer_cache_key(layer: &BackgroundLayer) -> BackgroundLayerCacheKey {
    match layer {
        BackgroundLayer::LinearGradient(gradient) => {
            BackgroundLayerCacheKey::Linear(LinearGradientCacheKey {
                direction: gradient_direction_cache_key(gradient.direction),
                interpolation: interpolation_cache_key(gradient.interpolation),
                repeating: gradient.repeating,
                stops: gradient
                    .stops
                    .iter()
                    .map(|stop| GradientStopCacheKey {
                        color_rgba: color_rgba_key(stop.color),
                        position: length_percentage_cache_key(stop.position),
                    })
                    .collect(),
            })
        }
        BackgroundLayer::RadialGradient(gradient) => {
            BackgroundLayerCacheKey::Radial(RadialGradientCacheKey {
                shape: radial_shape_cache_key(gradient.shape),
                center: gradient_point_cache_key(gradient.center),
                interpolation: interpolation_cache_key(gradient.interpolation),
                repeating: gradient.repeating,
                stops: gradient
                    .stops
                    .iter()
                    .map(|stop| GradientStopCacheKey {
                        color_rgba: color_rgba_key(stop.color),
                        position: length_percentage_cache_key(stop.position),
                    })
                    .collect(),
            })
        }
        BackgroundLayer::ConicGradient(gradient) => {
            BackgroundLayerCacheKey::Conic(ConicGradientCacheKey {
                angle_bits: gradient.angle.to_bits(),
                center: gradient_point_cache_key(gradient.center),
                interpolation: interpolation_cache_key(gradient.interpolation),
                repeating: gradient.repeating,
                stops: gradient
                    .stops
                    .iter()
                    .map(|stop| GradientStopCacheKey {
                        color_rgba: color_rgba_key(stop.color),
                        position: angle_percentage_cache_key(stop.position),
                    })
                    .collect(),
            })
        }
    }
}

fn interpolation_cache_key(interpolation: GradientInterpolation) -> u8 {
    match interpolation {
        GradientInterpolation::LinearSrgb => 0,
        GradientInterpolation::Oklab => 1,
    }
}

fn color_rgba_key(color: Color) -> u32 {
    (u32::from(color.r) << 24)
        | (u32::from(color.g) << 16)
        | (u32::from(color.b) << 8)
        | u32::from(color.a)
}

fn gradient_direction_cache_key(direction: GradientDirection) -> GradientDirectionCacheKey {
    match direction {
        GradientDirection::Angle(degrees) => GradientDirectionCacheKey::Angle(degrees.to_bits()),
        GradientDirection::Horizontal(horizontal) => {
            GradientDirectionCacheKey::Horizontal(horizontal_cache_key(horizontal))
        }
        GradientDirection::Vertical(vertical) => {
            GradientDirectionCacheKey::Vertical(vertical_cache_key(vertical))
        }
        GradientDirection::Corner {
            horizontal,
            vertical,
        } => GradientDirectionCacheKey::Corner {
            horizontal: horizontal_cache_key(horizontal),
            vertical: vertical_cache_key(vertical),
        },
    }
}

fn gradient_point_cache_key(point: GradientPoint) -> GradientPointCacheKey {
    GradientPointCacheKey {
        x: length_percentage_cache_key(point.x),
        y: length_percentage_cache_key(point.y),
    }
}

fn length_percentage_cache_key(value: LengthPercentageValue) -> LengthPercentageCacheKey {
    LengthPercentageCacheKey {
        px_bits: value.px.to_bits(),
        fraction_bits: value.fraction.to_bits(),
    }
}

fn angle_percentage_cache_key(value: AnglePercentageValue) -> AnglePercentageCacheKey {
    AnglePercentageCacheKey {
        degrees_bits: value.degrees.to_bits(),
        turns_bits: value.turns.to_bits(),
    }
}

fn radial_shape_cache_key(shape: RadialShape) -> RadialShapeCacheKey {
    match shape {
        RadialShape::Circle(radius) => RadialShapeCacheKey::Circle(circle_radius_cache_key(radius)),
        RadialShape::Ellipse(radius) => {
            RadialShapeCacheKey::Ellipse(ellipse_radius_cache_key(radius))
        }
    }
}

fn circle_radius_cache_key(radius: CircleRadius) -> CircleRadiusCacheKey {
    match radius {
        CircleRadius::Explicit(radius) => CircleRadiusCacheKey::Explicit(radius.to_bits()),
        CircleRadius::Extent(extent) => {
            CircleRadiusCacheKey::Extent(shape_extent_cache_key(extent))
        }
    }
}

fn ellipse_radius_cache_key(radius: EllipseRadius) -> EllipseRadiusCacheKey {
    match radius {
        EllipseRadius::Explicit { x, y } => EllipseRadiusCacheKey::Explicit {
            x: length_percentage_cache_key(x),
            y: length_percentage_cache_key(y),
        },
        EllipseRadius::Extent(extent) => {
            EllipseRadiusCacheKey::Extent(shape_extent_cache_key(extent))
        }
    }
}

fn horizontal_cache_key(horizontal: GradientHorizontal) -> u8 {
    match horizontal {
        GradientHorizontal::Left => 0,
        GradientHorizontal::Right => 1,
    }
}

fn vertical_cache_key(vertical: GradientVertical) -> u8 {
    match vertical {
        GradientVertical::Top => 0,
        GradientVertical::Bottom => 1,
    }
}

fn shape_extent_cache_key(extent: ShapeExtent) -> u8 {
    match extent {
        ShapeExtent::ClosestSide => 0,
        ShapeExtent::FarthestSide => 1,
        ShapeExtent::ClosestCorner => 2,
        ShapeExtent::FarthestCorner => 3,
    }
}

impl CachedGradientLayer {
    fn new(width: usize, height: usize, pixels: Vec<LinearRgba>) -> Self {
        let binary_alpha = pixels
            .iter()
            .all(|pixel| pixel.a <= f32::EPSILON || pixel.a >= 1.0 - f32::EPSILON);
        let pixels = if binary_alpha {
            CachedGradientPixels::BinaryAlpha(
                pixels
                    .into_iter()
                    .map(pack_binary_alpha_pixel)
                    .collect::<Vec<_>>(),
            )
        } else {
            CachedGradientPixels::Linear(pixels)
        };
        Self {
            width,
            height,
            pixels,
        }
    }

    fn byte_len(&self) -> usize {
        match &self.pixels {
            CachedGradientPixels::BinaryAlpha(pixels) => pixels.len() * size_of::<u32>(),
            CachedGradientPixels::Linear(pixels) => pixels.len() * size_of::<LinearRgba>(),
        }
    }
}

fn pack_binary_alpha_pixel(color: LinearRgba) -> u32 {
    if color.a <= f32::EPSILON {
        0
    } else {
        pack_linear_rgb(LinearRgba { a: 1.0, ..color })
    }
}

fn rasterize_gradient_layer(
    layout: LayoutBox,
    radius: CornerRadius,
    layer: &BackgroundLayer,
) -> Option<CachedGradientLayer> {
    let (width, height) = gradient_layer_dimensions(layout);
    if width == 0 || height == 0 {
        return None;
    }

    let mut pixels = vec![LinearRgba::TRANSPARENT; width.saturating_mul(height)];
    match layer {
        BackgroundLayer::LinearGradient(gradient) => {
            rasterize_linear_gradient(&mut pixels, width, height, layout, radius, gradient)
        }
        BackgroundLayer::RadialGradient(gradient) => {
            rasterize_radial_gradient(&mut pixels, width, height, layout, radius, gradient)
        }
        BackgroundLayer::ConicGradient(gradient) => {
            rasterize_conic_gradient(&mut pixels, width, height, layout, radius, gradient)
        }
    }
    Some(CachedGradientLayer::new(width, height, pixels))
}

fn rasterize_static_gradient_layer(
    layout: LayoutBox,
    radius: CornerRadius,
    layer: &BackgroundLayer,
) -> Option<CachedGradientLayer> {
    rasterize_gradient_layer(layout, radius, layer)
}

fn draw_cached_gradient_layer(
    buffer: &mut [u32],
    width: usize,
    height: usize,
    layout: LayoutBox,
    clip: ClipRect,
    cached: &CachedGradientLayer,
    offset_x: i32,
    offset_y: i32,
) {
    let Some((x0, y0, x1, y1)) = pixel_bounds(layout, clip, width, height) else {
        return;
    };
    let rows = current_render_buffer_rows();
    let draw_y0 = rows.start.max(y0 as usize);
    let draw_y1 = rows.end.min(y1 as usize);
    if x0 >= x1 || draw_y0 >= draw_y1 {
        return;
    }

    match &cached.pixels {
        CachedGradientPixels::BinaryAlpha(pixels) => {
            for y in draw_y0..draw_y1 {
                let global_y = y as i32;
                let dest_row_start = (y - rows.start) * width;
                let src_y = (global_y - offset_y) as usize;
                let src_row_start = src_y * cached.width;
                debug_assert!(src_y < cached.height);
                for x in x0..x1 {
                    let source = pixels[src_row_start + (x - offset_x) as usize];
                    if source == 0 {
                        continue;
                    }
                    buffer[dest_row_start + x as usize] = source;
                }
            }
        }
        CachedGradientPixels::Linear(pixels) => {
            for y in draw_y0..draw_y1 {
                let global_y = y as i32;
                let dest_row_start = (y - rows.start) * width;
                let src_y = (global_y - offset_y) as usize;
                let src_row_start = src_y * cached.width;
                debug_assert!(src_y < cached.height);
                for x in x0..x1 {
                    let source = pixels[src_row_start + (x - offset_x) as usize];
                    let alpha = source.a.clamp(0.0, 1.0);
                    if alpha <= f32::EPSILON {
                        continue;
                    }
                    let buffer_index = dest_row_start + x as usize;
                    if alpha >= 1.0 - f32::EPSILON {
                        buffer[buffer_index] = pack_linear_rgb(LinearRgba { a: 1.0, ..source });
                    } else {
                        blend_linear_over(buffer, buffer_index, LinearRgba { a: alpha, ..source });
                    }
                }
            }
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
        fill_gradient_rounded_rect(
            buffer,
            width,
            height,
            layout,
            radius,
            first_stop.color.to_linear_rgba(),
            clip,
        );
        return;
    }

    let stops = resolve_length_stops(&gradient.stops, projection_span, min_projection);
    let prepared = prepare_resolved_gradient(&stops, gradient.interpolation);
    let projection_step = direction.0;
    for y in y0..y1 {
        let Some((span_x0, span_x1)) = rounded_rect_row_span(layout, radius, y, x0, x1) else {
            continue;
        };
        let py = y as f32 + 0.5;
        let mut projection =
            ((span_x0 as f32 + 0.5 - center_x) * direction.0) + ((py - center_y) * direction.1);
        for x in span_x0..span_x1 {
            let color = sample_prepared_gradient(&prepared, projection, gradient.repeating);
            blend_gradient_sample(buffer, width, height, x, y, color);
            projection += projection_step;
        }
    }
}

fn rasterize_linear_gradient(
    pixels: &mut [LinearRgba],
    width: usize,
    height: usize,
    layout: LayoutBox,
    radius: CornerRadius,
    gradient: &LinearGradient,
) {
    let Some((x0, y0, x1, y1)) = pixel_bounds(
        layout,
        ClipRect::full(width as f32, height as f32),
        width,
        height,
    ) else {
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
        fill_rasterized_rounded_rect(
            pixels,
            width,
            height,
            layout,
            radius,
            first_stop.color.to_linear_rgba(),
        );
        return;
    }

    let stops = resolve_length_stops(&gradient.stops, projection_span, min_projection);
    let prepared = prepare_resolved_gradient(&stops, gradient.interpolation);
    let projection_step = direction.0;
    for y in y0..y1 {
        let Some((span_x0, span_x1)) = rounded_rect_row_span(layout, radius, y, x0, x1) else {
            continue;
        };
        let py = y as f32 + 0.5;
        let mut projection =
            ((span_x0 as f32 + 0.5 - center_x) * direction.0) + ((py - center_y) * direction.1);
        for x in span_x0..span_x1 {
            let color = sample_prepared_gradient(&prepared, projection, gradient.repeating);
            write_raster_pixel(pixels, width, x, y, color);
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
        fill_gradient_rounded_rect(
            buffer,
            width,
            height,
            layout,
            radius,
            first_stop.color.to_linear_rgba(),
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
        let Some((span_x0, span_x1)) = rounded_rect_row_span(layout, radius, y, x0, x1) else {
            continue;
        };
        let py = y as f32 + 0.5;
        for x in span_x0..span_x1 {
            let px = x as f32 + 0.5;
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
            blend_gradient_sample(buffer, width, height, x, y, color);
        }
    }
}

fn rasterize_radial_gradient(
    pixels: &mut [LinearRgba],
    width: usize,
    height: usize,
    layout: LayoutBox,
    radius: CornerRadius,
    gradient: &RadialGradient,
) {
    let Some((x0, y0, x1, y1)) = pixel_bounds(
        layout,
        ClipRect::full(width as f32, height as f32),
        width,
        height,
    ) else {
        return;
    };

    let Some(first_stop) = gradient.stops.first() else {
        return;
    };

    let (center_x, center_y) = resolve_gradient_point(gradient.center, layout);
    let resolved_shape = resolve_radial_shape(gradient.shape, layout, center_x, center_y);
    if resolved_shape.radius_x <= f32::EPSILON || resolved_shape.radius_y <= f32::EPSILON {
        fill_rasterized_rounded_rect(
            pixels,
            width,
            height,
            layout,
            radius,
            first_stop.color.to_linear_rgba(),
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
        let Some((span_x0, span_x1)) = rounded_rect_row_span(layout, radius, y, x0, x1) else {
            continue;
        };
        let py = y as f32 + 0.5;
        for x in span_x0..span_x1 {
            let px = x as f32 + 0.5;
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
            write_raster_pixel(pixels, width, x, y, color);
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
        let Some((span_x0, span_x1)) = rounded_rect_row_span(layout, radius, y, x0, x1) else {
            continue;
        };
        for x in span_x0..span_x1 {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let dx = px - center_x;
            let dy = py - center_y;
            let angle = if dx.abs() <= f32::EPSILON && dy.abs() <= f32::EPSILON {
                0.0
            } else {
                dx.atan2(-dy).to_degrees().rem_euclid(360.0)
            };
            let position = (angle - gradient.angle).rem_euclid(360.0);
            let color = sample_prepared_gradient(&prepared, position, gradient.repeating);
            blend_gradient_sample(buffer, width, height, x, y, color);
        }
    }
}

fn rasterize_conic_gradient(
    pixels: &mut [LinearRgba],
    width: usize,
    height: usize,
    layout: LayoutBox,
    radius: CornerRadius,
    gradient: &ConicGradient,
) {
    let Some((x0, y0, x1, y1)) = pixel_bounds(
        layout,
        ClipRect::full(width as f32, height as f32),
        width,
        height,
    ) else {
        return;
    };

    let Some(_first_stop) = gradient.stops.first() else {
        return;
    };

    let stops = resolve_angle_stops(&gradient.stops);
    let prepared = prepare_resolved_gradient(&stops, gradient.interpolation);
    let (center_x, center_y) = resolve_gradient_point(gradient.center, layout);

    for y in y0..y1 {
        let Some((span_x0, span_x1)) = rounded_rect_row_span(layout, radius, y, x0, x1) else {
            continue;
        };
        for x in span_x0..span_x1 {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let dx = px - center_x;
            let dy = py - center_y;
            let angle = if dx.abs() <= f32::EPSILON && dy.abs() <= f32::EPSILON {
                0.0
            } else {
                dx.atan2(-dy).to_degrees().rem_euclid(360.0)
            };
            let position = (angle - gradient.angle).rem_euclid(360.0);
            let color = sample_prepared_gradient(&prepared, position, gradient.repeating);
            write_raster_pixel(pixels, width, x, y, color);
        }
    }
}

fn fill_gradient_rounded_rect(
    buffer: &mut [u32],
    width: usize,
    height: usize,
    layout: LayoutBox,
    radius: CornerRadius,
    color: LinearRgba,
    clip: ClipRect,
) {
    let Some((x0, y0, x1, y1)) = pixel_bounds(layout, clip, width, height) else {
        return;
    };
    for y in y0..y1 {
        let Some((span_x0, span_x1)) = rounded_rect_row_span(layout, radius, y, x0, x1) else {
            continue;
        };
        for x in span_x0..span_x1 {
            blend_gradient_sample(buffer, width, height, x, y, color);
        }
    }
}

fn fill_rasterized_rounded_rect(
    pixels: &mut [LinearRgba],
    width: usize,
    height: usize,
    layout: LayoutBox,
    radius: CornerRadius,
    color: LinearRgba,
) {
    let Some((x0, y0, x1, y1)) = pixel_bounds(
        layout,
        ClipRect::full(width as f32, height as f32),
        width,
        height,
    ) else {
        return;
    };
    for y in y0..y1 {
        let Some((span_x0, span_x1)) = rounded_rect_row_span(layout, radius, y, x0, x1) else {
            continue;
        };
        for x in span_x0..span_x1 {
            write_raster_pixel(pixels, width, x, y, color);
        }
    }
}

fn write_raster_pixel(pixels: &mut [LinearRgba], width: usize, x: i32, y: i32, color: LinearRgba) {
    let index = y as usize * width + x as usize;
    pixels[index] = color;
}

fn blend_gradient_sample(
    buffer: &mut [u32],
    width: usize,
    height: usize,
    x: i32,
    y: i32,
    color: LinearRgba,
) {
    let alpha = color.a.clamp(0.0, 1.0);
    if alpha <= f32::EPSILON || x < 0 || y < 0 || x >= width as i32 || y >= height as i32 {
        return;
    }

    let rows = current_render_buffer_rows();
    let y = y as usize;
    if y < rows.start || y >= rows.end {
        return;
    }

    let index = (y - rows.start) * width + x as usize;
    if alpha >= 1.0 - f32::EPSILON {
        buffer[index] = pack_linear_rgb(LinearRgba { a: 1.0, ..color });
    } else {
        blend_linear_over(buffer, index, LinearRgba { a: alpha, ..color });
    }
}

fn blend_gradient_sample_with_coverage(
    buffer: &mut [u32],
    width: usize,
    height: usize,
    x: i32,
    y: i32,
    color: LinearRgba,
    coverage: u8,
) {
    let coverage = coverage as f32 / 255.0;
    if coverage <= f32::EPSILON {
        return;
    }

    blend_gradient_sample(
        buffer,
        width,
        height,
        x,
        y,
        LinearRgba {
            a: color.a * coverage,
            ..color
        },
    );
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
    use std::sync::Arc;

    use cssimpler_core::{
        AnglePercentageValue, BackgroundLayer, CircleRadius, Color, ConicGradient, CornerRadius,
        GradientDirection, GradientHorizontal, GradientInterpolation, GradientPoint, GradientStop,
        LayoutBox, LengthPercentageValue, LinearGradient, RadialGradient, RadialShape,
    };

    use super::{
        PreparedLengthGradient, ResolvedGradientStop, cached_gradient_layer,
        cached_static_gradient_layer, clear_gradient_layer_cache_for_tests,
        length_stops_use_fraction_only, lock_gradient_cache_for_tests, prepare_length_gradient,
        prepare_resolved_gradient, resolve_length_stops, sample_gradient, sample_prepared_gradient,
        sample_prepared_length_gradient,
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

    fn linear_gradient_layer() -> BackgroundLayer {
        BackgroundLayer::LinearGradient(LinearGradient {
            direction: GradientDirection::Horizontal(GradientHorizontal::Right),
            interpolation: GradientInterpolation::Oklab,
            repeating: false,
            stops: vec![
                GradientStop {
                    color: Color::rgb(14, 165, 233),
                    position: LengthPercentageValue::from_fraction(0.0),
                },
                GradientStop {
                    color: Color::rgb(244, 114, 182),
                    position: LengthPercentageValue::from_fraction(1.0),
                },
            ],
        })
    }

    fn radial_gradient_layer() -> BackgroundLayer {
        BackgroundLayer::RadialGradient(RadialGradient {
            shape: RadialShape::Circle(CircleRadius::Explicit(36.0)),
            center: GradientPoint::CENTER,
            interpolation: GradientInterpolation::Oklab,
            repeating: false,
            stops: vec![
                GradientStop {
                    color: Color::rgb(251, 191, 36),
                    position: LengthPercentageValue::from_fraction(0.0),
                },
                GradientStop {
                    color: Color::rgba(15, 23, 42, 0),
                    position: LengthPercentageValue::from_fraction(1.0),
                },
            ],
        })
    }

    fn conic_gradient_layer() -> BackgroundLayer {
        BackgroundLayer::ConicGradient(ConicGradient {
            angle: 20.0,
            center: GradientPoint::CENTER,
            interpolation: GradientInterpolation::Oklab,
            repeating: false,
            stops: vec![
                GradientStop {
                    color: Color::rgb(45, 212, 191),
                    position: AnglePercentageValue::from_degrees(0.0),
                },
                GradientStop {
                    color: Color::rgb(99, 102, 241),
                    position: AnglePercentageValue::from_degrees(180.0),
                },
                GradientStop {
                    color: Color::rgb(244, 114, 182),
                    position: AnglePercentageValue::from_degrees(360.0),
                },
            ],
        })
    }

    #[test]
    fn linear_gradient_cache_reuses_rasters_for_integer_translation() {
        let _cache_guard = lock_gradient_cache_for_tests();
        clear_gradient_layer_cache_for_tests();
        let layer = linear_gradient_layer();
        let radius = CornerRadius::all(12.0);

        assert!(
            cached_gradient_layer(LayoutBox::new(10.25, 20.75, 96.0, 48.0), radius, &layer)
                .is_none()
        );
        let (first, first_offset_x, first_offset_y) =
            cached_gradient_layer(LayoutBox::new(10.25, 20.75, 96.0, 48.0), radius, &layer)
                .expect("linear gradient raster should cache");
        let (second, second_offset_x, second_offset_y) =
            cached_gradient_layer(LayoutBox::new(74.25, 44.75, 96.0, 48.0), radius, &layer)
                .expect("translated linear gradient raster should cache");

        assert!(Arc::ptr_eq(&first, &second));
        assert_ne!(
            (first_offset_x, first_offset_y),
            (second_offset_x, second_offset_y)
        );
    }

    #[test]
    fn radial_gradient_cache_reuses_rasters_for_integer_translation() {
        let _cache_guard = lock_gradient_cache_for_tests();
        clear_gradient_layer_cache_for_tests();
        let layer = radial_gradient_layer();
        let radius = CornerRadius::all(16.0);

        assert!(
            cached_gradient_layer(LayoutBox::new(18.5, 28.25, 72.0, 72.0), radius, &layer)
                .is_none()
        );
        let (first, _, _) =
            cached_gradient_layer(LayoutBox::new(18.5, 28.25, 72.0, 72.0), radius, &layer)
                .expect("radial gradient raster should cache");
        let (second, _, _) =
            cached_gradient_layer(LayoutBox::new(82.5, 60.25, 72.0, 72.0), radius, &layer)
                .expect("translated radial gradient raster should cache");

        assert!(Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn conic_gradient_cache_reuses_rasters_for_integer_translation() {
        let _cache_guard = lock_gradient_cache_for_tests();
        clear_gradient_layer_cache_for_tests();
        let layer = conic_gradient_layer();

        assert!(
            cached_gradient_layer(
                LayoutBox::new(24.25, 12.5, 88.0, 88.0),
                CornerRadius::ZERO,
                &layer
            )
            .is_none()
        );
        let (first, _, _) = cached_gradient_layer(
            LayoutBox::new(24.25, 12.5, 88.0, 88.0),
            CornerRadius::ZERO,
            &layer,
        )
        .expect("conic gradient raster should cache");
        let (second, _, _) = cached_gradient_layer(
            LayoutBox::new(88.25, 36.5, 88.0, 88.0),
            CornerRadius::ZERO,
            &layer,
        )
        .expect("translated conic gradient raster should cache");

        assert!(Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn gradient_cache_invalidates_when_size_or_gradient_changes() {
        let _cache_guard = lock_gradient_cache_for_tests();
        clear_gradient_layer_cache_for_tests();
        let radius = CornerRadius::all(10.0);
        let base_layer = linear_gradient_layer();

        assert!(
            cached_gradient_layer(
                LayoutBox::new(10.25, 12.75, 96.0, 40.0),
                radius,
                &base_layer
            )
            .is_none()
        );
        let (base, _, _) = cached_gradient_layer(
            LayoutBox::new(10.25, 12.75, 96.0, 40.0),
            radius,
            &base_layer,
        )
        .expect("base gradient raster should cache");
        assert!(
            cached_gradient_layer(
                LayoutBox::new(10.25, 12.75, 120.0, 40.0),
                radius,
                &base_layer
            )
            .is_none()
        );
        let (resized, _, _) = cached_gradient_layer(
            LayoutBox::new(10.25, 12.75, 120.0, 40.0),
            radius,
            &base_layer,
        )
        .expect("resized gradient raster should cache");
        let changed_layer = BackgroundLayer::LinearGradient(LinearGradient {
            direction: GradientDirection::Angle(45.0),
            ..match base_layer {
                BackgroundLayer::LinearGradient(ref gradient) => gradient.clone(),
                _ => unreachable!(),
            }
        });
        assert!(
            cached_gradient_layer(
                LayoutBox::new(10.25, 12.75, 96.0, 40.0),
                radius,
                &changed_layer
            )
            .is_none()
        );
        let (changed, _, _) = cached_gradient_layer(
            LayoutBox::new(10.25, 12.75, 96.0, 40.0),
            radius,
            &changed_layer,
        )
        .expect("changed gradient raster should cache");

        assert!(!Arc::ptr_eq(&base, &resized));
        assert!(!Arc::ptr_eq(&base, &changed));
    }

    #[test]
    fn oversized_gradient_layers_skip_preraster_cache() {
        let _cache_guard = lock_gradient_cache_for_tests();
        clear_gradient_layer_cache_for_tests();
        let layer = linear_gradient_layer();

        for _ in 0..4 {
            assert!(
                cached_gradient_layer(
                    LayoutBox::new(0.0, 0.0, 308.0, 272.0),
                    CornerRadius::all(18.0),
                    &layer
                )
                .is_none()
            );
        }
    }

    #[test]
    fn large_gradient_layers_use_static_preraster_cache_after_reuse() {
        let _cache_guard = lock_gradient_cache_for_tests();
        clear_gradient_layer_cache_for_tests();
        let layer = radial_gradient_layer();
        let radius = CornerRadius::all(20.0);

        assert!(
            cached_static_gradient_layer(LayoutBox::new(12.5, 18.25, 512.0, 512.0), radius, &layer)
                .is_none()
        );
        let (first, first_offset_x, first_offset_y) =
            cached_static_gradient_layer(LayoutBox::new(12.5, 18.25, 512.0, 512.0), radius, &layer)
                .expect("large gradient raster should enter the static cache");
        let (second, second_offset_x, second_offset_y) =
            cached_static_gradient_layer(LayoutBox::new(76.5, 50.25, 512.0, 512.0), radius, &layer)
                .expect("translated large gradient raster should reuse the static cache");

        assert!(Arc::ptr_eq(&first, &second));
        assert_ne!(
            (first_offset_x, first_offset_y),
            (second_offset_x, second_offset_y)
        );
    }

    #[test]
    fn oversized_static_gradient_layers_skip_preraster_cache() {
        let _cache_guard = lock_gradient_cache_for_tests();
        clear_gradient_layer_cache_for_tests();
        let layer = radial_gradient_layer();

        for _ in 0..4 {
            assert!(
                cached_static_gradient_layer(
                    LayoutBox::new(0.0, 0.0, 2048.0, 1024.0),
                    CornerRadius::all(24.0),
                    &layer
                )
                .is_none()
            );
        }
    }
}
