use super::{
    blend_linear_over, clip_pixel_bounds, current_render_buffer_rows,
    shapes::{rounded_rect_coverage, transformed_rounded_rect_coverage},
    transform::{AffineTransform, ClipState, transform_layout_bounds},
};
use cssimpler_core::{
    CornerRadius, LayoutBox, LinearRgba,
    images::{ImageFit, RasterBackground, RasterImage},
};

pub(crate) fn draw(
    buffer: &mut [u32],
    width: usize,
    height: usize,
    layout: LayoutBox,
    radius: CornerRadius,
    style: &RasterBackground,
    matrix: AffineTransform,
    clip: &ClipState,
) {
    let Some(image) = style
        .source
        .as_deref()
        .and_then(cssimpler_core::images::image)
    else {
        return;
    };
    if layout.width <= 0.0 || layout.height <= 0.0 || style.opacity <= 0.0 {
        return;
    }
    let Some(inverse) = matrix.invert() else {
        return;
    };
    let Some(bounds) =
        transform_layout_bounds(layout, matrix).and_then(|b| b.intersect(clip.coarse))
    else {
        return;
    };
    let Some((x0, y0, x1, y1)) = clip_pixel_bounds(bounds, width, height) else {
        return;
    };
    let natural = (image.width as f32, image.height as f32);
    if natural.0 <= 0.0 || natural.1 <= 0.0 {
        return;
    }
    let (image_width, image_height) = match style.fit {
        ImageFit::Auto => natural,
        ImageFit::Fill => (layout.width, layout.height),
        ImageFit::Cover | ImageFit::Contain => {
            let x = layout.width / natural.0;
            let y = layout.height / natural.1;
            let scale = if style.fit == ImageFit::Cover {
                x.max(y)
            } else {
                x.min(y)
            };
            (natural.0 * scale, natural.1 * scale)
        }
    };
    if image_width <= 0.0 || image_height <= 0.0 {
        return;
    }
    let origin = (
        layout.x + (layout.width - image_width) * style.position.0,
        layout.y + (layout.height - image_height) * style.position.1,
    );
    let rows = current_render_buffer_rows();
    let row_start_y = y0.max(rows.start.min(height) as i32);
    let row_end_y = y1.min(rows.end.min(height) as i32);
    let simple = matrix.is_identity() && clip.is_simple();
    for y in row_start_y..row_end_y {
        let row_offset = (y as usize - rows.start) * width;
        for x in x0..x1 {
            let (sx, sy) = inverse.transform_point(x as f32 + 0.5, y as f32 + 0.5);
            let mut u = (sx - origin.0) / image_width;
            let mut v = (sy - origin.1) / image_height;
            if style.repeat.0 {
                u = u.rem_euclid(1.0);
            }
            if style.repeat.1 {
                v = v.rem_euclid(1.0);
            }
            if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
                continue;
            }
            let coverage = if simple {
                rounded_rect_coverage(layout, radius, clip.coarse, x, y)
            } else {
                transformed_rounded_rect_coverage(layout, radius, inverse, clip, x, y)
            };
            if coverage == 0 {
                continue;
            }
            let mut color = sample(&image, u, v);
            color.a *= style.opacity * coverage as f32 / 255.0;
            blend_linear_over(buffer, row_offset + x as usize, color);
        }
    }
}

fn sample(image: &RasterImage, u: f32, v: f32) -> LinearRgba {
    if image.width == 0 || image.height == 0 || image.pixels.is_empty() {
        return LinearRgba::TRANSPARENT;
    }
    let x = (u * image.width as f32 - 0.5).clamp(0.0, (image.width - 1) as f32);
    let y = (v * image.height as f32 - 0.5).clamp(0.0, (image.height - 1) as f32);
    let (x0, y0) = (x.floor() as usize, y.floor() as usize);
    let (x1, y1) = (
        (x0 + 1).min(image.width - 1),
        (y0 + 1).min(image.height - 1),
    );
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let mut out = LinearRgba {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 0.0,
    };
    for (x, y, weight) in [
        (x0, y0, (1.0 - fx) * (1.0 - fy)),
        (x1, y0, fx * (1.0 - fy)),
        (x0, y1, (1.0 - fx) * fy),
        (x1, y1, fx * fy),
    ] {
        let color = image
            .pixels
            .get(y * image.width + x)
            .map(|c| c.to_linear_rgba())
            .unwrap_or(LinearRgba::TRANSPARENT);
        let alpha = color.a * weight;
        out.r += color.r * alpha;
        out.g += color.g * alpha;
        out.b += color.b * alpha;
        out.a += alpha;
    }
    if out.a > 0.0 {
        out.r /= out.a;
        out.g /= out.a;
        out.b /= out.a;
    }
    out
}

