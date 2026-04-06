use std::collections::HashMap;
use std::hash::Hash;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;

use ab_glyph::{Font, ScaleFont, point};
use cssimpler_core::fonts::{
    FontFamily, FontStyle, GenericFontFamily, LineHeight, ResolvedFont, TextLayout, TextStyle,
    TextTransform, layout_text_block, resolve_font,
};
use cssimpler_core::{Color, LayoutBox, ShadowEffect, TextStrokeStyle, VisualStyle};
use font8x8::{BASIC_FONTS, UnicodeFonts};

use crate::{
    ClipRect, PreparedBlendColor, blend_mask_row, clip_pixel_bounds, current_render_buffer_rows,
};

const BITMAP_LINE_HEIGHT_PX: f32 = 20.0;
const MAX_TEXT_RASTER_CACHE_ENTRIES: usize = 256;
const MAX_TEXT_EFFECT_CACHE_ENTRIES: usize = 512;
const MAX_TEXT_EFFECT_WORKERS: usize = 8;
const MIN_PARALLEL_BLUR_PIXELS: usize = 24_576;
const TEXT_BLUR_PASS_COUNT: usize = 3;

pub(crate) fn draw_text(
    buffer: &mut [u32],
    width: usize,
    height: usize,
    layout: LayoutBox,
    text: &str,
    style: &VisualStyle,
    clip: ClipRect,
) {
    let Some(raster) = cached_text_mask(layout, text, &style.text) else {
        return;
    };

    for shadow in style
        .filter_drop_shadows
        .iter()
        .chain(style.text_shadows.iter())
    {
        draw_shadow_mask(
            buffer,
            width,
            height,
            &raster,
            *shadow,
            style.foreground,
            clip,
        );
    }

    if style.text_stroke.width > 0.0 {
        draw_stroke_mask(
            buffer,
            width,
            height,
            &raster,
            style.text_stroke,
            style.foreground,
            clip,
        );
    }

    draw_mask(
        buffer,
        width,
        height,
        &raster.mask,
        style.foreground,
        raster.offset_x,
        raster.offset_y,
        clip,
    );
}

fn draw_shadow_mask(
    buffer: &mut [u32],
    width: usize,
    height: usize,
    raster: &CachedTextMask,
    shadow: ShadowEffect,
    fallback_color: Color,
    clip: ClipRect,
) {
    let mask = if shadow.spread > 0.0 || shadow.blur_radius > 0.0 {
        cached_text_effect_mask(
            raster,
            TextEffectCacheKind::Shadow {
                spread_bits: shadow.spread.to_bits(),
                blur_bits: shadow.blur_radius.to_bits(),
            },
            |base| shadow_mask_from_raster(base, shadow),
        )
    } else {
        raster.mask.clone()
    };

    draw_mask(
        buffer,
        width,
        height,
        &mask,
        shadow.color.unwrap_or(fallback_color),
        raster.offset_x + shadow.offset_x.round() as i32,
        raster.offset_y + shadow.offset_y.round() as i32,
        clip,
    );
}

fn draw_stroke_mask(
    buffer: &mut [u32],
    width: usize,
    height: usize,
    raster: &CachedTextMask,
    stroke: TextStrokeStyle,
    fallback_color: Color,
    clip: ClipRect,
) {
    let outline = cached_text_effect_mask(
        raster,
        TextEffectCacheKind::Stroke {
            width_bits: stroke.width.to_bits(),
        },
        |base| stroke_mask_from_raster(base, stroke.width),
    );
    draw_mask(
        buffer,
        width,
        height,
        &outline,
        stroke.color.unwrap_or(fallback_color),
        raster.offset_x,
        raster.offset_y,
        clip,
    );
}

fn draw_mask(
    buffer: &mut [u32],
    width: usize,
    height: usize,
    mask: &AlphaMask,
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
    let rows = current_render_buffer_rows();
    let row_start = rows.start.min(height) as i32;
    let row_end = rows.end.min(height) as i32;
    let mask_x0 = mask.origin_x + offset_x;
    let mask_y0 = mask.origin_y + offset_y;
    let mask_x1 = mask_x0 + mask.width as i32;
    let mask_y1 = mask_y0 + mask.height as i32;
    let draw_x0 = mask_x0.max(clip_x0);
    let draw_y0 = mask_y0.max(clip_y0).max(row_start);
    let draw_x1 = mask_x1.min(clip_x1);
    let draw_y1 = mask_y1.min(clip_y1).min(row_end);
    if draw_x0 >= draw_x1 || draw_y0 >= draw_y1 {
        return;
    }

    let prepared_color = PreparedBlendColor::new(color);
    let local_x0 = (draw_x0 - mask_x0) as usize;
    let local_x1 = (draw_x1 - mask_x0) as usize;
    for y in draw_y0..draw_y1 {
        let local_y = (y - mask_y0) as usize;
        let mask_row_start = local_y * mask.width + local_x0;
        let mask_row_end = local_y * mask.width + local_x1;
        let buffer_row_start = (y as usize - rows.start) * width + draw_x0 as usize;
        let buffer_row_end = buffer_row_start + (local_x1 - local_x0);
        blend_mask_row(
            &mut buffer[buffer_row_start..buffer_row_end],
            &mask.alpha[mask_row_start..mask_row_end],
            prepared_color,
            color.a,
        );
    }
}

fn alpha_mask_with_alpha(mask: &AlphaMask, alpha: Vec<u8>) -> AlphaMask {
    AlphaMask {
        origin_x: mask.origin_x,
        origin_y: mask.origin_y,
        width: mask.width,
        height: mask.height,
        alpha,
    }
}

fn rasterize_resolved_text(
    layout: LayoutBox,
    wrapped: &TextLayout,
    font: &ResolvedFont,
    letter_spacing_px: f32,
) -> Option<AlphaMask> {
    let scaled_font = font.font().as_scaled(font.size_px());
    let glyphs = positioned_glyphs(layout, wrapped, font, letter_spacing_px);
    let mut bounds: Option<(i32, i32, i32, i32)> = None;

    for glyph in &glyphs {
        let Some(outlined) = font.font().outline_glyph(glyph.clone()) else {
            continue;
        };
        bounds = union_pixel_bounds(bounds, outlined.px_bounds());
    }

    let (min_x, min_y, max_x, max_y) = bounds?;
    let mut mask = AlphaMask::new(min_x, min_y, max_x - min_x, max_y - min_y);

    for glyph in glyphs {
        let Some(outlined) = font.font().outline_glyph(glyph) else {
            continue;
        };
        let bounds = outlined.px_bounds();
        let origin_x = bounds.min.x.floor() as i32;
        let origin_y = bounds.min.y.floor() as i32;

        outlined.draw(|x, y, coverage| {
            let local_x = origin_x - mask.origin_x + x as i32;
            let local_y = origin_y - mask.origin_y + y as i32;
            let alpha = (coverage.clamp(0.0, 1.0) * 255.0).round() as u8;
            mask.set_max(local_x, local_y, alpha);
        });
    }

    let _ = scaled_font;
    Some(mask)
}

fn cached_text_mask(
    layout: LayoutBox,
    text: &str,
    text_style: &TextStyle,
) -> Option<CachedTextMask> {
    let (relative_layout, offset_x, offset_y) = split_layout_for_cache(layout);
    let key = Arc::new(TextRasterCacheKey::new(text, text_style, relative_layout));

    if let Some(mask) = cached_raster_mask(key.as_ref()) {
        return Some(CachedTextMask {
            mask,
            key,
            offset_x,
            offset_y,
        });
    }

    let wrapped = layout_text_block(text, text_style, Some(relative_layout.width.max(1.0)));
    let mask = if let Some(font) = resolve_font(text_style) {
        rasterize_resolved_text(
            relative_layout,
            &wrapped,
            &font,
            text_style.letter_spacing_px,
        )
    } else {
        rasterize_bitmap_text(relative_layout, &wrapped, text_style)
    }?;
    let mask = Arc::new(mask);

    insert_cached_raster_mask(key.as_ref().clone(), mask.clone());

    Some(CachedTextMask {
        mask,
        key,
        offset_x,
        offset_y,
    })
}

fn cached_raster_mask(key: &TextRasterCacheKey) -> Option<Arc<AlphaMask>> {
    let mut cache = text_mask_cache()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let last_used = next_cache_use(&mut cache.next_use);
    cached_cache_entry(&mut cache.rasters, key, last_used)
}

fn insert_cached_raster_mask(key: TextRasterCacheKey, mask: Arc<AlphaMask>) {
    let mut cache = text_mask_cache()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let last_used = next_cache_use(&mut cache.next_use);
    insert_lru_cache_entry(
        &mut cache.rasters,
        key,
        mask,
        last_used,
        MAX_TEXT_RASTER_CACHE_ENTRIES,
    );
}

fn cached_text_effect_mask(
    raster: &CachedTextMask,
    kind: TextEffectCacheKind,
    build: impl FnOnce(&AlphaMask) -> AlphaMask,
) -> Arc<AlphaMask> {
    let key = TextEffectCacheKey {
        raster: raster.key.clone(),
        kind,
    };

    {
        let mut cache = text_mask_cache()
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let last_used = next_cache_use(&mut cache.next_use);
        if let Some(mask) = cached_cache_entry(&mut cache.effects, &key, last_used) {
            return mask;
        }
    }

    let mask = Arc::new(build(raster.mask.as_ref()));
    let mut cache = text_mask_cache()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let last_used = next_cache_use(&mut cache.next_use);
    if let Some(existing) = cached_cache_entry(&mut cache.effects, &key, last_used) {
        return existing;
    }
    insert_lru_cache_entry(
        &mut cache.effects,
        key,
        mask.clone(),
        last_used,
        MAX_TEXT_EFFECT_CACHE_ENTRIES,
    );
    mask
}

fn shadow_mask_from_raster(raster: &AlphaMask, shadow: ShadowEffect) -> AlphaMask {
    let mut mask = raster.clone();
    if shadow.spread > 0.0 {
        mask = dilate_mask(&mask, shadow.spread);
    }
    if shadow.blur_radius > 0.0 {
        mask = blur_mask(&pad_mask(&mask, shadow.blur_radius), shadow.blur_radius);
    }
    mask
}

fn stroke_mask_from_raster(raster: &AlphaMask, width: f32) -> AlphaMask {
    let radius = width.ceil().max(0.0);
    if radius <= 0.0 {
        return raster.clone();
    }

    let padded = pad_mask(raster, radius);
    let outer = dilate_mask(&padded, radius);
    let inner = erode_mask(&padded, radius);
    subtract_mask(&outer, &inner)
}

fn split_layout_for_cache(layout: LayoutBox) -> (LayoutBox, i32, i32) {
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

fn positioned_glyphs(
    layout: LayoutBox,
    wrapped: &TextLayout,
    font: &ResolvedFont,
    letter_spacing_px: f32,
) -> Vec<ab_glyph::Glyph> {
    let scaled_font = font.font().as_scaled(font.size_px());
    let start_x = layout.x;
    let start_y = layout.y;
    let mut glyphs = Vec::new();

    for (line_index, line) in wrapped.lines.iter().enumerate() {
        let mut caret_x = start_x;
        let baseline_y = start_y + scaled_font.ascent() + line_index as f32 * font.line_height_px();
        let mut previous = None;
        let mut characters = line.text.chars().peekable();

        while let Some(character) = characters.next() {
            let glyph_id = scaled_font.glyph_id(character);
            if let Some(previous) = previous {
                caret_x += scaled_font.kern(previous, glyph_id);
            }

            glyphs
                .push(glyph_id.with_scale_and_position(font.size_px(), point(caret_x, baseline_y)));
            caret_x += scaled_font.h_advance(glyph_id);
            if characters.peek().is_some() {
                caret_x += letter_spacing_px;
            }
            previous = Some(glyph_id);
        }
    }

    glyphs
}

fn rasterize_bitmap_text(
    layout: LayoutBox,
    wrapped: &TextLayout,
    text_style: &TextStyle,
) -> Option<AlphaMask> {
    let scale = ((text_style.size_px.max(1.0) / 8.0).round() as i32).max(1);
    let start_x = layout.x.round() as i32;
    let start_y = layout.y.round() as i32;
    let line_step = wrapped
        .line_height
        .max(BITMAP_LINE_HEIGHT_PX * (scale as f32 / 2.0));
    let mut bounds: Option<(i32, i32, i32, i32)> = None;

    for (line_index, line) in wrapped.lines.iter().enumerate() {
        let mut cursor_x = start_x as f32;
        let cursor_y = start_y + (line_index as f32 * line_step).round() as i32;
        let mut characters = line.text.chars().peekable();

        while let Some(character) = characters.next() {
            if BASIC_FONTS.get(character).is_some() {
                bounds = Some(match bounds {
                    Some((min_x, min_y, max_x, max_y)) => (
                        min_x.min(cursor_x.round() as i32),
                        min_y.min(cursor_y),
                        max_x.max(cursor_x.round() as i32 + (8 * scale)),
                        max_y.max(cursor_y + (8 * scale)),
                    ),
                    None => (
                        cursor_x.round() as i32,
                        cursor_y,
                        cursor_x.round() as i32 + (8 * scale),
                        cursor_y + (8 * scale),
                    ),
                });
            }

            cursor_x += 9.0 * scale as f32;
            if characters.peek().is_some() {
                cursor_x += text_style.letter_spacing_px;
            }
        }
    }

    let (min_x, min_y, max_x, max_y) = bounds?;
    let mut mask = AlphaMask::new(min_x, min_y, max_x - min_x, max_y - min_y);

    for (line_index, line) in wrapped.lines.iter().enumerate() {
        let mut cursor_x = start_x as f32;
        let cursor_y = start_y + (line_index as f32 * line_step).round() as i32;
        let mut characters = line.text.chars().peekable();

        while let Some(character) = characters.next() {
            let glyph_origin_x = cursor_x.round() as i32;
            if let Some(glyph) = BASIC_FONTS.get(character) {
                for (row_index, row) in glyph.iter().enumerate() {
                    for column in 0..8 {
                        if ((*row >> column) & 1) == 0 {
                            continue;
                        }

                        for y_step in 0..scale {
                            for x_step in 0..scale {
                                let local_x =
                                    glyph_origin_x - mask.origin_x + (column * scale) + x_step;
                                let local_y =
                                    cursor_y - mask.origin_y + (row_index as i32 * scale) + y_step;
                                mask.set_max(local_x, local_y, 255);
                            }
                        }
                    }
                }
            }

            cursor_x += 9.0 * scale as f32;
            if characters.peek().is_some() {
                cursor_x += text_style.letter_spacing_px;
            }
        }
    }

    Some(mask)
}

fn pad_mask(mask: &AlphaMask, radius: f32) -> AlphaMask {
    let pad = radius.ceil().max(0.0) as i32;
    if pad == 0 {
        return mask.clone();
    }

    let mut padded = AlphaMask::new(
        mask.origin_x - pad,
        mask.origin_y - pad,
        mask.width as i32 + pad * 2,
        mask.height as i32 + pad * 2,
    );

    for y in 0..mask.height {
        for x in 0..mask.width {
            padded.alpha[(x + pad as usize) + (y + pad as usize) * padded.width] =
                mask.alpha[x + y * mask.width];
        }
    }

    padded
}

fn dilate_mask(mask: &AlphaMask, radius: f32) -> AlphaMask {
    let radius = radius.max(0.0);
    if radius <= 0.0 {
        return mask.clone();
    }

    let pad = radius.ceil() as i32;
    let mut expanded = AlphaMask::new(
        mask.origin_x - pad,
        mask.origin_y - pad,
        mask.width as i32 + pad * 2,
        mask.height as i32 + pad * 2,
    );
    let radius_squared = radius * radius;

    for y in 0..expanded.height {
        for x in 0..expanded.width {
            let mut alpha = 0_u8;
            for dy in -pad..=pad {
                for dx in -pad..=pad {
                    let distance_squared = (dx * dx + dy * dy) as f32;
                    if distance_squared > radius_squared {
                        continue;
                    }

                    let source_x = x as i32 - pad + dx;
                    let source_y = y as i32 - pad + dy;
                    alpha = alpha.max(mask.get(source_x, source_y));
                    if alpha == 255 {
                        break;
                    }
                }
                if alpha == 255 {
                    break;
                }
            }
            expanded.alpha[x + y * expanded.width] = alpha;
        }
    }

    expanded
}

fn erode_mask(mask: &AlphaMask, radius: f32) -> AlphaMask {
    let radius = radius.max(0.0);
    if radius <= 0.0 {
        return mask.clone();
    }

    let pad = radius.ceil() as i32;
    let radius_squared = radius * radius;
    let mut contracted = mask.clone();

    for y in 0..mask.height {
        for x in 0..mask.width {
            let mut alpha = 255_u8;
            for dy in -pad..=pad {
                for dx in -pad..=pad {
                    let distance_squared = (dx * dx + dy * dy) as f32;
                    if distance_squared > radius_squared {
                        continue;
                    }

                    alpha = alpha.min(mask.get(x as i32 + dx, y as i32 + dy));
                    if alpha == 0 {
                        break;
                    }
                }
                if alpha == 0 {
                    break;
                }
            }
            contracted.alpha[x + y * mask.width] = alpha;
        }
    }

    contracted
}

fn blur_mask(mask: &AlphaMask, radius: f32) -> AlphaMask {
    let radius = radius.max(0.0);
    if radius <= 0.0 {
        return mask.clone();
    }

    let kernel_radius = radius.ceil() as usize;
    if kernel_radius == 0 || mask.width == 0 || mask.height == 0 {
        return mask.clone();
    }

    let worker_count = blur_worker_count(mask);
    blur_mask_with_workers(mask, kernel_radius, worker_count)
}

fn blur_mask_with_workers(mask: &AlphaMask, radius: usize, worker_count: usize) -> AlphaMask {
    // A single wide box blur looks chunky and rectangular. Split the requested
    // radius across a few smaller separable passes so the glow falloff reads more
    // like a soft Gaussian while staying linear-time and cache-friendly.
    let mut blurred = mask.clone();
    for pass_radius in blur_pass_radii(radius) {
        blurred = blur_mask_horizontally(&blurred, pass_radius, worker_count);
        blurred = blur_mask_vertically(&blurred, pass_radius, worker_count);
    }
    blurred
}

fn blur_worker_count(mask: &AlphaMask) -> usize {
    let total_pixels = mask.width.saturating_mul(mask.height);
    if total_pixels < MIN_PARALLEL_BLUR_PIXELS {
        return 1;
    }

    thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
        .min(MAX_TEXT_EFFECT_WORKERS)
        .max(1)
}

fn blur_pass_radii(radius: usize) -> Vec<usize> {
    if radius == 0 {
        return Vec::new();
    }

    let pass_count = TEXT_BLUR_PASS_COUNT.min(radius.max(1));
    let base_radius = radius / pass_count;
    let remainder = radius % pass_count;
    let mut radii = Vec::with_capacity(pass_count);

    for pass_index in 0..pass_count {
        let extra = usize::from(pass_index < remainder);
        let pass_radius = base_radius + extra;
        if pass_radius > 0 {
            radii.push(pass_radius);
        }
    }

    if radii.is_empty() {
        radii.push(1);
    }

    radii
}

fn blur_mask_horizontally(mask: &AlphaMask, radius: usize, worker_count: usize) -> AlphaMask {
    let worker_count = worker_count.max(1).min(mask.height.max(1));

    if worker_count == 1 {
        return alpha_mask_with_alpha(mask, blur_rows(mask, radius, 0, mask.height));
    }

    let rows_per_worker = mask.height.div_ceil(worker_count);
    let mut alpha = vec![0_u8; mask.width * mask.height];

    thread::scope(|scope| {
        let mut handles = Vec::with_capacity(worker_count);
        for row_start in (0..mask.height).step_by(rows_per_worker) {
            let row_end = (row_start + rows_per_worker).min(mask.height);
            handles.push(
                scope.spawn(move || (row_start, blur_rows(mask, radius, row_start, row_end))),
            );
        }

        for handle in handles {
            let (row_start, chunk) = handle
                .join()
                .expect("horizontal text blur worker should not panic");
            let start = row_start * mask.width;
            let end = start + chunk.len();
            alpha[start..end].copy_from_slice(&chunk);
        }
    });

    alpha_mask_with_alpha(mask, alpha)
}

fn blur_rows(mask: &AlphaMask, radius: usize, row_start: usize, row_end: usize) -> Vec<u8> {
    let mut alpha = vec![0_u8; (row_end - row_start) * mask.width];
    let mut prefix = vec![0_u32; mask.width + 1];

    for y in row_start..row_end {
        let source_row_start = y * mask.width;
        let target_row_start = (y - row_start) * mask.width;
        prefix[0] = 0;
        for x in 0..mask.width {
            prefix[x + 1] = prefix[x] + mask.alpha[source_row_start + x] as u32;
        }

        for x in 0..mask.width {
            let left = x.saturating_sub(radius);
            let right = (x + radius + 1).min(mask.width);
            let sum = prefix[right] - prefix[left];
            let count = (right - left) as u32;
            alpha[target_row_start + x] = (sum / count) as u8;
        }
    }

    alpha
}

fn blur_mask_vertically(mask: &AlphaMask, radius: usize, worker_count: usize) -> AlphaMask {
    let worker_count = worker_count.max(1).min(mask.width.max(1));

    if worker_count == 1 {
        return alpha_mask_with_alpha(mask, blur_columns(mask, radius, 0, mask.width));
    }

    let columns_per_worker = mask.width.div_ceil(worker_count);
    let mut alpha = vec![0_u8; mask.width * mask.height];

    thread::scope(|scope| {
        let mut handles = Vec::with_capacity(worker_count);
        for column_start in (0..mask.width).step_by(columns_per_worker) {
            let column_end = (column_start + columns_per_worker).min(mask.width);
            handles.push(scope.spawn(move || {
                (
                    column_start,
                    column_end,
                    blur_columns(mask, radius, column_start, column_end),
                )
            }));
        }

        for handle in handles {
            let (column_start, column_end, chunk) = handle
                .join()
                .expect("vertical text blur worker should not panic");
            let chunk_width = column_end - column_start;
            for y in 0..mask.height {
                let source_row_start = y * chunk_width;
                let destination_row_start = y * mask.width + column_start;
                let destination_row_end = destination_row_start + chunk_width;
                alpha[destination_row_start..destination_row_end]
                    .copy_from_slice(&chunk[source_row_start..source_row_start + chunk_width]);
            }
        }
    });

    alpha_mask_with_alpha(mask, alpha)
}

fn blur_columns(
    mask: &AlphaMask,
    radius: usize,
    column_start: usize,
    column_end: usize,
) -> Vec<u8> {
    let chunk_width = column_end - column_start;
    let mut alpha = vec![0_u8; chunk_width * mask.height];
    let mut prefix = vec![0_u32; mask.height + 1];

    for x in column_start..column_end {
        let local_x = x - column_start;
        prefix[0] = 0;
        for y in 0..mask.height {
            prefix[y + 1] = prefix[y] + mask.alpha[x + y * mask.width] as u32;
        }

        for y in 0..mask.height {
            let top = y.saturating_sub(radius);
            let bottom = (y + radius + 1).min(mask.height);
            let sum = prefix[bottom] - prefix[top];
            let count = (bottom - top) as u32;
            alpha[local_x + y * chunk_width] = (sum / count) as u8;
        }
    }

    alpha
}

fn subtract_mask(mask: &AlphaMask, subtract: &AlphaMask) -> AlphaMask {
    let mut result = mask.clone();

    for y in 0..result.height {
        for x in 0..result.width {
            let world_x = result.origin_x + x as i32;
            let world_y = result.origin_y + y as i32;
            let source = result.alpha[x + y * result.width];
            let removed = subtract.get(world_x - subtract.origin_x, world_y - subtract.origin_y);
            result.alpha[x + y * result.width] = source.saturating_sub(removed);
        }
    }

    result
}

fn union_pixel_bounds(
    current: Option<(i32, i32, i32, i32)>,
    bounds: ab_glyph::Rect,
) -> Option<(i32, i32, i32, i32)> {
    let min_x = bounds.min.x.floor() as i32;
    let min_y = bounds.min.y.floor() as i32;
    let max_x = bounds.max.x.ceil() as i32;
    let max_y = bounds.max.y.ceil() as i32;
    if min_x >= max_x || min_y >= max_y {
        return current;
    }

    Some(match current {
        Some((current_min_x, current_min_y, current_max_x, current_max_y)) => (
            current_min_x.min(min_x),
            current_min_y.min(min_y),
            current_max_x.max(max_x),
            current_max_y.max(max_y),
        ),
        None => (min_x, min_y, max_x, max_y),
    })
}

#[derive(Clone)]
struct AlphaMask {
    origin_x: i32,
    origin_y: i32,
    width: usize,
    height: usize,
    alpha: Vec<u8>,
}

impl AlphaMask {
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

    fn get(&self, x: i32, y: i32) -> u8 {
        if x < 0 || y < 0 || x as usize >= self.width || y as usize >= self.height {
            return 0;
        }

        self.alpha[x as usize + y as usize * self.width]
    }

    fn set_max(&mut self, x: i32, y: i32, alpha: u8) {
        if x < 0 || y < 0 || x as usize >= self.width || y as usize >= self.height {
            return;
        }

        let index = x as usize + y as usize * self.width;
        self.alpha[index] = self.alpha[index].max(alpha);
    }
}

#[derive(Clone)]
struct CachedTextMask {
    mask: Arc<AlphaMask>,
    key: Arc<TextRasterCacheKey>,
    offset_x: i32,
    offset_y: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct TextRasterCacheKey {
    text: Arc<str>,
    style: TextStyleCacheKey,
    width_bits: u32,
    origin_x_bits: u32,
    origin_y_bits: u32,
}

impl TextRasterCacheKey {
    fn new(text: &str, style: &TextStyle, layout: LayoutBox) -> Self {
        Self {
            text: Arc::<str>::from(text),
            style: TextStyleCacheKey::new(style),
            width_bits: layout.width.to_bits(),
            origin_x_bits: layout.x.to_bits(),
            origin_y_bits: layout.y.to_bits(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct TextStyleCacheKey {
    families: Vec<FontFamilyCacheKey>,
    size_bits: u32,
    weight: u16,
    style: u8,
    line_height: LineHeightCacheKey,
    letter_spacing_bits: u32,
    text_transform: u8,
}

impl TextStyleCacheKey {
    fn new(style: &TextStyle) -> Self {
        Self {
            families: style
                .families
                .iter()
                .map(font_family_cache_key)
                .collect::<Vec<_>>(),
            size_bits: style.size_px.to_bits(),
            weight: style.weight,
            style: font_style_cache_key(style.style),
            line_height: line_height_cache_key(&style.line_height),
            letter_spacing_bits: style.letter_spacing_px.to_bits(),
            text_transform: text_transform_cache_key(style.text_transform),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum FontFamilyCacheKey {
    Named(String),
    Generic(u8),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum LineHeightCacheKey {
    Normal,
    Px(u32),
    Scale(u32),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum TextEffectCacheKind {
    Stroke { width_bits: u32 },
    Shadow { spread_bits: u32, blur_bits: u32 },
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct TextEffectCacheKey {
    raster: Arc<TextRasterCacheKey>,
    kind: TextEffectCacheKind,
}

#[derive(Clone)]
struct CacheEntry<T> {
    value: T,
    last_used: u64,
}

#[derive(Default)]
struct TextMaskCache {
    next_use: u64,
    rasters: HashMap<TextRasterCacheKey, CacheEntry<Arc<AlphaMask>>>,
    effects: HashMap<TextEffectCacheKey, CacheEntry<Arc<AlphaMask>>>,
}

fn font_family_cache_key(family: &FontFamily) -> FontFamilyCacheKey {
    match family {
        FontFamily::Named(name) => FontFamilyCacheKey::Named(name.clone()),
        FontFamily::Generic(generic) => {
            FontFamilyCacheKey::Generic(generic_font_family_cache_key(generic.clone()))
        }
    }
}

fn generic_font_family_cache_key(family: GenericFontFamily) -> u8 {
    match family {
        GenericFontFamily::Serif => 0,
        GenericFontFamily::SansSerif => 1,
        GenericFontFamily::Cursive => 2,
        GenericFontFamily::Fantasy => 3,
        GenericFontFamily::Monospace => 4,
        GenericFontFamily::SystemUi => 5,
        GenericFontFamily::Emoji => 6,
        GenericFontFamily::Math => 7,
        GenericFontFamily::FangSong => 8,
        GenericFontFamily::UiSerif => 9,
        GenericFontFamily::UiSansSerif => 10,
        GenericFontFamily::UiMonospace => 11,
        GenericFontFamily::UiRounded => 12,
    }
}

fn font_style_cache_key(style: FontStyle) -> u8 {
    match style {
        FontStyle::Normal => 0,
        FontStyle::Italic => 1,
        FontStyle::Oblique => 2,
    }
}

fn line_height_cache_key(line_height: &LineHeight) -> LineHeightCacheKey {
    match line_height {
        LineHeight::Normal => LineHeightCacheKey::Normal,
        LineHeight::Px(value) => LineHeightCacheKey::Px(value.to_bits()),
        LineHeight::Scale(value) => LineHeightCacheKey::Scale(value.to_bits()),
    }
}

fn text_transform_cache_key(text_transform: TextTransform) -> u8 {
    match text_transform {
        TextTransform::None => 0,
        TextTransform::Uppercase => 1,
        TextTransform::Lowercase => 2,
        TextTransform::Capitalize => 3,
    }
}

fn next_cache_use(next_use: &mut u64) -> u64 {
    let last_used = *next_use;
    *next_use = next_use.saturating_add(1);
    last_used
}

fn cached_cache_entry<K, V>(
    entries: &mut HashMap<K, CacheEntry<Arc<V>>>,
    key: &K,
    last_used: u64,
) -> Option<Arc<V>>
where
    K: Eq + Hash,
{
    let entry = entries.get_mut(key)?;
    entry.last_used = last_used;
    Some(entry.value.clone())
}

fn insert_lru_cache_entry<K, V>(
    entries: &mut HashMap<K, CacheEntry<Arc<V>>>,
    key: K,
    value: Arc<V>,
    last_used: u64,
    max_entries: usize,
) where
    K: Clone + Eq + Hash,
{
    if let Some(entry) = entries.get_mut(&key) {
        entry.value = value;
        entry.last_used = last_used;
        return;
    }
    if max_entries == 0 {
        return;
    }
    while entries.len() >= max_entries {
        evict_lru_cache_entry(entries);
    }
    entries.insert(key, CacheEntry { value, last_used });
}

fn evict_lru_cache_entry<K, V>(entries: &mut HashMap<K, CacheEntry<Arc<V>>>)
where
    K: Clone + Eq + Hash,
{
    let lru_key = entries
        .iter()
        .min_by_key(|(_, entry)| entry.last_used)
        .map(|(key, _)| key.clone());
    if let Some(key) = lru_key {
        entries.remove(&key);
    }
}

fn text_mask_cache() -> &'static Mutex<TextMaskCache> {
    static CACHE: OnceLock<Mutex<TextMaskCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(TextMaskCache::default()))
}

#[cfg(test)]
fn clear_text_mask_cache_for_tests() {
    let mut cache = text_mask_cache()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    cache.next_use = 0;
    cache.rasters.clear();
    cache.effects.clear();
}

#[cfg(test)]
pub(crate) fn lock_text_mask_cache_for_tests() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use cssimpler_core::fonts::TextStyle;
    use cssimpler_core::{LayoutBox, ShadowEffect};

    use super::{
        MAX_TEXT_EFFECT_CACHE_ENTRIES, MAX_TEXT_RASTER_CACHE_ENTRIES, TextEffectCacheKind,
        blur_mask_with_workers, blur_pass_radii, cached_text_effect_mask, cached_text_mask,
        clear_text_mask_cache_for_tests, lock_text_mask_cache_for_tests, shadow_mask_from_raster,
        text_mask_cache,
    };

    #[test]
    fn identical_text_masks_are_reused_across_integer_position_changes() {
        let _cache_guard = lock_text_mask_cache_for_tests();
        clear_text_mask_cache_for_tests();
        let style = TextStyle::default();
        let first = cached_text_mask(LayoutBox::new(10.25, 20.0, 160.0, 40.0), "Cache", &style)
            .expect("first text mask should rasterize");
        let second = cached_text_mask(LayoutBox::new(90.25, 44.0, 160.0, 40.0), "Cache", &style)
            .expect("second text mask should rasterize");

        assert!(Arc::ptr_eq(&first.mask, &second.mask));
        assert_eq!(first.offset_x, 10);
        assert_eq!(second.offset_x, 90);
    }

    #[test]
    fn identical_shadow_masks_are_reused_for_the_same_text_raster() {
        let _cache_guard = lock_text_mask_cache_for_tests();
        clear_text_mask_cache_for_tests();
        let style = TextStyle::default();
        let raster = cached_text_mask(LayoutBox::new(12.0, 16.0, 160.0, 40.0), "Glow", &style)
            .expect("text mask should rasterize");
        let shadow = ShadowEffect {
            color: None,
            offset_x: 0.0,
            offset_y: 0.0,
            blur_radius: 6.0,
            spread: 0.0,
        };
        let first = cached_text_effect_mask(
            &raster,
            TextEffectCacheKind::Shadow {
                spread_bits: shadow.spread.to_bits(),
                blur_bits: shadow.blur_radius.to_bits(),
            },
            |base| shadow_mask_from_raster(base, shadow),
        );
        let second = cached_text_effect_mask(
            &raster,
            TextEffectCacheKind::Shadow {
                spread_bits: shadow.spread.to_bits(),
                blur_bits: shadow.blur_radius.to_bits(),
            },
            |base| shadow_mask_from_raster(base, shadow),
        );

        assert!(Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn text_raster_cache_evicts_lru_entries_without_clearing_everything() {
        let _cache_guard = lock_text_mask_cache_for_tests();
        clear_text_mask_cache_for_tests();
        let style = TextStyle::default();
        let layout = LayoutBox::new(10.25, 20.0, 160.0, 40.0);
        let first =
            cached_text_mask(layout, "Cache 0", &style).expect("first text mask should rasterize");
        let retained =
            cached_text_mask(layout, "Cache 1", &style).expect("second text mask should rasterize");
        for index in 2..MAX_TEXT_RASTER_CACHE_ENTRIES {
            cached_text_mask(layout, &format!("Cache {index}"), &style)
                .expect("cache fill text mask should rasterize");
        }

        let retained_again =
            cached_text_mask(layout, "Cache 1", &style).expect("retained text mask should cache");
        let overflow_text = format!("Cache {}", MAX_TEXT_RASTER_CACHE_ENTRIES);
        cached_text_mask(layout, &overflow_text, &style)
            .expect("overflow text mask should rasterize");
        let first_after =
            cached_text_mask(layout, "Cache 0", &style).expect("evicted text mask should rebuild");

        assert!(Arc::ptr_eq(&retained.mask, &retained_again.mask));
        assert!(!Arc::ptr_eq(&first.mask, &first_after.mask));
        let cache = text_mask_cache()
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        assert_eq!(cache.rasters.len(), MAX_TEXT_RASTER_CACHE_ENTRIES);
    }

    #[test]
    fn text_effect_cache_evicts_lru_entries_without_clearing_everything() {
        let _cache_guard = lock_text_mask_cache_for_tests();
        clear_text_mask_cache_for_tests();
        let style = TextStyle::default();
        let raster = cached_text_mask(LayoutBox::new(12.0, 16.0, 160.0, 40.0), "Glow", &style)
            .expect("text mask should rasterize");
        let first = cached_text_effect_mask(
            &raster,
            TextEffectCacheKind::Stroke {
                width_bits: 0.25_f32.to_bits(),
            },
            |base| base.clone(),
        );
        let retained = cached_text_effect_mask(
            &raster,
            TextEffectCacheKind::Stroke {
                width_bits: 1.25_f32.to_bits(),
            },
            |base| base.clone(),
        );
        for index in 2..MAX_TEXT_EFFECT_CACHE_ENTRIES {
            cached_text_effect_mask(
                &raster,
                TextEffectCacheKind::Stroke {
                    width_bits: (index as f32 + 0.25).to_bits(),
                },
                |base| base.clone(),
            );
        }

        let retained_again = cached_text_effect_mask(
            &raster,
            TextEffectCacheKind::Stroke {
                width_bits: 1.25_f32.to_bits(),
            },
            |base| base.clone(),
        );
        cached_text_effect_mask(
            &raster,
            TextEffectCacheKind::Stroke {
                width_bits: (MAX_TEXT_EFFECT_CACHE_ENTRIES as f32 + 0.25).to_bits(),
            },
            |base| base.clone(),
        );
        let first_after = cached_text_effect_mask(
            &raster,
            TextEffectCacheKind::Stroke {
                width_bits: 0.25_f32.to_bits(),
            },
            |base| base.clone(),
        );

        assert!(Arc::ptr_eq(&retained, &retained_again));
        assert!(!Arc::ptr_eq(&first, &first_after));
        let cache = text_mask_cache()
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        assert_eq!(cache.effects.len(), MAX_TEXT_EFFECT_CACHE_ENTRIES);
    }

    #[test]
    fn stroke_mask_keeps_edge_pixels_inside_the_original_fill_area() {
        let mut raster = super::AlphaMask::new(0, 0, 3, 3);
        for y in 0..3 {
            for x in 0..3 {
                raster.set_max(x, y, 255);
            }
        }

        let outline = super::stroke_mask_from_raster(&raster, 1.0);

        assert_eq!(outline.get(1 - outline.origin_x, 1 - outline.origin_y), 0);

        for y in 0..3 {
            for x in 0..3 {
                if x == 1 && y == 1 {
                    continue;
                }
                let local_x = x - outline.origin_x;
                let local_y = y - outline.origin_y;
                assert!(outline.get(local_x, local_y) > 0);
            }
        }

        assert!(outline.alpha.iter().any(|alpha| *alpha > 0));
    }

    #[test]
    fn multithreaded_blur_matches_the_single_threaded_result() {
        let mut mask = super::AlphaMask::new(0, 0, 96, 64);
        for y in 0..64 {
            for x in 0..96 {
                let value = (((x * 13) + (y * 17)) % 256) as u8;
                mask.set_max(x, y, value);
            }
        }

        let single = blur_mask_with_workers(&mask, 12, 1);
        let threaded = blur_mask_with_workers(&mask, 12, 4);

        assert_eq!(single.origin_x, threaded.origin_x);
        assert_eq!(single.origin_y, threaded.origin_y);
        assert_eq!(single.width, threaded.width);
        assert_eq!(single.height, threaded.height);
        assert_eq!(single.alpha, threaded.alpha);
    }

    #[test]
    fn blur_passes_preserve_the_requested_total_radius() {
        assert_eq!(blur_pass_radii(1), vec![1]);
        assert_eq!(blur_pass_radii(2), vec![1, 1]);
        assert_eq!(blur_pass_radii(3), vec![1, 1, 1]);
        assert_eq!(blur_pass_radii(6), vec![2, 2, 2]);
        assert_eq!(blur_pass_radii(10), vec![4, 3, 3]);
        assert_eq!(blur_pass_radii(10).iter().sum::<usize>(), 10);
    }
}
