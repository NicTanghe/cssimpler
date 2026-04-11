use cssimpler_core::{CornerRadius, LayoutBox, LinearRgba};

use super::shapes::{
    clip_pixel_bounds, layout_clip, point_in_rounded_rect, transformed_rounded_rect_coverage,
};
use super::transform::{AffineTransform, ClipState, transform_layout_bounds};
use super::{ClipRect, buffer_pixel_index, pack_linear_rgb, unpack_rgb};

pub(crate) fn draw_backdrop_blur(
    buffer: &mut [u32],
    width: usize,
    height: usize,
    layout: LayoutBox,
    radius: CornerRadius,
    blur_radius: f32,
    clip: ClipRect,
) {
    let kernel_radius = blur_kernel_radius(blur_radius);
    if kernel_radius == 0 {
        return;
    }

    let Some(destination_bounds) = clip.intersect(layout_clip(layout)) else {
        return;
    };
    let Some(snapshot) = blurred_snapshot(
        buffer,
        width,
        height,
        layout_clip(layout).expand(kernel_radius as f32),
        kernel_radius,
    ) else {
        return;
    };
    let Some((x0, y0, x1, y1)) = clip_pixel_bounds(destination_bounds, width, height) else {
        return;
    };

    for y in y0..y1 {
        for x in x0..x1 {
            if !point_in_rounded_rect(x as f32 + 0.5, y as f32 + 0.5, layout, radius) {
                continue;
            }
            let Some(color) = snapshot.pixel_at_screen(x, y) else {
                continue;
            };
            blend_backdrop_pixel(buffer, width, height, x, y, color, u8::MAX);
        }
    }
}

pub(crate) fn draw_backdrop_blur_transformed(
    buffer: &mut [u32],
    width: usize,
    height: usize,
    layout: LayoutBox,
    radius: CornerRadius,
    blur_radius: f32,
    matrix: AffineTransform,
    clip_state: &ClipState,
) {
    let kernel_radius = blur_kernel_radius(blur_radius);
    if kernel_radius == 0 {
        return;
    }

    let Some(destination_bounds) = transform_layout_bounds(layout, matrix)
        .and_then(|bounds| bounds.intersect(clip_state.coarse))
    else {
        return;
    };
    let Some(snapshot) = blurred_snapshot(
        buffer,
        width,
        height,
        destination_bounds.expand(kernel_radius as f32),
        kernel_radius,
    ) else {
        return;
    };
    let Some(inverse) = matrix.invert() else {
        return;
    };
    let Some((x0, y0, x1, y1)) = clip_pixel_bounds(destination_bounds, width, height) else {
        return;
    };

    for y in y0..y1 {
        for x in x0..x1 {
            let coverage =
                transformed_rounded_rect_coverage(layout, radius, inverse, clip_state, x, y);
            if coverage == 0 {
                continue;
            }
            let Some(color) = snapshot.pixel_at_screen(x, y) else {
                continue;
            };
            blend_backdrop_pixel(buffer, width, height, x, y, color, coverage);
        }
    }
}

fn blur_kernel_radius(blur_radius: f32) -> usize {
    blur_radius.max(0.0).ceil() as usize
}

#[derive(Clone)]
struct BackdropSnapshot {
    origin_x: i32,
    origin_y: i32,
    width: usize,
    height: usize,
    pixels: Vec<LinearRgba>,
}

impl BackdropSnapshot {
    fn pixel_at_screen(&self, x: i32, y: i32) -> Option<LinearRgba> {
        if x < self.origin_x || y < self.origin_y {
            return None;
        }
        let local_x = (x - self.origin_x) as usize;
        let local_y = (y - self.origin_y) as usize;
        if local_x >= self.width || local_y >= self.height {
            return None;
        }
        Some(self.pixels[local_y * self.width + local_x])
    }
}

fn blurred_snapshot(
    buffer: &[u32],
    width: usize,
    height: usize,
    bounds: ClipRect,
    kernel_radius: usize,
) -> Option<BackdropSnapshot> {
    let (x0, y0, x1, y1) = clip_pixel_bounds(bounds, width, height)?;
    let snapshot_width = (x1 - x0).max(0) as usize;
    let snapshot_height = (y1 - y0).max(0) as usize;
    if snapshot_width == 0 || snapshot_height == 0 {
        return None;
    }

    let mut pixels = Vec::with_capacity(snapshot_width.saturating_mul(snapshot_height));
    for y in y0..y1 {
        let row_start = y as usize * width;
        for x in x0..x1 {
            pixels.push(unpack_rgb(buffer[row_start + x as usize]).to_linear_rgba());
        }
    }

    Some(BackdropSnapshot {
        origin_x: x0,
        origin_y: y0,
        width: snapshot_width,
        height: snapshot_height,
        pixels: box_blur(&pixels, snapshot_width, snapshot_height, kernel_radius),
    })
}

fn box_blur(source: &[LinearRgba], width: usize, height: usize, radius: usize) -> Vec<LinearRgba> {
    if radius == 0 || width == 0 || height == 0 {
        return source.to_vec();
    }

    let mut horizontal = vec![LinearRgba::TRANSPARENT; width.saturating_mul(height)];
    let mut prefix_r = vec![0.0; width + 1];
    let mut prefix_g = vec![0.0; width + 1];
    let mut prefix_b = vec![0.0; width + 1];
    let mut prefix_a = vec![0.0; width + 1];

    for y in 0..height {
        prefix_r.fill(0.0);
        prefix_g.fill(0.0);
        prefix_b.fill(0.0);
        prefix_a.fill(0.0);
        let row_start = y * width;
        for x in 0..width {
            let pixel = source[row_start + x];
            prefix_r[x + 1] = prefix_r[x] + pixel.r;
            prefix_g[x + 1] = prefix_g[x] + pixel.g;
            prefix_b[x + 1] = prefix_b[x] + pixel.b;
            prefix_a[x + 1] = prefix_a[x] + pixel.a;
        }
        for x in 0..width {
            let x0 = x.saturating_sub(radius);
            let x1 = (x + radius + 1).min(width);
            let count = (x1 - x0) as f32;
            horizontal[row_start + x] = LinearRgba {
                r: (prefix_r[x1] - prefix_r[x0]) / count,
                g: (prefix_g[x1] - prefix_g[x0]) / count,
                b: (prefix_b[x1] - prefix_b[x0]) / count,
                a: (prefix_a[x1] - prefix_a[x0]) / count,
            };
        }
    }

    let mut vertical = vec![LinearRgba::TRANSPARENT; width.saturating_mul(height)];
    let mut prefix_r = vec![0.0; height + 1];
    let mut prefix_g = vec![0.0; height + 1];
    let mut prefix_b = vec![0.0; height + 1];
    let mut prefix_a = vec![0.0; height + 1];

    for x in 0..width {
        prefix_r.fill(0.0);
        prefix_g.fill(0.0);
        prefix_b.fill(0.0);
        prefix_a.fill(0.0);
        for y in 0..height {
            let pixel = horizontal[y * width + x];
            prefix_r[y + 1] = prefix_r[y] + pixel.r;
            prefix_g[y + 1] = prefix_g[y] + pixel.g;
            prefix_b[y + 1] = prefix_b[y] + pixel.b;
            prefix_a[y + 1] = prefix_a[y] + pixel.a;
        }
        for y in 0..height {
            let y0 = y.saturating_sub(radius);
            let y1 = (y + radius + 1).min(height);
            let count = (y1 - y0) as f32;
            vertical[y * width + x] = LinearRgba {
                r: (prefix_r[y1] - prefix_r[y0]) / count,
                g: (prefix_g[y1] - prefix_g[y0]) / count,
                b: (prefix_b[y1] - prefix_b[y0]) / count,
                a: (prefix_a[y1] - prefix_a[y0]) / count,
            };
        }
    }

    vertical
}

fn blend_backdrop_pixel(
    buffer: &mut [u32],
    width: usize,
    height: usize,
    x: i32,
    y: i32,
    color: LinearRgba,
    coverage: u8,
) {
    let Some(index) = buffer_pixel_index(width, height, x, y) else {
        return;
    };
    if coverage == 0 {
        return;
    }
    if coverage == u8::MAX {
        buffer[index] = pack_linear_rgb(color);
        return;
    }

    let alpha = coverage as f32 / 255.0;
    let inverse_alpha = 1.0 - alpha;
    let destination = unpack_rgb(buffer[index]).to_linear_rgba();
    buffer[index] = pack_linear_rgb(LinearRgba {
        r: color.r * alpha + destination.r * inverse_alpha,
        g: color.g * alpha + destination.g * inverse_alpha,
        b: color.b * alpha + destination.b * inverse_alpha,
        a: 1.0,
    });
}
