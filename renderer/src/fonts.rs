use ab_glyph::{Font, ScaleFont, point};
use cssimpler_core::fonts::{TextStyle, layout_text_block, resolve_font};
use cssimpler_core::{Color, LayoutBox};
use font8x8::{BASIC_FONTS, UnicodeFonts};

use crate::{ClipRect, blend_pixel};

const BITMAP_LINE_HEIGHT_PX: f32 = 20.0;

pub(crate) fn draw_text(
    buffer: &mut [u32],
    width: usize,
    height: usize,
    layout: LayoutBox,
    text: &str,
    text_style: &TextStyle,
    color: Color,
    clip: ClipRect,
) {
    let wrapped = layout_text_block(text, text_style, Some(layout.width.max(1.0)));

    if let Some(font) = resolve_font(text_style) {
        draw_resolved_font_text(
            buffer,
            width,
            height,
            layout,
            &wrapped,
            &font,
            text_style.letter_spacing_px,
            color,
            clip,
        );
    } else {
        draw_bitmap_fallback_text(
            buffer, width, height, layout, &wrapped, text_style, color, clip,
        );
    }
}

fn draw_resolved_font_text(
    buffer: &mut [u32],
    width: usize,
    height: usize,
    layout: LayoutBox,
    wrapped: &cssimpler_core::fonts::TextLayout,
    font: &cssimpler_core::fonts::ResolvedFont,
    letter_spacing_px: f32,
    color: Color,
    clip: ClipRect,
) {
    let scaled_font = font.font().as_scaled(font.size_px());
    let start_x = layout.x;
    let start_y = layout.y;

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

            let glyph =
                glyph_id.with_scale_and_position(font.size_px(), point(caret_x, baseline_y));
            if let Some(outlined) = font.font().outline_glyph(glyph) {
                let bounds = outlined.px_bounds();
                let origin_x = bounds.min.x.floor() as i32;
                let origin_y = bounds.min.y.floor() as i32;

                outlined.draw(|x, y, coverage| {
                    let pixel_x = origin_x + x as i32;
                    let pixel_y = origin_y + y as i32;
                    if !clip.contains(pixel_x as f32 + 0.5, pixel_y as f32 + 0.5) {
                        return;
                    }

                    let alpha = (coverage.clamp(0.0, 1.0) * color.a as f32).round() as u8;
                    if alpha == 0 {
                        return;
                    }

                    blend_pixel(
                        buffer,
                        width,
                        height,
                        pixel_x,
                        pixel_y,
                        color.with_alpha(alpha),
                    );
                });
            }

            caret_x += scaled_font.h_advance(glyph_id);
            if characters.peek().is_some() {
                caret_x += letter_spacing_px;
            }
            previous = Some(glyph_id);
        }
    }
}

fn draw_bitmap_fallback_text(
    buffer: &mut [u32],
    width: usize,
    height: usize,
    layout: LayoutBox,
    wrapped: &cssimpler_core::fonts::TextLayout,
    text_style: &TextStyle,
    color: Color,
    clip: ClipRect,
) {
    let scale = ((text_style.size_px.max(1.0) / 8.0).round() as i32).max(1);
    let start_x = layout.x;
    let start_y = layout.y;
    let line_step = wrapped
        .line_height
        .max(BITMAP_LINE_HEIGHT_PX * (scale as f32 / 2.0));

    for (line_index, line) in wrapped.lines.iter().enumerate() {
        let mut cursor_x = start_x;
        let cursor_y = start_y + line_index as f32 * line_step;
        let mut characters = line.text.chars().peekable();

        while let Some(character) = characters.next() {
            let glyph_origin_x = cursor_x.round() as i32;
            let glyph_origin_y = cursor_y.round() as i32;
            if let Some(glyph) = BASIC_FONTS.get(character) {
                for (row_index, row) in glyph.iter().enumerate() {
                    for column in 0..8 {
                        if ((*row >> column) & 1) == 0 {
                            continue;
                        }

                        for y_step in 0..scale {
                            for x_step in 0..scale {
                                let x = glyph_origin_x + (column * scale) + x_step;
                                let y = glyph_origin_y + (row_index as i32 * scale) + y_step;
                                if !clip.contains(x as f32 + 0.5, y as f32 + 0.5) {
                                    continue;
                                }
                                blend_pixel(buffer, width, height, x, y, color);
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
}
