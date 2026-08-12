use cssimpler_core::{Color, CornerRadius, Insets, LayoutBox};

use super::{
    ClipRect, PreparedBlendColor, blend_prepared_pixel_with_coverage,
    blend_prepared_pixel_with_coverage_at_index,
    blend_prepared_pixel_with_coverage_at_index_opaque_target, current_render_buffer_rows,
    fill_current_alpha_span, pack_rgb, render_alpha_target_active,
    transform::{AffineTransform, ClipState},
};

const AXIS_ALIGNED_COARSE_SAMPLE_MIN: f32 = 0.25;
const AXIS_ALIGNED_COARSE_SAMPLE_MAX: f32 = 0.75;
const AXIS_ALIGNED_EDGE_COARSE_COVERAGE_SAMPLES: [(f32, f32); 4] = [
    (
        AXIS_ALIGNED_COARSE_SAMPLE_MIN,
        AXIS_ALIGNED_COARSE_SAMPLE_MIN,
    ),
    (
        AXIS_ALIGNED_COARSE_SAMPLE_MAX,
        AXIS_ALIGNED_COARSE_SAMPLE_MIN,
    ),
    (
        AXIS_ALIGNED_COARSE_SAMPLE_MIN,
        AXIS_ALIGNED_COARSE_SAMPLE_MAX,
    ),
    (
        AXIS_ALIGNED_COARSE_SAMPLE_MAX,
        AXIS_ALIGNED_COARSE_SAMPLE_MAX,
    ),
];
const AXIS_ALIGNED_EDGE_FINE_COVERAGE_SAMPLES: [(f32, f32); 16] = [
    (0.125, 0.125),
    (0.375, 0.125),
    (0.625, 0.125),
    (0.875, 0.125),
    (0.125, 0.375),
    (0.375, 0.375),
    (0.625, 0.375),
    (0.875, 0.375),
    (0.125, 0.625),
    (0.375, 0.625),
    (0.625, 0.625),
    (0.875, 0.625),
    (0.125, 0.875),
    (0.375, 0.875),
    (0.625, 0.875),
    (0.875, 0.875),
];
const TRANSFORMED_EDGE_COARSE_COVERAGE_SAMPLES: [(f32, f32); 4] =
    [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)];
const TRANSFORMED_EDGE_FINE_COVERAGE_SAMPLES: [(f32, f32); 16] = [
    (0.125, 0.125),
    (0.375, 0.125),
    (0.625, 0.125),
    (0.875, 0.125),
    (0.125, 0.375),
    (0.375, 0.375),
    (0.625, 0.375),
    (0.875, 0.375),
    (0.125, 0.625),
    (0.375, 0.625),
    (0.625, 0.625),
    (0.875, 0.625),
    (0.125, 0.875),
    (0.375, 0.875),
    (0.625, 0.875),
    (0.875, 0.875),
];
const DASH_LENGTH_MULTIPLIER: f32 = 3.0;
const DASH_GAP_LENGTH_MULTIPLIER: f32 = 2.0;
const DASH_MIN_LENGTH: f32 = 2.0;
const DASH_MIN_GAP: f32 = 1.0;

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
    let rows = current_render_buffer_rows();
    let draw_y0 = y0.max(rows.start.min(height) as i32);
    let draw_y1 = y1.min(rows.end.min(height) as i32);
    let has_alpha_target = render_alpha_target_active();
    for y in draw_y0..draw_y1 {
        let row_start = (y as usize - rows.start) * width;
        let (full_x0, full_x1) =
            rounded_rect_full_coverage_row_span(layout, radius, clip, y, x0, x1)
                .unwrap_or((x0, x0));
        for x in x0..full_x0 {
            let coverage = rounded_rect_coverage(layout, radius, clip, x, y);
            if coverage == 0 {
                continue;
            }
            let index = row_start + x as usize;
            if has_alpha_target {
                blend_prepared_pixel_with_coverage_at_index(
                    buffer,
                    index,
                    prepared_color,
                    color.a,
                    coverage,
                );
            } else {
                blend_prepared_pixel_with_coverage_at_index_opaque_target(
                    buffer,
                    index,
                    prepared_color,
                    color.a,
                    coverage,
                );
            }
        }
        for x in full_x0..full_x1 {
            let index = row_start + x as usize;
            if has_alpha_target {
                blend_prepared_pixel_with_coverage_at_index(
                    buffer,
                    index,
                    prepared_color,
                    color.a,
                    u8::MAX,
                );
            } else {
                blend_prepared_pixel_with_coverage_at_index_opaque_target(
                    buffer,
                    index,
                    prepared_color,
                    color.a,
                    u8::MAX,
                );
            }
        }
        for x in full_x1..x1 {
            let coverage = rounded_rect_coverage(layout, radius, clip, x, y);
            if coverage == 0 {
                continue;
            }
            let index = row_start + x as usize;
            if has_alpha_target {
                blend_prepared_pixel_with_coverage_at_index(
                    buffer,
                    index,
                    prepared_color,
                    color.a,
                    coverage,
                );
            } else {
                blend_prepared_pixel_with_coverage_at_index_opaque_target(
                    buffer,
                    index,
                    prepared_color,
                    color.a,
                    coverage,
                );
            }
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
        for x in x0..x1 {
            let coverage = rounded_ring_coverage(outer_layout, outer_radius, inner, clip, x, y);
            if coverage == 0 {
                continue;
            }
            blend_prepared_pixel_with_coverage(
                buffer,
                width,
                height,
                x,
                y,
                prepared_color,
                color.a,
                coverage,
            );
        }
    }
}

pub(crate) fn draw_dashed_rounded_ring(
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
        for x in x0..x1 {
            let coverage =
                dashed_rounded_ring_coverage(outer_layout, outer_radius, inner, clip, x, y);
            if coverage == 0 {
                continue;
            }
            blend_prepared_pixel_with_coverage(
                buffer,
                width,
                height,
                x,
                y,
                prepared_color,
                color.a,
                coverage,
            );
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

pub(crate) fn transformed_rounded_rect_coverage(
    layout: LayoutBox,
    radius: CornerRadius,
    inverse: AffineTransform,
    clip_state: &ClipState,
    x: i32,
    y: i32,
) -> u8 {
    transformed_shape_coverage(inverse, clip_state, x, y, |source_x, source_y| {
        point_in_rounded_rect(source_x, source_y, layout, radius)
    })
}

pub(crate) fn rounded_rect_coverage(
    layout: LayoutBox,
    radius: CornerRadius,
    clip: ClipRect,
    x: i32,
    y: i32,
) -> u8 {
    axis_aligned_shape_coverage(clip, x, y, |sample_x, sample_y| {
        point_in_rounded_rect(sample_x, sample_y, layout, radius)
    })
}

pub(crate) fn rounded_rect_full_coverage_row_span(
    layout: LayoutBox,
    radius: CornerRadius,
    clip: ClipRect,
    y: i32,
    x0: i32,
    x1: i32,
) -> Option<(i32, i32)> {
    if x0 >= x1 {
        return None;
    }

    let layout_right = layout.x + layout.width;
    let layout_bottom = layout.y + layout.height;
    let sample_y0 = y as f32 + AXIS_ALIGNED_COARSE_SAMPLE_MIN;
    let sample_y1 = y as f32 + AXIS_ALIGNED_COARSE_SAMPLE_MAX;
    let vertical_start = layout.y.max(clip.y0);
    let vertical_end = layout_bottom.min(clip.y1);
    if !(sample_y0 >= vertical_start && sample_y1 < vertical_end) {
        return None;
    }

    let radius = clamp_corner_radius(radius, layout.width, layout.height);
    let left_guard = if sample_y0 < layout.y + radius.top_left {
        radius.top_left
    } else {
        0.0
    }
    .max(if sample_y1 > layout_bottom - radius.bottom_left {
        radius.bottom_left
    } else {
        0.0
    });
    let right_guard = if sample_y0 < layout.y + radius.top_right {
        radius.top_right
    } else {
        0.0
    }
    .max(if sample_y1 > layout_bottom - radius.bottom_right {
        radius.bottom_right
    } else {
        0.0
    });

    // This deliberately excludes each active corner's entire horizontal radius band.
    // It may omit some fully covered circle pixels, but every returned pixel is guaranteed
    // to pass all four coarse samples and therefore has exactly u8::MAX coverage.
    let safe_start = clip.x0.max(layout.x + left_guard);
    let safe_end = clip.x1.min(layout_right - right_guard);
    let span_x0 = x0.max((safe_start - AXIS_ALIGNED_COARSE_SAMPLE_MIN).ceil() as i32);
    let span_x1 = x1.min((safe_end - AXIS_ALIGNED_COARSE_SAMPLE_MAX).ceil() as i32);
    (span_x0 < span_x1).then_some((span_x0, span_x1))
}

pub(crate) fn transformed_rounded_ring_coverage(
    outer_layout: LayoutBox,
    outer_radius: CornerRadius,
    inner: Option<(LayoutBox, CornerRadius)>,
    inverse: AffineTransform,
    clip_state: &ClipState,
    x: i32,
    y: i32,
) -> u8 {
    transformed_shape_coverage(inverse, clip_state, x, y, |source_x, source_y| {
        point_in_rounded_rect(source_x, source_y, outer_layout, outer_radius)
            && inner.is_none_or(|(inner_layout, inner_radius)| {
                !point_in_rounded_rect(source_x, source_y, inner_layout, inner_radius)
            })
    })
}

pub(crate) fn rounded_ring_coverage(
    outer_layout: LayoutBox,
    outer_radius: CornerRadius,
    inner: Option<(LayoutBox, CornerRadius)>,
    clip: ClipRect,
    x: i32,
    y: i32,
) -> u8 {
    axis_aligned_shape_coverage(clip, x, y, |sample_x, sample_y| {
        point_in_rounded_rect(sample_x, sample_y, outer_layout, outer_radius)
            && inner.is_none_or(|(inner_layout, inner_radius)| {
                !point_in_rounded_rect(sample_x, sample_y, inner_layout, inner_radius)
            })
    })
}

pub(crate) fn transformed_dashed_rounded_ring_coverage(
    outer_layout: LayoutBox,
    outer_radius: CornerRadius,
    inner: Option<(LayoutBox, CornerRadius)>,
    inverse: AffineTransform,
    clip_state: &ClipState,
    x: i32,
    y: i32,
) -> u8 {
    transformed_shape_coverage(inverse, clip_state, x, y, |source_x, source_y| {
        dashed_ring_contains_point(source_x, source_y, outer_layout, outer_radius, inner)
    })
}

pub(crate) fn dashed_rounded_ring_coverage(
    outer_layout: LayoutBox,
    outer_radius: CornerRadius,
    inner: Option<(LayoutBox, CornerRadius)>,
    clip: ClipRect,
    x: i32,
    y: i32,
) -> u8 {
    axis_aligned_shape_coverage(clip, x, y, |sample_x, sample_y| {
        dashed_ring_contains_point(sample_x, sample_y, outer_layout, outer_radius, inner)
    })
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
        let start = row_start + x0 as usize;
        let end = row_start + x1 as usize;
        buffer[start..end].fill(packed);
        fill_current_alpha_span(start, end - start, u8::MAX);
    }
}

fn transformed_shape_coverage(
    inverse: AffineTransform,
    clip_state: &ClipState,
    x: i32,
    y: i32,
    contains: impl Fn(f32, f32) -> bool,
) -> u8 {
    let coarse_hits = transformed_shape_sample_hits(
        TRANSFORMED_EDGE_COARSE_COVERAGE_SAMPLES,
        inverse,
        clip_state,
        x,
        y,
        &contains,
    );
    if coarse_hits == 0 {
        return 0;
    }
    if coarse_hits == TRANSFORMED_EDGE_COARSE_COVERAGE_SAMPLES.len() as u8 {
        return u8::MAX;
    }

    let fine_hits = transformed_shape_sample_hits(
        TRANSFORMED_EDGE_FINE_COVERAGE_SAMPLES,
        inverse,
        clip_state,
        x,
        y,
        contains,
    );
    coverage_from_sample_hits(
        fine_hits,
        TRANSFORMED_EDGE_FINE_COVERAGE_SAMPLES.len() as u8,
    )
}

fn axis_aligned_shape_coverage(
    clip: ClipRect,
    x: i32,
    y: i32,
    contains: impl Fn(f32, f32) -> bool,
) -> u8 {
    let coarse_hits = axis_aligned_shape_sample_hits(
        AXIS_ALIGNED_EDGE_COARSE_COVERAGE_SAMPLES,
        clip,
        x,
        y,
        &contains,
    );
    if coarse_hits == 0 {
        return 0;
    }
    if coarse_hits == AXIS_ALIGNED_EDGE_COARSE_COVERAGE_SAMPLES.len() as u8 {
        return u8::MAX;
    }

    let fine_hits = axis_aligned_shape_sample_hits(
        AXIS_ALIGNED_EDGE_FINE_COVERAGE_SAMPLES,
        clip,
        x,
        y,
        contains,
    );
    coverage_from_sample_hits(
        fine_hits,
        AXIS_ALIGNED_EDGE_FINE_COVERAGE_SAMPLES.len() as u8,
    )
}

fn axis_aligned_shape_sample_hits<const N: usize>(
    samples: [(f32, f32); N],
    clip: ClipRect,
    x: i32,
    y: i32,
    contains: impl Fn(f32, f32) -> bool,
) -> u8 {
    let mut hits = 0_u8;
    for (sample_x, sample_y) in samples {
        let screen_x = x as f32 + sample_x;
        let screen_y = y as f32 + sample_y;
        if !clip.contains(screen_x, screen_y) {
            continue;
        }
        if contains(screen_x, screen_y) {
            hits += 1;
        }
    }
    hits
}

fn transformed_shape_sample_hits<const N: usize>(
    samples: [(f32, f32); N],
    inverse: AffineTransform,
    clip_state: &ClipState,
    x: i32,
    y: i32,
    contains: impl Fn(f32, f32) -> bool,
) -> u8 {
    let mut hits = 0_u8;
    for (sample_x, sample_y) in samples {
        let screen_x = x as f32 + sample_x;
        let screen_y = y as f32 + sample_y;
        if !clip_state.contains(screen_x, screen_y) {
            continue;
        }

        let (source_x, source_y) = inverse.transform_point(screen_x, screen_y);
        if !source_x.is_finite() || !source_y.is_finite() {
            continue;
        }
        if contains(source_x, source_y) {
            hits += 1;
        }
    }

    hits
}

fn coverage_from_sample_hits(hits: u8, total_samples: u8) -> u8 {
    match hits {
        0 => 0,
        hits if hits >= total_samples => u8::MAX,
        hits => ((hits as f32 / total_samples as f32) * 255.0).round() as u8,
    }
}

#[cfg_attr(not(test), allow(dead_code))]
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

fn dashed_ring_contains_point(
    x: f32,
    y: f32,
    outer_layout: LayoutBox,
    outer_radius: CornerRadius,
    inner: Option<(LayoutBox, CornerRadius)>,
) -> bool {
    point_in_rounded_rect(x, y, outer_layout, outer_radius)
        && inner.is_none_or(|(inner_layout, inner_radius)| {
            !point_in_rounded_rect(x, y, inner_layout, inner_radius)
        })
        && point_in_dashed_segment(x, y, outer_layout, inner)
}

fn point_in_dashed_segment(
    x: f32,
    y: f32,
    outer_layout: LayoutBox,
    inner: Option<(LayoutBox, CornerRadius)>,
) -> bool {
    let width = outer_layout.width.max(0.0);
    let height = outer_layout.height.max(0.0);
    let perimeter = (2.0 * (width + height)).max(1.0);
    if perimeter <= f32::EPSILON {
        return true;
    }

    let (dash_length, cycle_length) = dash_cycle_lengths(outer_layout, inner);
    if cycle_length <= dash_length {
        return true;
    }

    let perimeter_position = perimeter_position_from_point(x, y, outer_layout);
    let cycle_position = perimeter_position.rem_euclid(cycle_length);
    cycle_position < dash_length
}

fn dash_cycle_lengths(
    outer_layout: LayoutBox,
    inner: Option<(LayoutBox, CornerRadius)>,
) -> (f32, f32) {
    let average_border_width = inner
        .map(|(inner_layout, _)| {
            let horizontal = (outer_layout.width - inner_layout.width).max(0.0) * 0.5;
            let vertical = (outer_layout.height - inner_layout.height).max(0.0) * 0.5;
            ((horizontal + vertical) * 0.5).max(1.0)
        })
        .unwrap_or(1.0);
    let dash_length = (average_border_width * DASH_LENGTH_MULTIPLIER).max(DASH_MIN_LENGTH);
    let dash_gap = (average_border_width * DASH_GAP_LENGTH_MULTIPLIER).max(DASH_MIN_GAP);
    (dash_length, dash_length + dash_gap)
}

fn perimeter_position_from_point(x: f32, y: f32, layout: LayoutBox) -> f32 {
    let left = layout.x;
    let top = layout.y;
    let right = layout.x + layout.width.max(0.0);
    let bottom = layout.y + layout.height.max(0.0);
    let clamped_x = x.clamp(left, right);
    let clamped_y = y.clamp(top, bottom);
    let dist_top = (clamped_y - top).abs();
    let dist_right = (right - clamped_x).abs();
    let dist_bottom = (bottom - clamped_y).abs();
    let dist_left = (clamped_x - left).abs();
    let width = (right - left).max(0.0);
    let height = (bottom - top).max(0.0);

    if dist_top <= dist_right && dist_top <= dist_bottom && dist_top <= dist_left {
        clamped_x - left
    } else if dist_right <= dist_bottom && dist_right <= dist_left {
        width + (clamped_y - top)
    } else if dist_bottom <= dist_left {
        width + height + (right - clamped_x)
    } else {
        (2.0 * width) + height + (bottom - clamped_y)
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

    use super::{
        draw_dashed_rounded_ring, draw_rounded_ring, point_in_rounded_rect, rounded_rect_coverage,
        rounded_rect_full_coverage_row_span, rounded_rect_row_span, rounded_ring_coverage,
        transformed_rounded_rect_coverage, transformed_rounded_ring_coverage,
    };
    use crate::transform::{AffineTransform, ClipState};
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

    fn assert_full_coverage_span_preserves_sampled_coverage(
        layout: LayoutBox,
        radius: CornerRadius,
        clip: ClipRect,
        x0: i32,
        y0: i32,
        x1: i32,
        y1: i32,
    ) {
        for y in y0..y1 {
            let span = rounded_rect_full_coverage_row_span(layout, radius, clip, y, x0, x1);
            if let Some((span_x0, span_x1)) = span {
                assert!(
                    x0 <= span_x0 && span_x0 < span_x1 && span_x1 <= x1,
                    "invalid span {span:?} at y={y} for {layout:?}, {radius:?}, {clip:?}"
                );
            }

            for x in x0..x1 {
                let sampled = rounded_rect_coverage(layout, radius, clip, x, y);
                let covered_by_span =
                    span.is_some_and(|(span_x0, span_x1)| x >= span_x0 && x < span_x1);
                if covered_by_span {
                    assert_eq!(
                        sampled,
                        u8::MAX,
                        "non-full pixel ({x}, {y}) included for {layout:?}, {radius:?}, {clip:?}"
                    );
                }
                let optimized = if covered_by_span { u8::MAX } else { sampled };
                assert_eq!(
                    optimized, sampled,
                    "coverage changed at ({x}, {y}) for {layout:?}, {radius:?}, {clip:?}"
                );
            }
        }
    }

    fn next_test_unit(state: &mut u64) -> f32 {
        *state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((*state >> 40) as u32) as f32 / ((1_u32 << 24) - 1) as f32
    }

    #[test]
    fn rounded_rect_full_coverage_span_preserves_sampled_coverage() {
        let cases = [
            (
                LayoutBox::new(0.35, 0.6, 15.2, 11.4),
                CornerRadius::all(3.75),
                ClipRect::full(20.0, 16.0),
            ),
            (
                LayoutBox::new(1.25, 1.5, 16.0, 12.0),
                CornerRadius {
                    top_left: 6.0,
                    top_right: 2.0,
                    bottom_right: 5.0,
                    bottom_left: 1.0,
                },
                ClipRect {
                    x0: 2.4,
                    y0: 0.75,
                    x1: 14.6,
                    y1: 13.25,
                },
            ),
            (
                LayoutBox::new(0.25, 0.25, 18.5, 13.5),
                CornerRadius::ZERO,
                ClipRect {
                    x0: 2.25,
                    y0: 1.25,
                    x1: 16.75,
                    y1: 12.75,
                },
            ),
            (
                LayoutBox::new(1.1, 0.9, 15.75, 15.75),
                CornerRadius::all(20.0),
                ClipRect::full(20.0, 20.0),
            ),
            (
                LayoutBox::new(0.4, 3.35, 18.2, 0.45),
                CornerRadius::all(4.0),
                ClipRect::full(20.0, 20.0),
            ),
            (
                LayoutBox::new(7.35, 0.4, 0.55, 18.2),
                CornerRadius::all(4.0),
                ClipRect::full(20.0, 20.0),
            ),
            (
                LayoutBox::new(0.75, 1.25, 17.5, 13.0),
                CornerRadius {
                    top_left: -0.25,
                    top_right: -0.75,
                    bottom_right: -1.0,
                    bottom_left: -0.5,
                },
                ClipRect {
                    x0: 1.25,
                    y0: 1.75,
                    x1: 17.75,
                    y1: 13.75,
                },
            ),
        ];

        for (layout, radius, clip) in cases {
            assert_full_coverage_span_preserves_sampled_coverage(
                layout, radius, clip, 0, 0, 20, 20,
            );
        }

        let mut state = 0x9e37_79b9_7f4a_7c15_u64;
        for _ in 0..512 {
            let layout = LayoutBox::new(
                -2.0 + next_test_unit(&mut state) * 8.0,
                -2.0 + next_test_unit(&mut state) * 8.0,
                0.05 + next_test_unit(&mut state) * 26.0,
                0.05 + next_test_unit(&mut state) * 26.0,
            );
            let mut radius_value = || {
                let value = next_test_unit(&mut state);
                if next_test_unit(&mut state) < 0.25 {
                    -value
                } else {
                    value * 18.0
                }
            };
            let radius = CornerRadius {
                top_left: radius_value(),
                top_right: radius_value(),
                bottom_right: radius_value(),
                bottom_left: radius_value(),
            };
            let clip_x0 = -1.0 + next_test_unit(&mut state) * 10.0;
            let clip_y0 = -1.0 + next_test_unit(&mut state) * 10.0;
            let clip = ClipRect {
                x0: clip_x0,
                y0: clip_y0,
                x1: clip_x0 + 0.05 + next_test_unit(&mut state) * 24.0,
                y1: clip_y0 + 0.05 + next_test_unit(&mut state) * 24.0,
            };
            assert_full_coverage_span_preserves_sampled_coverage(
                layout, radius, clip, 0, 0, 32, 32,
            );
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
                let coverage = rounded_ring_coverage(
                    outer_layout,
                    outer_radius,
                    Some((inner_layout, inner_radius)),
                    ClipRect::full(12.0, 12.0),
                    x as i32,
                    y as i32,
                );
                let pixel = buffer[y * 12 + x];
                if coverage == 0 {
                    assert_eq!(pixel, 0);
                } else if coverage == u8::MAX {
                    assert_eq!(pixel, accent);
                } else {
                    assert_ne!(pixel, 0);
                    assert_ne!(pixel, accent);
                }
            }
        }
    }

    #[test]
    fn dashed_ring_includes_visible_gaps_on_the_top_edge() {
        let outer_layout = LayoutBox::new(1.0, 1.0, 10.0, 10.0);
        let inner_layout = LayoutBox::new(3.0, 3.0, 6.0, 6.0);
        let color = Color::rgb(40, 120, 220);
        let mut buffer = vec![pack_rgb(Color::WHITE); 12 * 12];

        draw_dashed_rounded_ring(
            &mut buffer,
            12,
            12,
            outer_layout,
            CornerRadius::default(),
            Some((inner_layout, CornerRadius::default())),
            color,
            ClipRect::full(12.0, 12.0),
        );

        let accent = pack_rgb(color);
        let white = pack_rgb(Color::WHITE);
        let top_row = &buffer[12 + 1..12 + 11];
        assert!(
            top_row.contains(&accent),
            "dashed borders should still paint visible border segments"
        );
        assert!(
            top_row.contains(&white),
            "dashed borders should leave visible gaps between segments"
        );
    }

    #[test]
    fn transformed_rounded_rect_coverage_reports_partial_edge_alpha() {
        let coverage = transformed_rounded_rect_coverage(
            LayoutBox::new(0.6, 0.0, 1.0, 1.0),
            CornerRadius::default(),
            AffineTransform::IDENTITY,
            &ClipState::new(ClipRect::full(2.0, 2.0)),
            0,
            0,
        );

        assert_eq!(coverage, 128);
    }

    #[test]
    fn transformed_rounded_rect_coverage_uses_finer_edge_steps_when_needed() {
        let coverage = transformed_rounded_rect_coverage(
            LayoutBox::new(0.3, 0.0, 1.0, 1.0),
            CornerRadius::default(),
            AffineTransform::IDENTITY,
            &ClipState::new(ClipRect::full(2.0, 2.0)),
            0,
            0,
        );

        assert_eq!(coverage, 191);
    }

    #[test]
    fn transformed_rounded_ring_coverage_excludes_inner_samples() {
        let coverage = transformed_rounded_ring_coverage(
            LayoutBox::new(0.0, 0.0, 2.0, 2.0),
            CornerRadius::default(),
            Some((LayoutBox::new(0.5, 0.5, 1.0, 1.0), CornerRadius::default())),
            AffineTransform::IDENTITY,
            &ClipState::new(ClipRect::full(2.0, 2.0)),
            0,
            0,
        );

        assert_eq!(coverage, 191);
    }
}
