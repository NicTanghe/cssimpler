use std::error::Error;
use std::fmt::{Display, Formatter};
use std::time::{Duration, Instant};

use cssimpler_core::{Color, CornerRadius, EventHandler, Insets, LayoutBox, RenderKind, RenderNode};
use font8x8::{BASIC_FONTS, UnicodeFonts};
use minifb::{Key, MouseButton, MouseMode, Window, WindowOptions};

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

    let mut buffer_width = config.width.max(1);
    let mut buffer_height = config.height.max(1);
    let mut buffer = vec![pack_rgb(config.clear_color); buffer_width * buffer_height];
    let mut last_frame = Instant::now();
    let mut frame_index = 0_u64;
    let mut previous_left_down = false;
    let mut previous_presented_scene: Option<Vec<RenderNode>> = None;

    while window.is_open() && !window.is_key_down(Key::Escape) {
        let now = Instant::now();
        let delta = now.saturating_duration_since(last_frame);
        last_frame = now;

        let left_down = window.get_mouse_down(MouseButton::Left);
        let alt_dragging = should_suspend_updates(
            left_down,
            window.is_key_down(Key::LeftAlt),
            window.is_key_down(Key::RightAlt),
        );

        if alt_dragging {
            window.update();
            previous_left_down = left_down;
            frame_index += 1;
            continue;
        }

        let frame = FrameInfo { frame_index, delta };
        let mut scene = render_scene(frame);
        let click_started = left_down && !previous_left_down;

        if click_started
            && let Some((mouse_x, mouse_y)) = window.get_mouse_pos(MouseMode::Clamp)
            && dispatch_click(&scene, mouse_x, mouse_y)
        {
            scene = render_scene(frame);
        }

        let (window_width, window_height) = window.get_size();
        let resized = buffer_width != window_width.max(1) || buffer_height != window_height.max(1);
        resize_buffer(
            &mut buffer,
            &mut buffer_width,
            &mut buffer_height,
            window_width,
            window_height,
            config.clear_color,
        );
        if should_present_scene(previous_presented_scene.as_deref(), &scene, resized) {
            render_to_buffer(
                &scene,
                &mut buffer,
                buffer_width,
                buffer_height,
                config.clear_color,
            );
            window.update_with_buffer(&buffer, buffer_width, buffer_height)?;
            previous_presented_scene = Some(scene.clone());
        } else {
            window.update();
        }

        previous_left_down = left_down;
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
    let clip = ClipRect::full(width as f32, height as f32);

    for node in scene {
        draw_node(node, buffer, width, height, clip);
    }
}

fn should_present_scene(
    previous_scene: Option<&[RenderNode]>,
    scene: &[RenderNode],
    resized: bool,
) -> bool {
    if resized {
        return true;
    }

    let Some(previous_scene) = previous_scene else {
        return true;
    };

    !scenes_match_visuals(previous_scene, scene)
}

fn scenes_match_visuals(left: &[RenderNode], right: &[RenderNode]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right.iter())
            .all(|(left, right)| render_nodes_match_visuals(left, right))
}

fn render_nodes_match_visuals(left: &RenderNode, right: &RenderNode) -> bool {
    left.kind == right.kind
        && left.layout == right.layout
        && left.style == right.style
        && left.content_inset == right.content_inset
        && scenes_match_visuals(&left.children, &right.children)
}

fn should_suspend_updates(left_down: bool, left_alt_down: bool, right_alt_down: bool) -> bool {
    left_down && (left_alt_down || right_alt_down)
}

#[derive(Clone, Copy, Debug)]
struct ClipRect {
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
}

impl ClipRect {
    fn full(width: f32, height: f32) -> Self {
        Self {
            x0: 0.0,
            y0: 0.0,
            x1: width,
            y1: height,
        }
    }

    fn unbounded() -> Self {
        Self {
            x0: f32::MIN,
            y0: f32::MIN,
            x1: f32::MAX,
            y1: f32::MAX,
        }
    }

    fn intersect(self, other: Self) -> Option<Self> {
        let clipped = Self {
            x0: self.x0.max(other.x0),
            y0: self.y0.max(other.y0),
            x1: self.x1.min(other.x1),
            y1: self.y1.min(other.y1),
        };

        (!clipped.is_empty()).then_some(clipped)
    }

    fn contains(self, x: f32, y: f32) -> bool {
        x >= self.x0 && y >= self.y0 && x < self.x1 && y < self.y1
    }

    fn is_empty(self) -> bool {
        self.x0 >= self.x1 || self.y0 >= self.y1
    }
}

fn draw_node(node: &RenderNode, buffer: &mut [u32], width: usize, height: usize, clip: ClipRect) {
    if clip.is_empty() || !clip.intersect(layout_clip(node.layout)).is_some() {
        return;
    }

    for shadow in &node.style.shadows {
        draw_shadow(
            buffer,
            width,
            height,
            node.layout,
            node.style.corner_radius,
            *shadow,
            clip,
        );
    }

    draw_background_and_border(node, buffer, width, height, clip);

    if let RenderKind::Text(content) = &node.kind {
        let text_layout = inset_layout(node.layout, node.content_inset);
        let text_clip = clip
            .intersect(layout_clip(text_layout))
            .unwrap_or(ClipRect::full(0.0, 0.0));
        draw_text(
            buffer,
            width,
            height,
            text_layout,
            content,
            node.style.foreground,
            text_clip,
        );
    }

    let child_clip = if node.style.overflow.clips_any_axis() {
        clip.intersect(layout_clip(node.layout))
    } else {
        Some(clip)
    };

    let Some(child_clip) = child_clip else {
        return;
    };

    for child in &node.children {
        draw_node(child, buffer, width, height, child_clip);
    }
}

fn dispatch_click(scene: &[RenderNode], x: f32, y: f32) -> bool {
    let Some(handler) = hit_test_scene(scene, x, y) else {
        return false;
    };

    handler();
    true
}

fn hit_test_scene(scene: &[RenderNode], x: f32, y: f32) -> Option<EventHandler> {
    scene
        .iter()
        .rev()
        .find_map(|node| hit_test_node(node, x, y, ClipRect::unbounded()))
}

fn hit_test_node(node: &RenderNode, x: f32, y: f32, clip: ClipRect) -> Option<EventHandler> {
    if !clip.contains(x, y) || !layout_contains(node.layout, x, y) {
        return None;
    }

    let child_clip = if node.style.overflow.clips_any_axis() {
        clip.intersect(layout_clip(node.layout))?
    } else {
        clip
    };

    for child in node.children.iter().rev() {
        if let Some(handler) = hit_test_node(child, x, y, child_clip) {
            return Some(handler);
        }
    }

    node.on_click
}

fn layout_contains(layout: LayoutBox, x: f32, y: f32) -> bool {
    x >= layout.x && y >= layout.y && x < layout.x + layout.width && y < layout.y + layout.height
}

fn draw_background_and_border(
    node: &RenderNode,
    buffer: &mut [u32],
    width: usize,
    height: usize,
    clip: ClipRect,
) {
    if !node.style.border.widths.is_zero() {
        let inner_layout = inset_layout(node.layout, node.style.border.widths);
        let inner_radius = inset_corner_radius(node.style.corner_radius, node.style.border.widths);

        draw_rounded_ring(
            buffer,
            width,
            height,
            node.layout,
            node.style.corner_radius,
            Some((inner_layout, inner_radius)),
            node.style.border.color,
            clip,
        );
    }

    if let Some(background) = node.style.background {
        let fill_layout = if node.style.border.widths.is_zero() {
            node.layout
        } else {
            inset_layout(node.layout, node.style.border.widths)
        };
        let fill_radius = if node.style.border.widths.is_zero() {
            node.style.corner_radius
        } else {
            inset_corner_radius(node.style.corner_radius, node.style.border.widths)
        };
        draw_rounded_rect(
            buffer,
            width,
            height,
            fill_layout,
            fill_radius,
            background,
            clip,
        );
    }
}

fn draw_shadow(
    buffer: &mut [u32],
    width: usize,
    height: usize,
    layout: LayoutBox,
    radius: CornerRadius,
    shadow: cssimpler_core::BoxShadow,
    clip: ClipRect,
) {
    let base_layout =
        offset_layout(expand_layout(layout, shadow.spread), shadow.offset_x, shadow.offset_y);
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

    let blurred_bounds = expand_layout(base_layout, blur_radius);
    let Some((x0, y0, x1, y1)) = pixel_bounds(blurred_bounds, clip, width, height) else {
        return;
    };

    for y in y0..y1 {
        for x in x0..x1 {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let alpha = shadow_alpha(px, py, base_layout, base_radius, blur_radius, shadow.color.a);
            if alpha == 0 {
                continue;
            }

            blend_pixel(buffer, width, height, x, y, shadow.color.with_alpha(alpha));
        }
    }
}

fn draw_rounded_rect(
    buffer: &mut [u32],
    width: usize,
    height: usize,
    layout: LayoutBox,
    radius: CornerRadius,
    color: Color,
    clip: ClipRect,
) {
    let Some((x0, y0, x1, y1)) = pixel_bounds(layout, clip, width, height) else {
        return;
    };

    for y in y0..y1 {
        for x in x0..x1 {
            if point_in_rounded_rect(x as f32 + 0.5, y as f32 + 0.5, layout, radius) {
                blend_pixel(buffer, width, height, x, y, color);
            }
        }
    }
}

fn draw_rounded_ring(
    buffer: &mut [u32],
    width: usize,
    height: usize,
    outer_layout: LayoutBox,
    outer_radius: CornerRadius,
    inner: Option<(LayoutBox, CornerRadius)>,
    color: Color,
    clip: ClipRect,
) {
    let Some((x0, y0, x1, y1)) = pixel_bounds(outer_layout, clip, width, height) else {
        return;
    };

    for y in y0..y1 {
        for x in x0..x1 {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            if !point_in_rounded_rect(px, py, outer_layout, outer_radius) {
                continue;
            }

            if let Some((inner_layout, inner_radius)) = inner
                && point_in_rounded_rect(px, py, inner_layout, inner_radius)
            {
                continue;
            }

            blend_pixel(buffer, width, height, x, y, color);
        }
    }
}

fn draw_text(
    buffer: &mut [u32],
    width: usize,
    height: usize,
    layout: LayoutBox,
    text: &str,
    color: Color,
    clip: ClipRect,
) {
    let scale = 2_i32;
    let start_x = layout.x.round() as i32;
    let mut cursor_x = start_x;
    let mut cursor_y = layout.y.round() as i32;
    let lines = wrap_text(text, layout.width);

    for (line_index, line) in lines.iter().enumerate() {
        if line_index > 0 {
            cursor_x = start_x;
            cursor_y += 10 * scale;
        }

        for character in line.chars() {
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
                                if !clip.contains(x as f32 + 0.5, y as f32 + 0.5) {
                                    continue;
                                }
                                blend_pixel(buffer, width, height, x, y, color);
                            }
                        }
                    }
                }
            }

            cursor_x += 9 * scale;
        }
    }
}

fn resize_buffer(
    buffer: &mut Vec<u32>,
    width: &mut usize,
    height: &mut usize,
    next_width: usize,
    next_height: usize,
    clear_color: Color,
) {
    let next_width = next_width.max(1);
    let next_height = next_height.max(1);

    if *width == next_width && *height == next_height {
        return;
    }

    *width = next_width;
    *height = next_height;
    buffer.resize(next_width * next_height, pack_rgb(clear_color));
}

fn layout_clip(layout: LayoutBox) -> ClipRect {
    ClipRect {
        x0: layout.x,
        y0: layout.y,
        x1: layout.x + layout.width,
        y1: layout.y + layout.height,
    }
}

fn pixel_bounds(
    layout: LayoutBox,
    clip: ClipRect,
    width: usize,
    height: usize,
) -> Option<(i32, i32, i32, i32)> {
    let clip = clip.intersect(ClipRect::full(width as f32, height as f32))?;
    let x0 = layout.x.max(clip.x0).floor().max(0.0) as i32;
    let y0 = layout.y.max(clip.y0).floor().max(0.0) as i32;
    let x1 = (layout.x + layout.width).min(clip.x1).ceil().min(width as f32) as i32;
    let y1 = (layout.y + layout.height).min(clip.y1).ceil().min(height as f32) as i32;
    (x0 < x1 && y0 < y1).then_some((x0, y0, x1, y1))
}

fn point_in_rounded_rect(x: f32, y: f32, layout: LayoutBox, radius: CornerRadius) -> bool {
    if !layout_contains(layout, x, y) {
        return false;
    }

    let radius = clamp_corner_radius(radius, layout.width, layout.height);
    if radius.top_left == 0.0
        && radius.top_right == 0.0
        && radius.bottom_right == 0.0
        && radius.bottom_left == 0.0
    {
        return true;
    }

    if x < layout.x + radius.top_left && y < layout.y + radius.top_left {
        return point_in_corner(
            x,
            y,
            layout.x + radius.top_left,
            layout.y + radius.top_left,
            radius.top_left,
        );
    }

    if x > layout.x + layout.width - radius.top_right && y < layout.y + radius.top_right {
        return point_in_corner(
            x,
            y,
            layout.x + layout.width - radius.top_right,
            layout.y + radius.top_right,
            radius.top_right,
        );
    }

    if x > layout.x + layout.width - radius.bottom_right
        && y > layout.y + layout.height - radius.bottom_right
    {
        return point_in_corner(
            x,
            y,
            layout.x + layout.width - radius.bottom_right,
            layout.y + layout.height - radius.bottom_right,
            radius.bottom_right,
        );
    }

    if x < layout.x + radius.bottom_left && y > layout.y + layout.height - radius.bottom_left {
        return point_in_corner(
            x,
            y,
            layout.x + radius.bottom_left,
            layout.y + layout.height - radius.bottom_left,
            radius.bottom_left,
        );
    }

    true
}

fn point_in_corner(x: f32, y: f32, center_x: f32, center_y: f32, radius: f32) -> bool {
    if radius <= 0.0 {
        return true;
    }

    let dx = x - center_x;
    let dy = y - center_y;
    (dx * dx) + (dy * dy) <= radius * radius
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

fn inset_layout(layout: LayoutBox, insets: Insets) -> LayoutBox {
    let width = (layout.width - insets.left - insets.right).max(0.0);
    let height = (layout.height - insets.top - insets.bottom).max(0.0);
    LayoutBox::new(layout.x + insets.left, layout.y + insets.top, width, height)
}

fn inset_corner_radius(radius: CornerRadius, insets: Insets) -> CornerRadius {
    CornerRadius {
        top_left: (radius.top_left - insets.top.max(insets.left)).max(0.0),
        top_right: (radius.top_right - insets.top.max(insets.right)).max(0.0),
        bottom_right: (radius.bottom_right - insets.bottom.max(insets.right)).max(0.0),
        bottom_left: (radius.bottom_left - insets.bottom.max(insets.left)).max(0.0),
    }
}

fn expand_layout(layout: LayoutBox, amount: f32) -> LayoutBox {
    let width = (layout.width + amount * 2.0).max(0.0);
    let height = (layout.height + amount * 2.0).max(0.0);
    LayoutBox::new(layout.x - amount, layout.y - amount, width, height)
}

fn offset_layout(layout: LayoutBox, x: f32, y: f32) -> LayoutBox {
    LayoutBox::new(layout.x + x, layout.y + y, layout.width, layout.height)
}

fn expand_corner_radius(radius: CornerRadius, amount: f32) -> CornerRadius {
    CornerRadius {
        top_left: (radius.top_left + amount).max(0.0),
        top_right: (radius.top_right + amount).max(0.0),
        bottom_right: (radius.bottom_right + amount).max(0.0),
        bottom_left: (radius.bottom_left + amount).max(0.0),
    }
}

fn wrap_text(text: &str, width: f32) -> Vec<String> {
    let max_columns = ((width.max(18.0)) / 18.0).floor().max(1.0) as usize;
    let mut wrapped = Vec::new();
    for source_line in text.lines() {
        wrap_line(source_line, max_columns, &mut wrapped);
    }
    if wrapped.is_empty() {
        wrapped.push(String::new());
    }
    wrapped
}

fn wrap_line(line: &str, max_columns: usize, wrapped: &mut Vec<String>) {
    if line.is_empty() {
        wrapped.push(String::new());
        return;
    }

    let mut current = String::new();
    for word in line.split_whitespace() {
        let word_len = word.chars().count();
        let spacing = usize::from(!current.is_empty());

        if word_len > max_columns {
            if !current.is_empty() {
                wrapped.push(std::mem::take(&mut current));
            }
            push_broken_word(word, max_columns, wrapped);
            continue;
        }

        if current.chars().count() + spacing + word_len > max_columns {
            wrapped.push(std::mem::take(&mut current));
        }

        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }

    if !current.is_empty() {
        wrapped.push(current);
    }
}

fn push_broken_word(word: &str, max_columns: usize, wrapped: &mut Vec<String>) {
    let mut segment = String::new();
    for character in word.chars() {
        if segment.chars().count() == max_columns {
            wrapped.push(std::mem::take(&mut segment));
        }
        segment.push(character);
    }
    if !segment.is_empty() {
        wrapped.push(segment);
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
    use std::sync::atomic::{AtomicUsize, Ordering};

    use cssimpler_core::{
        BoxShadow, Color, CornerRadius, LayoutBox, Overflow, RenderNode, VisualStyle,
    };

    use crate::{
        dispatch_click, pack_rgb, render_to_buffer, resize_buffer, scenes_match_visuals,
        should_present_scene, should_suspend_updates,
    };

    static CLICK_COUNT: AtomicUsize = AtomicUsize::new(0);
    static CLICK_TARGET: AtomicUsize = AtomicUsize::new(0);

    fn increment_click_count() {
        CLICK_COUNT.fetch_add(1, Ordering::SeqCst);
    }

    fn mark_parent_clicked() {
        CLICK_TARGET.store(1, Ordering::SeqCst);
    }

    fn mark_child_clicked() {
        CLICK_TARGET.store(2, Ordering::SeqCst);
    }

    fn alternate_click_handler() {
        CLICK_COUNT.fetch_add(10, Ordering::SeqCst);
    }

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

    #[test]
    fn rounded_background_respects_corner_radius() {
        let scene = vec![
            RenderNode::container(LayoutBox::new(2.0, 2.0, 8.0, 8.0)).with_style(VisualStyle {
                background: Some(Color::rgb(40, 120, 220)),
                corner_radius: CornerRadius::all(4.0),
                ..VisualStyle::default()
            }),
        ];
        let mut buffer = vec![0_u32; 16 * 16];

        render_to_buffer(&scene, &mut buffer, 16, 16, Color::WHITE);

        assert_eq!(buffer[2 * 16 + 2], pack_rgb(Color::WHITE));
        assert_eq!(buffer[6 * 16 + 6], pack_rgb(Color::rgb(40, 120, 220)));
    }

    #[test]
    fn overflow_clip_hides_child_pixels_outside_parent_bounds() {
        let scene = vec![
            RenderNode::container(LayoutBox::new(4.0, 4.0, 6.0, 6.0))
                .with_style(VisualStyle {
                    overflow: Overflow::CLIP,
                    ..VisualStyle::default()
                })
                .with_child(
                    RenderNode::container(LayoutBox::new(6.0, 6.0, 10.0, 10.0)).with_style(
                        VisualStyle {
                            background: Some(Color::rgb(220, 38, 38)),
                            ..VisualStyle::default()
                        },
                    ),
                ),
        ];
        let mut buffer = vec![0_u32; 20 * 20];

        render_to_buffer(&scene, &mut buffer, 20, 20, Color::WHITE);

        assert_eq!(buffer[8 * 20 + 8], pack_rgb(Color::rgb(220, 38, 38)));
        assert_eq!(buffer[14 * 20 + 14], pack_rgb(Color::WHITE));
    }

    #[test]
    fn box_shadow_renders_behind_the_element() {
        let scene = vec![
            RenderNode::container(LayoutBox::new(6.0, 6.0, 6.0, 6.0)).with_style(VisualStyle {
                shadows: vec![BoxShadow {
                    color: Color::rgba(15, 23, 42, 160),
                    offset_x: 2.0,
                    offset_y: 2.0,
                    blur_radius: 0.0,
                    spread: 0.0,
                }],
                ..VisualStyle::default()
            }),
        ];
        let mut buffer = vec![0_u32; 20 * 20];

        render_to_buffer(&scene, &mut buffer, 20, 20, Color::WHITE);

        assert_ne!(buffer[13 * 20 + 13], pack_rgb(Color::WHITE));
    }

    #[test]
    fn dispatch_click_invokes_the_hit_handler() {
        CLICK_COUNT.store(0, Ordering::SeqCst);
        let scene = vec![
            RenderNode::container(LayoutBox::new(4.0, 6.0, 40.0, 24.0))
                .on_click(increment_click_count),
        ];

        assert!(dispatch_click(&scene, 12.0, 12.0));
        assert_eq!(CLICK_COUNT.load(Ordering::SeqCst), 1);
        assert!(!dispatch_click(&scene, 80.0, 80.0));
    }

    #[test]
    fn dispatch_click_prefers_the_topmost_child() {
        CLICK_TARGET.store(0, Ordering::SeqCst);
        let scene = vec![
            RenderNode::container(LayoutBox::new(0.0, 0.0, 80.0, 60.0))
                .on_click(mark_parent_clicked)
                .with_child(
                    RenderNode::container(LayoutBox::new(12.0, 10.0, 30.0, 20.0))
                        .on_click(mark_child_clicked),
                ),
        ];

        assert!(dispatch_click(&scene, 20.0, 18.0));
        assert_eq!(CLICK_TARGET.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn hit_testing_respects_parent_clipping() {
        CLICK_COUNT.store(0, Ordering::SeqCst);
        let scene = vec![
            RenderNode::container(LayoutBox::new(0.0, 0.0, 20.0, 20.0))
                .with_style(VisualStyle {
                    overflow: Overflow::CLIP,
                    ..VisualStyle::default()
                })
                .with_child(
                    RenderNode::container(LayoutBox::new(10.0, 10.0, 20.0, 20.0))
                        .on_click(increment_click_count),
                ),
        ];

        assert!(dispatch_click(&scene, 12.0, 12.0));
        assert_eq!(CLICK_COUNT.load(Ordering::SeqCst), 1);
        assert!(!dispatch_click(&scene, 28.0, 28.0));
    }

    #[test]
    fn resize_buffer_tracks_the_latest_window_size_without_scaling() {
        let mut width = 320;
        let mut height = 180;
        let mut buffer = vec![0_u32; width * height];

        resize_buffer(
            &mut buffer,
            &mut width,
            &mut height,
            640,
            360,
            Color::WHITE,
        );

        assert_eq!(width, 640);
        assert_eq!(height, 360);
        assert_eq!(buffer.len(), 640 * 360);
    }

    #[test]
    fn visual_scene_comparison_ignores_click_handlers() {
        let left = vec![
            RenderNode::container(LayoutBox::new(4.0, 6.0, 40.0, 24.0)).on_click(increment_click_count),
        ];
        let right = vec![
            RenderNode::container(LayoutBox::new(4.0, 6.0, 40.0, 24.0))
                .on_click(alternate_click_handler),
        ];

        assert!(scenes_match_visuals(&left, &right));
    }

    #[test]
    fn should_present_scene_when_visuals_change() {
        let previous = vec![
            RenderNode::container(LayoutBox::new(4.0, 6.0, 40.0, 24.0)).with_style(VisualStyle {
                background: Some(Color::rgb(40, 120, 220)),
                ..VisualStyle::default()
            }),
        ];
        let next = vec![
            RenderNode::container(LayoutBox::new(4.0, 6.0, 40.0, 24.0)).with_style(VisualStyle {
                background: Some(Color::rgb(220, 38, 38)),
                ..VisualStyle::default()
            }),
        ];

        assert!(should_present_scene(Some(&previous), &next, false));
        assert!(!should_present_scene(Some(&previous), &previous, false));
    }

    #[test]
    fn should_present_scene_when_buffer_is_resized() {
        let scene = vec![
            RenderNode::container(LayoutBox::new(4.0, 6.0, 40.0, 24.0)),
        ];

        assert!(should_present_scene(Some(&scene), &scene, true));
    }

    #[test]
    fn alt_dragging_suspends_updates() {
        assert!(should_suspend_updates(true, true, false));
        assert!(should_suspend_updates(true, false, true));
        assert!(!should_suspend_updates(true, false, false));
        assert!(!should_suspend_updates(false, true, true));
    }
}
