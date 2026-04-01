use ab_glyph::{Font, ScaleFont, point};
use cssimpler_core::fonts::{ResolvedFont, TextLayout, TextStyle, layout_text_block, resolve_font};
use cssimpler_core::{Color, LayoutBox, ShadowEffect, TextStrokeStyle, VisualStyle};
use font8x8::{BASIC_FONTS, UnicodeFonts};

use crate::{ClipRect, blend_pixel};

const BITMAP_LINE_HEIGHT_PX: f32 = 20.0;

pub(crate) fn draw_text(
    buffer: &mut [u32],
    width: usize,
    height: usize,
    layout: LayoutBox,
    text: &str,
    style: &VisualStyle,
    clip: ClipRect,
) {
    let wrapped = layout_text_block(text, &style.text, Some(layout.width.max(1.0)));
    let raster = if let Some(font) = resolve_font(&style.text) {
        rasterize_resolved_text(layout, &wrapped, &font, style.text.letter_spacing_px)
    } else {
        rasterize_bitmap_text(layout, &wrapped, &style.text)
    };

    let Some(raster) = raster else {
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

    draw_mask(buffer, width, height, &raster, style.foreground, 0, 0, clip);
}

fn draw_shadow_mask(
    buffer: &mut [u32],
    width: usize,
    height: usize,
    raster: &AlphaMask,
    shadow: ShadowEffect,
    fallback_color: Color,
    clip: ClipRect,
) {
    let mut mask = raster.clone();
    if shadow.spread > 0.0 {
        mask = dilate_mask(&mask, shadow.spread);
    }
    if shadow.blur_radius > 0.0 {
        mask = blur_mask(&pad_mask(&mask, shadow.blur_radius), shadow.blur_radius);
    }

    draw_mask(
        buffer,
        width,
        height,
        &mask,
        shadow.color.unwrap_or(fallback_color),
        shadow.offset_x.round() as i32,
        shadow.offset_y.round() as i32,
        clip,
    );
}

fn draw_stroke_mask(
    buffer: &mut [u32],
    width: usize,
    height: usize,
    raster: &AlphaMask,
    stroke: TextStrokeStyle,
    fallback_color: Color,
    clip: ClipRect,
) {
    let outline = dilate_mask(&pad_mask(raster, stroke.width), stroke.width);
    draw_mask(
        buffer,
        width,
        height,
        &outline,
        stroke.color.unwrap_or(fallback_color),
        0,
        0,
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
    for y in 0..mask.height {
        for x in 0..mask.width {
            let alpha = mask.alpha[x + y * mask.width];
            if alpha == 0 {
                continue;
            }

            let pixel_x = mask.origin_x + offset_x + x as i32;
            let pixel_y = mask.origin_y + offset_y + y as i32;
            if !clip.contains(pixel_x as f32 + 0.5, pixel_y as f32 + 0.5) {
                continue;
            }

            let alpha = ((alpha as f32 / 255.0) * color.a as f32)
                .round()
                .clamp(0.0, 255.0) as u8;
            if alpha == 0 {
                continue;
            }

            blend_pixel(
                buffer,
                width,
                height,
                pixel_x,
                pixel_y,
                color.with_alpha(alpha),
            );
        }
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

fn blur_mask(mask: &AlphaMask, radius: f32) -> AlphaMask {
    let radius = radius.max(0.0);
    if radius <= 0.0 {
        return mask.clone();
    }

    let blur = radius.ceil() as i32;
    let radius_squared = radius * radius;
    let mut blurred = mask.clone();

    for y in 0..mask.height {
        for x in 0..mask.width {
            let mut alpha_sum = 0_u32;
            let mut weight_sum = 0_u32;

            for dy in -blur..=blur {
                for dx in -blur..=blur {
                    let distance_squared = (dx * dx + dy * dy) as f32;
                    if distance_squared > radius_squared {
                        continue;
                    }

                    alpha_sum += mask.get(x as i32 + dx, y as i32 + dy) as u32;
                    weight_sum += 1;
                }
            }

            blurred.alpha[x + y * mask.width] = if weight_sum == 0 {
                0
            } else {
                (alpha_sum / weight_sum) as u8
            };
        }
    }

    blurred
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
