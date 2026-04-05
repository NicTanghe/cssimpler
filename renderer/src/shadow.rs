use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use cssimpler_core::{Color, CornerRadius, LayoutBox, TextStrokeStyle};

use super::{ClipRect, PreparedBlendColor, blend_prepared_pixel, scale_alpha};
use super::shapes::{
    clip_pixel_bounds, draw_rounded_rect, expand_corner_radius, expand_layout, non_empty_layout_clip,
    offset_layout, point_in_rounded_rect,
};

const MAX_SHADOW_MASK_CACHE_ENTRIES: usize = 256;

#[derive(Clone)]
pub(crate) struct ShadowMask {
    origin_x: i32,
    origin_y: i32,
    width: usize,
    height: usize,
    alpha: Vec<u8>,
}

impl ShadowMask {
    fn new(origin_x: i32, origin_y: i32, width: i32, height: i32) -> Self {
        let width = width.max(0) as usize;
        let height = height.max(0) as usize;
        Self {
            origin_x,
            origin_y,
            width,
            height,
            alpha: vec![0; width.saturating_mul(height)],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct ShadowMaskCacheKey {
    x_bits: u32,
    y_bits: u32,
    width_bits: u32,
    height_bits: u32,
    top_left_bits: u32,
    top_right_bits: u32,
    bottom_right_bits: u32,
    bottom_left_bits: u32,
    blur_bits: u32,
}

#[derive(Default)]
struct ShadowMaskCache {
    masks: HashMap<ShadowMaskCacheKey, Arc<ShadowMask>>,
}

fn shadow_mask_cache() -> &'static Mutex<ShadowMaskCache> {
    static CACHE: OnceLock<Mutex<ShadowMaskCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(ShadowMaskCache::default()))
}

#[cfg(test)]
pub(crate) fn clear_shadow_mask_cache_for_tests() {
    let mut cache = shadow_mask_cache()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    cache.masks.clear();
}

fn split_layout_for_shadow_cache(layout: LayoutBox) -> (LayoutBox, i32, i32) {
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

fn shadow_mask_cache_key(
    layout: LayoutBox,
    radius: CornerRadius,
    blur_radius: f32,
) -> ShadowMaskCacheKey {
    ShadowMaskCacheKey {
        x_bits: layout.x.to_bits(),
        y_bits: layout.y.to_bits(),
        width_bits: layout.width.to_bits(),
        height_bits: layout.height.to_bits(),
        top_left_bits: radius.top_left.to_bits(),
        top_right_bits: radius.top_right.to_bits(),
        bottom_right_bits: radius.bottom_right.to_bits(),
        bottom_left_bits: radius.bottom_left.to_bits(),
        blur_bits: blur_radius.to_bits(),
    }
}

pub(crate) fn cached_shadow_mask(
    layout: LayoutBox,
    radius: CornerRadius,
    blur_radius: f32,
) -> (Arc<ShadowMask>, i32, i32) {
    let blur_radius = blur_radius.max(0.0);
    let (relative_layout, offset_x, offset_y) = split_layout_for_shadow_cache(layout);
    let key = shadow_mask_cache_key(relative_layout, radius, blur_radius);

    if let Some(mask) = shadow_mask_cache()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .masks
        .get(&key)
        .cloned()
    {
        return (mask, offset_x, offset_y);
    }

    let mask = Arc::new(rasterize_shadow_mask(relative_layout, radius, blur_radius));
    let mut cache = shadow_mask_cache()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    if cache.masks.len() >= MAX_SHADOW_MASK_CACHE_ENTRIES {
        cache.masks.clear();
    }
    cache.masks.insert(key, mask.clone());
    (mask, offset_x, offset_y)
}

pub(crate) fn draw_shadow(
    buffer: &mut [u32],
    width: usize,
    height: usize,
    layout: LayoutBox,
    radius: CornerRadius,
    shadow: cssimpler_core::BoxShadow,
    clip: ClipRect,
) {
    let base_layout = offset_layout(
        expand_layout(layout, shadow.spread),
        shadow.offset_x,
        shadow.offset_y,
    );
    let base_radius = expand_corner_radius(radius, shadow.spread);
    let blur_radius = shadow.blur_radius.max(0.0);

    if blur_radius <= 0.0 {
        draw_rounded_rect(
            buffer,
            width,
            height,
            base_layout,
            base_radius,
            shadow.color,
            clip,
        );
        return;
    }

    let (mask, offset_x, offset_y) = cached_shadow_mask(base_layout, base_radius, blur_radius);
    draw_shadow_mask(
        buffer,
        width,
        height,
        &mask,
        shadow.color,
        offset_x,
        offset_y,
        clip,
    );
}

pub(crate) fn draw_shadow_effect(
    buffer: &mut [u32],
    width: usize,
    height: usize,
    layout: LayoutBox,
    radius: CornerRadius,
    shadow: cssimpler_core::ShadowEffect,
    fallback_color: Color,
    clip: ClipRect,
) {
    draw_shadow(
        buffer,
        width,
        height,
        layout,
        radius,
        cssimpler_core::BoxShadow {
            color: shadow.color.unwrap_or(fallback_color),
            offset_x: shadow.offset_x,
            offset_y: shadow.offset_y,
            blur_radius: shadow.blur_radius,
            spread: shadow.spread,
        },
        clip,
    );
}

pub(crate) fn shadow_bounds(layout: LayoutBox, shadow: cssimpler_core::BoxShadow) -> Option<ClipRect> {
    let shadow_layout = offset_layout(
        expand_layout(layout, shadow.spread),
        shadow.offset_x,
        shadow.offset_y,
    );
    non_empty_layout_clip(expand_layout(shadow_layout, shadow.blur_radius.max(0.0)))
}

pub(crate) fn shadow_effect_bounds(
    layout: LayoutBox,
    shadow: cssimpler_core::ShadowEffect,
) -> Option<ClipRect> {
    shadow_bounds(
        layout,
        cssimpler_core::BoxShadow {
            color: shadow.color.unwrap_or(Color::BLACK),
            offset_x: shadow.offset_x,
            offset_y: shadow.offset_y,
            blur_radius: shadow.blur_radius,
            spread: shadow.spread,
        },
    )
}

pub(crate) fn text_stroke_bounds(layout: LayoutBox, stroke: TextStrokeStyle) -> Option<ClipRect> {
    if stroke.width <= 0.0 {
        return None;
    }

    non_empty_layout_clip(expand_layout(layout, stroke.width.ceil().max(0.0)))
}

fn rasterize_shadow_mask(layout: LayoutBox, radius: CornerRadius, blur_radius: f32) -> ShadowMask {
    let blurred_bounds = expand_layout(layout, blur_radius);
    let x0 = blurred_bounds.x.floor() as i32;
    let y0 = blurred_bounds.y.floor() as i32;
    let x1 = (blurred_bounds.x + blurred_bounds.width).ceil() as i32;
    let y1 = (blurred_bounds.y + blurred_bounds.height).ceil() as i32;
    let mut mask = ShadowMask::new(x0, y0, x1 - x0, y1 - y0);

    if mask.width == 0 || mask.height == 0 {
        return mask;
    }

    for y in y0..y1 {
        let local_row_start = (y - y0) as usize * mask.width;
        for x in x0..x1 {
            let alpha = shadow_alpha(
                x as f32 + 0.5,
                y as f32 + 0.5,
                layout,
                radius,
                blur_radius,
                u8::MAX,
            );
            if alpha == 0 {
                continue;
            }

            let index = local_row_start + (x - x0) as usize;
            mask.alpha[index] = alpha;
        }
    }

    mask
}

fn draw_shadow_mask(
    buffer: &mut [u32],
    width: usize,
    height: usize,
    mask: &ShadowMask,
    color: Color,
    offset_x: i32,
    offset_y: i32,
    clip: ClipRect,
) {
    if color.a == 0 || mask.width == 0 || mask.height == 0 {
        return;
    }

    let Some((clip_x0, clip_y0, clip_x1, clip_y1)) = clip_pixel_bounds(clip, width, height) else {
        return;
    };
    let mask_x0 = mask.origin_x + offset_x;
    let mask_y0 = mask.origin_y + offset_y;
    let mask_x1 = mask_x0 + mask.width as i32;
    let mask_y1 = mask_y0 + mask.height as i32;
    let draw_x0 = mask_x0.max(clip_x0);
    let draw_y0 = mask_y0.max(clip_y0);
    let draw_x1 = mask_x1.min(clip_x1);
    let draw_y1 = mask_y1.min(clip_y1);

    if draw_x0 >= draw_x1 || draw_y0 >= draw_y1 {
        return;
    }

    let prepared_color = PreparedBlendColor::new(color);
    for y in draw_y0..draw_y1 {
        let local_y = (y - mask_y0) as usize;
        let row_start = local_y * mask.width;
        for x in draw_x0..draw_x1 {
            let coverage = mask.alpha[row_start + (x - mask_x0) as usize];
            if coverage == 0 {
                continue;
            }

            let alpha = scale_alpha(coverage, color.a);
            if alpha == 0 {
                continue;
            }

            blend_prepared_pixel(
                buffer,
                width,
                height,
                x,
                y,
                prepared_color.with_alpha(alpha),
            );
        }
    }
}

fn shadow_alpha(
    x: f32,
    y: f32,
    layout: LayoutBox,
    radius: CornerRadius,
    blur_radius: f32,
    max_alpha: u8,
) -> u8 {
    if point_in_rounded_rect(x, y, layout, radius) {
        return max_alpha;
    }

    let distance = distance_to_rounded_rect(x, y, layout, radius);
    if distance >= blur_radius {
        return 0;
    }

    let falloff = 1.0 - (distance / blur_radius);
    ((max_alpha as f32) * falloff * falloff).round() as u8
}

fn distance_to_rounded_rect(x: f32, y: f32, layout: LayoutBox, radius: CornerRadius) -> f32 {
    let radius = clamp_corner_radius(radius, layout.width, layout.height);
    let left = layout.x;
    let top = layout.y;
    let right = layout.x + layout.width;
    let bottom = layout.y + layout.height;

    if x < left + radius.top_left && y < top + radius.top_left {
        return distance_to_corner(
            x,
            y,
            left + radius.top_left,
            top + radius.top_left,
            radius.top_left,
        );
    }

    if x > right - radius.top_right && y < top + radius.top_right {
        return distance_to_corner(
            x,
            y,
            right - radius.top_right,
            top + radius.top_right,
            radius.top_right,
        );
    }

    if x > right - radius.bottom_right && y > bottom - radius.bottom_right {
        return distance_to_corner(
            x,
            y,
            right - radius.bottom_right,
            bottom - radius.bottom_right,
            radius.bottom_right,
        );
    }

    if x < left + radius.bottom_left && y > bottom - radius.bottom_left {
        return distance_to_corner(
            x,
            y,
            left + radius.bottom_left,
            bottom - radius.bottom_left,
            radius.bottom_left,
        );
    }

    let dx = if x < left {
        left - x
    } else if x > right {
        x - right
    } else {
        0.0
    };
    let dy = if y < top {
        top - y
    } else if y > bottom {
        y - bottom
    } else {
        0.0
    };

    if dx > 0.0 || dy > 0.0 {
        (dx * dx + dy * dy).sqrt()
    } else {
        0.0
    }
}

fn distance_to_corner(x: f32, y: f32, center_x: f32, center_y: f32, radius: f32) -> f32 {
    if radius <= 0.0 {
        let dx = x - center_x;
        let dy = y - center_y;
        return (dx * dx + dy * dy).sqrt();
    }

    let dx = x - center_x;
    let dy = y - center_y;
    ((dx * dx + dy * dy).sqrt() - radius).max(0.0)
}

fn clamp_corner_radius(radius: CornerRadius, width: f32, height: f32) -> CornerRadius {
    let max_radius = 0.5 * width.min(height).max(0.0);
    CornerRadius {
        top_left: radius.top_left.min(max_radius).max(0.0),
        top_right: radius.top_right.min(max_radius).max(0.0),
        bottom_right: radius.bottom_right.min(max_radius).max(0.0),
        bottom_left: radius.bottom_left.min(max_radius).max(0.0),
    }
}
