use cssimpler_core::{Color, CornerRadius, Insets, LayoutBox};

use super::{
    ClipRect, PreparedBlendColor, blend_prepared_pixel, current_render_buffer_rows, pack_rgb,
};

pub(crate) fn draw_rounded_rect(
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

    let prepared_color = PreparedBlendColor::new(color);
    for y in y0..y1 {
        if let Some((span_x0, span_x1)) = rounded_rect_row_span(layout, radius, y, x0, x1) {
            blend_prepared_span_row(buffer, width, height, span_x0, span_x1, y, prepared_color);
        }
    }
}

pub(crate) fn draw_axis_aligned_opaque_rect(
    buffer: &mut [u32],
    width: usize,
    height: usize,
    layout: LayoutBox,
    radius: CornerRadius,
    color: Color,
    clip: ClipRect,
) -> bool {
    if color.a != u8::MAX || !corner_radius_is_zero(layout, radius) {
        return false;
    }

    let Some((x0, y0, x1, y1)) = opaque_fill_pixel_bounds(layout, clip, width, height) else {
        return true;
    };
    fill_opaque_span_rows(buffer, width, x0, x1, y0, y1, pack_rgb(color));
    true
}

pub(crate) fn draw_rounded_ring(
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

    let prepared_color = PreparedBlendColor::new(color);
    for y in y0..y1 {
        let Some((outer_x0, outer_x1)) =
            rounded_rect_row_span(outer_layout, outer_radius, y, x0, x1)
        else {
            continue;
        };

        let Some((inner_layout, inner_radius)) = inner else {
            blend_prepared_span_row(buffer, width, height, outer_x0, outer_x1, y, prepared_color);
            continue;
        };

        let inner_span = rounded_rect_row_span(inner_layout, inner_radius, y, outer_x0, outer_x1);
        match inner_span {
            Some((inner_x0, inner_x1)) => {
                blend_prepared_span_row(
                    buffer,
                    width,
                    height,
                    outer_x0,
                    inner_x0,
                    y,
                    prepared_color,
                );
                blend_prepared_span_row(
                    buffer,
                    width,
                    height,
                    inner_x1,
                    outer_x1,
                    y,
                    prepared_color,
                );
            }
            None => {
                blend_prepared_span_row(
                    buffer,
                    width,
                    height,
                    outer_x0,
                    outer_x1,
                    y,
                    prepared_color,
                );
            }
        }
    }
}

pub(crate) fn draw_axis_aligned_opaque_ring(
    buffer: &mut [u32],
    width: usize,
    height: usize,
    outer_layout: LayoutBox,
    outer_radius: CornerRadius,
    inner: Option<(LayoutBox, CornerRadius)>,
    color: Color,
    clip: ClipRect,
) -> bool {
    if color.a != u8::MAX || !corner_radius_is_zero(outer_layout, outer_radius) {
        return false;
    }
    if let Some((inner_layout, inner_radius)) = inner
        && !corner_radius_is_zero(inner_layout, inner_radius)
    {
        return false;
    }

    let Some((outer_x0, outer_y0, outer_x1, outer_y1)) =
        opaque_fill_pixel_bounds(outer_layout, clip, width, height)
    else {
        return true;
    };
    let packed = pack_rgb(color);

    let Some((inner_layout, _)) = inner else {
        fill_opaque_span_rows(
            buffer, width, outer_x0, outer_x1, outer_y0, outer_y1, packed,
        );
        return true;
    };

    let Some((inner_x0, inner_y0, inner_x1, inner_y1)) =
        center_pixel_bounds(inner_layout, width, height)
    else {
        fill_opaque_span_rows(
            buffer, width, outer_x0, outer_x1, outer_y0, outer_y1, packed,
        );
        return true;
    };

    fill_opaque_span_rows(
        buffer,
        width,
        outer_x0,
        outer_x1,
        outer_y0,
        inner_y0.min(outer_y1),
        packed,
    );
    fill_opaque_span_rows(
        buffer,
        width,
        outer_x0,
        outer_x1,
        inner_y1.max(outer_y0),
        outer_y1,
        packed,
    );

    let middle_y0 = inner_y0.max(outer_y0);
    let middle_y1 = inner_y1.min(outer_y1);
    if middle_y0 < middle_y1 {
        fill_opaque_span_rows(
            buffer,
            width,
            outer_x0,
            inner_x0.min(outer_x1),
            middle_y0,
            middle_y1,
            packed,
        );
        fill_opaque_span_rows(
            buffer,
            width,
            inner_x1.max(outer_x0),
            outer_x1,
            middle_y0,
            middle_y1,
            packed,
        );
    }

    true
}

pub(crate) fn layout_clip(layout: LayoutBox) -> ClipRect {
    ClipRect {
        x0: layout.x,
        y0: layout.y,
        x1: layout.x + layout.width,
        y1: layout.y + layout.height,
    }
}

pub(crate) fn pixel_bounds(
    layout: LayoutBox,
    clip: ClipRect,
    width: usize,
    height: usize,
) -> Option<(i32, i32, i32, i32)> {
    let clip = clip.intersect(ClipRect::full(width as f32, height as f32))?;
    let x0 = layout.x.max(clip.x0).floor().max(0.0) as i32;
    let y0 = layout.y.max(clip.y0).floor().max(0.0) as i32;
    let x1 = (layout.x + layout.width)
        .min(clip.x1)
        .ceil()
        .min(width as f32) as i32;
    let y1 = (layout.y + layout.height)
        .min(clip.y1)
        .ceil()
        .min(height as f32) as i32;
    (x0 < x1 && y0 < y1).then_some((x0, y0, x1, y1))
}

pub(crate) fn opaque_fill_pixel_bounds(
    layout: LayoutBox,
    clip: ClipRect,
    width: usize,
    height: usize,
) -> Option<(i32, i32, i32, i32)> {
    let clip = clip.intersect(ClipRect::full(width as f32, height as f32))?;
    let x0 = layout.x.max(clip.x0).floor().max(0.0) as i32;
    let y0 = layout.y.max(clip.y0).floor().max(0.0) as i32;
    let x1 = (layout.x + layout.width)
        .min(clip.x1)
        .ceil()
        .min(width as f32) as i32;
    let y1 = (layout.y + layout.height)
        .min(clip.y1)
        .ceil()
        .min(height as f32) as i32;
    let center_x0 = (layout.x - 0.5).ceil().max(0.0) as i32;
    let center_y0 = (layout.y - 0.5).ceil().max(0.0) as i32;
    let center_x1 = ((layout.x + layout.width) - 0.5).ceil().min(width as f32) as i32;
    let center_y1 = ((layout.y + layout.height) - 0.5).ceil().min(height as f32) as i32;
    let x0 = x0.max(center_x0);
    let y0 = y0.max(center_y0);
    let x1 = x1.min(center_x1);
    let y1 = y1.min(center_y1);
    (x0 < x1 && y0 < y1).then_some((x0, y0, x1, y1))
}

pub(crate) fn center_pixel_bounds(
    layout: LayoutBox,
    width: usize,
    height: usize,
) -> Option<(i32, i32, i32, i32)> {
    let x0 = (layout.x - 0.5).ceil().max(0.0) as i32;
    let y0 = (layout.y - 0.5).ceil().max(0.0) as i32;
    let x1 = ((layout.x + layout.width) - 0.5).ceil().min(width as f32) as i32;
    let y1 = ((layout.y + layout.height) - 0.5).ceil().min(height as f32) as i32;
    (x0 < x1 && y0 < y1).then_some((x0, y0, x1, y1))
}

pub(crate) fn clip_pixel_bounds(
    clip: ClipRect,
    width: usize,
    height: usize,
) -> Option<(i32, i32, i32, i32)> {
    let clip = clip.intersect(ClipRect::full(width as f32, height as f32))?;
    let x0 = clip.x0.floor().max(0.0) as i32;
    let y0 = clip.y0.floor().max(0.0) as i32;
    let x1 = clip.x1.ceil().min(width as f32) as i32;
    let y1 = clip.y1.ceil().min(height as f32) as i32;
    (x0 < x1 && y0 < y1).then_some((x0, y0, x1, y1))
}

pub(crate) fn snap_clip_to_pixel_grid(
    clip: ClipRect,
    width: usize,
    height: usize,
) -> Option<ClipRect> {
    let (x0, y0, x1, y1) = clip_pixel_bounds(clip, width, height)?;
    Some(ClipRect {
        x0: x0 as f32,
        y0: y0 as f32,
        x1: x1 as f32,
        y1: y1 as f32,
    })
}

pub(crate) fn non_empty_layout_clip(layout: LayoutBox) -> Option<ClipRect> {
    let clip = layout_clip(layout);
    (!clip.is_empty()).then_some(clip)
}

pub(crate) fn union_optional_bounds(
    left: Option<ClipRect>,
    right: Option<ClipRect>,
) -> Option<ClipRect> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.union(right)),
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => None,
    }
}

pub(crate) fn fill_opaque_span_rows(
    buffer: &mut [u32],
    width: usize,
    x0: i32,
    x1: i32,
    y0: i32,
    y1: i32,
    packed: u32,
) {
    if x0 >= x1 || y0 >= y1 {
        return;
    }

    let rows = current_render_buffer_rows();
    for y in y0 as usize..y1 as usize {
        if y < rows.start || y >= rows.end {
            continue;
        }
        let row_start = (y - rows.start) * width;
        buffer[row_start + x0 as usize..row_start + x1 as usize].fill(packed);
    }
}

fn blend_prepared_span_row(
    buffer: &mut [u32],
    width: usize,
    height: usize,
    x0: i32,
    x1: i32,
    y: i32,
    color: PreparedBlendColor,
) {
    if x0 >= x1 {
        return;
    }

    if color.linear.a >= 1.0 {
        fill_opaque_span_rows(buffer, width, x0, x1, y, y + 1, color.packed);
        return;
    }

    for x in x0..x1 {
        blend_prepared_pixel(buffer, width, height, x, y, color);
    }
}

pub(crate) fn rounded_rect_row_span(
    layout: LayoutBox,
    radius: CornerRadius,
    y: i32,
    x0: i32,
    x1: i32,
) -> Option<(i32, i32)> {
    if x0 >= x1 {
        return None;
    }

    let py = y as f32 + 0.5;
    if py < layout.y || py >= layout.y + layout.height {
        return None;
    }

    let clamped_radius = clamp_corner_radius(radius, layout.width, layout.height);
    if clamped_radius.top_left == 0.0
        && clamped_radius.top_right == 0.0
        && clamped_radius.bottom_right == 0.0
        && clamped_radius.bottom_left == 0.0
    {
        return Some((x0, x1));
    }

    let mut span_x0 = x0;
    while span_x0 < x1
        && !point_in_rounded_rect_with_radius(span_x0 as f32 + 0.5, py, layout, clamped_radius)
    {
        span_x0 += 1;
    }

    let mut span_x1 = x1;
    while span_x1 > span_x0
        && !point_in_rounded_rect_with_radius(span_x1 as f32 - 0.5, py, layout, clamped_radius)
    {
        span_x1 -= 1;
    }

    (span_x0 < span_x1).then_some((span_x0, span_x1))
}

pub(crate) fn point_in_rounded_rect(
    x: f32,
    y: f32,
    layout: LayoutBox,
    radius: CornerRadius,
) -> bool {
    point_in_rounded_rect_with_radius(
        x,
        y,
        layout,
        clamp_corner_radius(radius, layout.width, layout.height),
    )
}

fn point_in_rounded_rect_with_radius(
    x: f32,
    y: f32,
    layout: LayoutBox,
    radius: CornerRadius,
) -> bool {
    if !layout_contains(layout, x, y) {
        return false;
    }

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

pub(crate) fn corner_radius_is_zero(layout: LayoutBox, radius: CornerRadius) -> bool {
    let radius = clamp_corner_radius(radius, layout.width, layout.height);
    radius.top_left == 0.0
        && radius.top_right == 0.0
        && radius.bottom_right == 0.0
        && radius.bottom_left == 0.0
}

pub(crate) fn inset_layout(layout: LayoutBox, insets: Insets) -> LayoutBox {
    let width = (layout.width - insets.left - insets.right).max(0.0);
    let height = (layout.height - insets.top - insets.bottom).max(0.0);
    LayoutBox::new(layout.x + insets.left, layout.y + insets.top, width, height)
}

pub(crate) fn inset_corner_radius(
    layout: LayoutBox,
    radius: CornerRadius,
    insets: Insets,
) -> CornerRadius {
    let radius = clamp_corner_radius(radius, layout.width, layout.height);
    CornerRadius {
        top_left: (radius.top_left - insets.top.max(insets.left)).max(0.0),
        top_right: (radius.top_right - insets.top.max(insets.right)).max(0.0),
        bottom_right: (radius.bottom_right - insets.bottom.max(insets.right)).max(0.0),
        bottom_left: (radius.bottom_left - insets.bottom.max(insets.left)).max(0.0),
    }
}

pub(crate) fn expand_layout(layout: LayoutBox, amount: f32) -> LayoutBox {
    let width = (layout.width + amount * 2.0).max(0.0);
    let height = (layout.height + amount * 2.0).max(0.0);
    LayoutBox::new(layout.x - amount, layout.y - amount, width, height)
}

pub(crate) fn offset_layout(layout: LayoutBox, x: f32, y: f32) -> LayoutBox {
    LayoutBox::new(layout.x + x, layout.y + y, layout.width, layout.height)
}

pub(crate) fn expand_corner_radius(
    layout: LayoutBox,
    radius: CornerRadius,
    amount: f32,
) -> CornerRadius {
    let radius = clamp_corner_radius(radius, layout.width, layout.height);
    CornerRadius {
        top_left: (radius.top_left + amount).max(0.0),
        top_right: (radius.top_right + amount).max(0.0),
        bottom_right: (radius.bottom_right + amount).max(0.0),
        bottom_left: (radius.bottom_left + amount).max(0.0),
    }
}

fn layout_contains(layout: LayoutBox, x: f32, y: f32) -> bool {
    x >= layout.x && x < layout.x + layout.width && y >= layout.y && y < layout.y + layout.height
}

fn clamp_corner_radius(radius: CornerRadius, width: f32, height: f32) -> CornerRadius {
    let max_radius = 0.5 * width.min(height).max(0.0);
    CornerRadius {
        top_left: resolve_corner_radius_value(radius.top_left, max_radius),
        top_right: resolve_corner_radius_value(radius.top_right, max_radius),
        bottom_right: resolve_corner_radius_value(radius.bottom_right, max_radius),
        bottom_left: resolve_corner_radius_value(radius.bottom_left, max_radius),
    }
}

fn resolve_corner_radius_value(value: f32, max_radius: f32) -> f32 {
    if value < 0.0 {
        (-value * max_radius).min(max_radius).max(0.0)
    } else {
        value.min(max_radius).max(0.0)
    }
}

fn point_in_corner(x: f32, y: f32, center_x: f32, center_y: f32, radius: f32) -> bool {
    if radius <= 0.0 {
        return true;
    }

    let dx = x - center_x;
    let dy = y - center_y;
    (dx * dx) + (dy * dy) <= radius * radius
}

#[cfg(test)]
mod tests {
    use cssimpler_core::{Color, CornerRadius, LayoutBox};

    use super::{draw_rounded_ring, point_in_rounded_rect, rounded_rect_row_span};
    use crate::{ClipRect, pack_rgb};

    #[test]
    fn rounded_rect_row_span_matches_point_sampling() {
        let layout = LayoutBox::new(2.25, 1.75, 11.5, 9.5);
        let radius = CornerRadius {
            top_left: 3.5,
            top_right: 2.0,
            bottom_right: 4.0,
            bottom_left: 1.5,
        };

        for y in 0..16 {
            let span = rounded_rect_row_span(layout, radius, y, 0, 16);
            let sampled = (0..16)
                .filter(|&x| point_in_rounded_rect(x as f32 + 0.5, y as f32 + 0.5, layout, radius))
                .collect::<Vec<_>>();

            match span {
                Some((x0, x1)) => {
                    assert_eq!(sampled, (x0..x1).collect::<Vec<_>>());
                }
                None => assert!(sampled.is_empty()),
            }
        }
    }

    #[test]
    fn rounded_ring_span_batches_match_point_sampling() {
        let outer_layout = LayoutBox::new(1.0, 1.0, 10.0, 10.0);
        let outer_radius = CornerRadius::all(4.0);
        let inner_layout = LayoutBox::new(3.0, 3.0, 6.0, 6.0);
        let inner_radius = CornerRadius::all(2.0);
        let mut buffer = vec![0_u32; 12 * 12];

        draw_rounded_ring(
            &mut buffer,
            12,
            12,
            outer_layout,
            outer_radius,
            Some((inner_layout, inner_radius)),
            Color::rgb(40, 120, 220),
            ClipRect::full(12.0, 12.0),
        );

        let accent = pack_rgb(Color::rgb(40, 120, 220));
        for y in 0..12 {
            for x in 0..12 {
                let expected = point_in_rounded_rect(
                    x as f32 + 0.5,
                    y as f32 + 0.5,
                    outer_layout,
                    outer_radius,
                ) && !point_in_rounded_rect(
                    x as f32 + 0.5,
                    y as f32 + 0.5,
                    inner_layout,
                    inner_radius,
                );
                assert_eq!(buffer[y * 12 + x] == accent, expected);
            }
        }
    }
}
