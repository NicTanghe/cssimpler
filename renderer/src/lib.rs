use std::error::Error;
use std::fmt::{Display, Formatter};
use std::time::{Duration, Instant};

use cssimpler_core::{Color, LayoutBox, RenderKind, RenderNode};
use font8x8::{BASIC_FONTS, UnicodeFonts};
use minifb::{Key, Window, WindowOptions};

#[derive(Clone, Copy, Debug)]
pub struct FrameInfo {
    pub frame_index: u64,
    pub delta: Duration,
}

#[derive(Clone, Debug)]
pub struct WindowConfig {
    pub title: String,
    pub width: usize,
    pub height: usize,
    pub clear_color: Color,
    pub frame_time: Duration,
}

impl WindowConfig {
    pub fn new(title: impl Into<String>, width: usize, height: usize) -> Self {
        Self {
            title: title.into(),
            width,
            height,
            clear_color: Color::rgb(248, 250, 252),
            frame_time: Duration::from_millis(16),
        }
    }
}

#[derive(Debug)]
pub enum RendererError {
    Window(minifb::Error),
}

impl Display for RendererError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Window(source) => write!(f, "renderer backend error: {source}"),
        }
    }
}

impl Error for RendererError {}

impl From<minifb::Error> for RendererError {
    fn from(value: minifb::Error) -> Self {
        Self::Window(value)
    }
}

pub type Result<T> = std::result::Result<T, RendererError>;

pub fn run<F>(config: WindowConfig, mut render_scene: F) -> Result<()>
where
    F: FnMut(FrameInfo) -> Vec<RenderNode>,
{
    let mut window = Window::new(
        &config.title,
        config.width,
        config.height,
        WindowOptions::default(),
    )?;
    window.set_target_fps(frame_time_to_fps(config.frame_time));

    let mut buffer = vec![pack_rgb(config.clear_color); config.width * config.height];
    let mut last_frame = Instant::now();
    let mut frame_index = 0_u64;

    while window.is_open() && !window.is_key_down(Key::Escape) {
        let now = Instant::now();
        let delta = now.saturating_duration_since(last_frame);
        last_frame = now;

        let scene = render_scene(FrameInfo { frame_index, delta });
        render_to_buffer(
            &scene,
            &mut buffer,
            config.width,
            config.height,
            config.clear_color,
        );
        window.update_with_buffer(&buffer, config.width, config.height)?;
        frame_index += 1;
    }

    Ok(())
}

pub fn render_to_buffer(
    scene: &[RenderNode],
    buffer: &mut [u32],
    width: usize,
    height: usize,
    clear_color: Color,
) {
    buffer.fill(pack_rgb(clear_color));

    for node in scene {
        draw_node(node, buffer, width, height);
    }
}

fn draw_node(node: &RenderNode, buffer: &mut [u32], width: usize, height: usize) {
    if let Some(background) = node.style.background {
        draw_rect(buffer, width, height, node.layout, background);
    }

    if let RenderKind::Text(content) = &node.kind {
        draw_text(
            buffer,
            width,
            height,
            node.layout.x.round() as i32,
            node.layout.y.round() as i32,
            content,
            node.style.foreground,
        );
    }

    for child in &node.children {
        draw_node(child, buffer, width, height);
    }
}

fn draw_rect(buffer: &mut [u32], width: usize, height: usize, layout: LayoutBox, color: Color) {
    let x0 = layout.x.max(0.0) as i32;
    let y0 = layout.y.max(0.0) as i32;
    let x1 = (layout.x + layout.width).min(width as f32) as i32;
    let y1 = (layout.y + layout.height).min(height as f32) as i32;

    for y in y0..y1 {
        for x in x0..x1 {
            blend_pixel(buffer, width, height, x, y, color);
        }
    }
}

fn draw_text(
    buffer: &mut [u32],
    width: usize,
    height: usize,
    start_x: i32,
    start_y: i32,
    text: &str,
    color: Color,
) {
    let scale = 2_i32;
    let mut cursor_x = start_x;
    let mut cursor_y = start_y;

    for character in text.chars() {
        if character == '\n' {
            cursor_x = start_x;
            cursor_y += 10 * scale;
            continue;
        }

        if let Some(glyph) = BASIC_FONTS.get(character) {
            for (row_index, row) in glyph.iter().enumerate() {
                for column in 0..8 {
                    if ((*row >> column) & 1) == 0 {
                        continue;
                    }

                    for y_step in 0..scale {
                        for x_step in 0..scale {
                            let x = cursor_x + (column * scale) + x_step;
                            let y = cursor_y + (row_index as i32 * scale) + y_step;
                            blend_pixel(buffer, width, height, x, y, color);
                        }
                    }
                }
            }
        }

        cursor_x += 9 * scale;
    }
}

fn blend_pixel(buffer: &mut [u32], width: usize, height: usize, x: i32, y: i32, color: Color) {
    if x < 0 || y < 0 || x >= width as i32 || y >= height as i32 {
        return;
    }

    let index = y as usize * width + x as usize;
    let destination = unpack_rgb(buffer[index]);
    let alpha = color.a as f32 / 255.0;
    let inverse_alpha = 1.0 - alpha;
    let blended = Color::rgb(
        (color.r as f32 * alpha + destination.r as f32 * inverse_alpha).round() as u8,
        (color.g as f32 * alpha + destination.g as f32 * inverse_alpha).round() as u8,
        (color.b as f32 * alpha + destination.b as f32 * inverse_alpha).round() as u8,
    );

    buffer[index] = pack_rgb(blended);
}

fn pack_rgb(color: Color) -> u32 {
    ((color.r as u32) << 16) | ((color.g as u32) << 8) | color.b as u32
}

fn unpack_rgb(pixel: u32) -> Color {
    Color::rgb(
        ((pixel >> 16) & 0xFF) as u8,
        ((pixel >> 8) & 0xFF) as u8,
        (pixel & 0xFF) as u8,
    )
}

fn frame_time_to_fps(frame_time: Duration) -> usize {
    if frame_time.is_zero() {
        return 0;
    }

    (1.0 / frame_time.as_secs_f64()).round().max(1.0) as usize
}

#[cfg(test)]
mod tests {
    use cssimpler_core::{Color, LayoutBox, RenderNode, VisualStyle};

    use crate::{pack_rgb, render_to_buffer};

    #[test]
    fn offscreen_rendering_marks_the_expected_pixels() {
        let scene = vec![
            RenderNode::container(LayoutBox::new(2.0, 3.0, 6.0, 5.0)).with_style(VisualStyle {
                background: Some(Color::rgb(40, 120, 220)),
                ..VisualStyle::default()
            }),
        ];
        let mut buffer = vec![0_u32; 20 * 20];

        render_to_buffer(&scene, &mut buffer, 20, 20, Color::WHITE);

        assert!(buffer.contains(&pack_rgb(Color::rgb(40, 120, 220))));
    }
}
