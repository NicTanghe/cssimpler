use cssimpler_core::{LinearRgba, SvgBounds, SvgPathGeometry, SvgPoint, SvgScene, SvgViewBox};

use super::{
    ClipRect, blend_linear_over, clip_pixel_bounds, current_render_buffer_rows,
    transform::{AffineTransform, ClipState, transform_clip_rect},
};

use cssimpler_core::LayoutBox;

const SVG_COVERAGE_SAMPLES: [(f32, f32); 4] = [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)];

pub(crate) fn draw_svg_scene(
    buffer: &mut [u32],
    width: usize,
    height: usize,
    layout: LayoutBox,
    scene: &SvgScene,
    clip: ClipRect,
) {
    let Some(matrix) = svg_view_box_matrix(layout, scene.view_box) else {
        return;
    };
    draw_svg_scene_with_matrix(
        buffer,
        width,
        height,
        scene,
        matrix,
        &ClipState::new(clip),
    );
}

pub(crate) fn draw_svg_scene_transformed(
    buffer: &mut [u32],
    width: usize,
    height: usize,
    layout: LayoutBox,
    scene: &SvgScene,
    matrix: AffineTransform,
    clip_state: &ClipState,
) {
    let Some(view_matrix) = svg_view_box_matrix(layout, scene.view_box) else {
        return;
    };
    draw_svg_scene_with_matrix(
        buffer,
        width,
        height,
        scene,
        matrix.multiply(view_matrix),
        clip_state,
    );
}

fn draw_svg_scene_with_matrix(
    buffer: &mut [u32],
    width: usize,
    height: usize,
    scene: &SvgScene,
    matrix: AffineTransform,
    clip_state: &ClipState,
) {
    let Some(inverse) = matrix.invert() else {
        return;
    };
    let rows = current_render_buffer_rows();
    let row_start = rows.start.min(height) as i32;
    let row_end = rows.end.min(height) as i32;

    for path in &scene.paths {
        let Some(source_bounds) = path.bounds() else {
            continue;
        };
        let Some(bounds) =
            transform_svg_bounds(source_bounds, matrix).and_then(|bounds| bounds.intersect(clip_state.coarse))
        else {
            continue;
        };
        let Some((x0, y0, x1, y1)) = clip_pixel_bounds(bounds, width, height) else {
            continue;
        };

        let fill = path.paint.fill.map(|color| color.to_linear_rgba());
        let stroke = if path.paint.stroke_width > f32::EPSILON {
            path.paint.stroke.map(|color| color.to_linear_rgba())
        } else {
            None
        };
        if fill.is_none() && stroke.is_none() {
            continue;
        }

        for y in y0.max(row_start)..y1.min(row_end) {
            let local_row_start = (y as usize - rows.start) * width;
            for x in x0..x1 {
                let mut fill_hits = 0_u8;
                let mut stroke_hits = 0_u8;
                for (sample_x, sample_y) in SVG_COVERAGE_SAMPLES {
                    let screen_x = x as f32 + sample_x;
                    let screen_y = y as f32 + sample_y;
                    if !clip_state.contains(screen_x, screen_y) {
                        continue;
                    }

                    let (source_x, source_y) = inverse.transform_point(screen_x, screen_y);
                    if !source_x.is_finite() || !source_y.is_finite() {
                        continue;
                    }

                    if fill.is_some() && point_in_svg_fill(&path.geometry, source_x, source_y) {
                        fill_hits = fill_hits.saturating_add(1);
                    }
                    if stroke.is_some()
                        && point_in_svg_stroke(
                            &path.geometry,
                            source_x,
                            source_y,
                            path.paint.stroke_width * 0.5,
                        )
                    {
                        stroke_hits = stroke_hits.saturating_add(1);
                    }
                }

                let index = local_row_start + x as usize;
                if let Some(fill) = fill && fill_hits > 0 {
                    blend_linear_over(
                        buffer,
                        index,
                        with_coverage(fill, fill_hits as f32 / SVG_COVERAGE_SAMPLES.len() as f32),
                    );
                }
                if let Some(stroke) = stroke && stroke_hits > 0 {
                    blend_linear_over(
                        buffer,
                        index,
                        with_coverage(
                            stroke,
                            stroke_hits as f32 / SVG_COVERAGE_SAMPLES.len() as f32,
                        ),
                    );
                }
            }
        }
    }
}

fn svg_view_box_matrix(layout: LayoutBox, view_box: SvgViewBox) -> Option<AffineTransform> {
    if layout.width <= f32::EPSILON
        || layout.height <= f32::EPSILON
        || view_box.width <= f32::EPSILON
        || view_box.height <= f32::EPSILON
    {
        return None;
    }

    let scale_x = layout.width / view_box.width;
    let scale_y = layout.height / view_box.height;
    Some(
        AffineTransform::translate(layout.x, layout.y)
            .multiply(scale_matrix(scale_x, scale_y))
            .multiply(AffineTransform::translate(-view_box.min_x, -view_box.min_y)),
    )
}

fn scale_matrix(scale_x: f32, scale_y: f32) -> AffineTransform {
    AffineTransform {
        a: scale_x,
        b: 0.0,
        c: 0.0,
        d: scale_y,
        e: 0.0,
        f: 0.0,
        g: 0.0,
        h: 0.0,
        i: 1.0,
    }
}

fn transform_svg_bounds(bounds: SvgBounds, matrix: AffineTransform) -> Option<ClipRect> {
    transform_clip_rect(
        ClipRect {
            x0: bounds.min_x,
            y0: bounds.min_y,
            x1: bounds.max_x,
            y1: bounds.max_y,
        },
        matrix,
    )
}

fn with_coverage(color: LinearRgba, coverage: f32) -> LinearRgba {
    LinearRgba {
        a: (color.a * coverage).clamp(0.0, 1.0),
        ..color
    }
}

fn point_in_svg_fill(geometry: &SvgPathGeometry, x: f32, y: f32) -> bool {
    let Some(bounds) = geometry.bounds else {
        return false;
    };
    if x < bounds.min_x || x > bounds.max_x || y < bounds.min_y || y > bounds.max_y {
        return false;
    }

    let point = SvgPoint::new(x, y);
    let mut winding = 0_i32;
    for contour in &geometry.contours {
        if contour.points.len() < 2 {
            continue;
        }

        for segment_index in 0..contour.points.len() {
            let start = contour.points[segment_index];
            let end = if segment_index + 1 < contour.points.len() {
                contour.points[segment_index + 1]
            } else {
                contour.points[0]
            };
            if point_segment_distance_sq(point, start, end) <= 1e-4 {
                return true;
            }
            if start.y <= y {
                if end.y > y && is_left(start, end, point) > 0.0 {
                    winding += 1;
                }
            } else if end.y <= y && is_left(start, end, point) < 0.0 {
                winding -= 1;
            }
        }
    }

    winding != 0
}

fn point_in_svg_stroke(
    geometry: &SvgPathGeometry,
    x: f32,
    y: f32,
    half_width: f32,
) -> bool {
    if half_width <= f32::EPSILON {
        return false;
    }

    let Some(bounds) = geometry.bounds else {
        return false;
    };
    let bounds = bounds.expand(half_width);
    if x < bounds.min_x || x > bounds.max_x || y < bounds.min_y || y > bounds.max_y {
        return false;
    }

    let threshold_sq = half_width * half_width;
    let point = SvgPoint::new(x, y);
    for contour in &geometry.contours {
        if contour.points.len() < 2 {
            continue;
        }

        for segment_index in 0..(contour.points.len() - 1) {
            let start = contour.points[segment_index];
            let end = contour.points[segment_index + 1];
            if point_segment_distance_sq(point, start, end) <= threshold_sq {
                return true;
            }
        }
        if contour.closed {
            let start = contour.points[contour.points.len() - 1];
            let end = contour.points[0];
            if point_segment_distance_sq(point, start, end) <= threshold_sq {
                return true;
            }
        }
    }

    false
}

fn is_left(start: SvgPoint, end: SvgPoint, point: SvgPoint) -> f32 {
    (end.x - start.x) * (point.y - start.y) - (point.x - start.x) * (end.y - start.y)
}

fn point_segment_distance_sq(point: SvgPoint, start: SvgPoint, end: SvgPoint) -> f32 {
    let dx = end.x - start.x;
    let dy = end.y - start.y;
    let length_sq = dx * dx + dy * dy;
    if length_sq <= f32::EPSILON {
        let px = point.x - start.x;
        let py = point.y - start.y;
        return px * px + py * py;
    }

    let t = (((point.x - start.x) * dx) + ((point.y - start.y) * dy)) / length_sq;
    let t = t.clamp(0.0, 1.0);
    let nearest_x = start.x + dx * t;
    let nearest_y = start.y + dy * t;
    let offset_x = point.x - nearest_x;
    let offset_y = point.y - nearest_y;
    offset_x * offset_x + offset_y * offset_y
}

#[cfg(test)]
mod tests {
    use cssimpler_core::{
        Color, LayoutBox, RenderNode, SvgContour, SvgPathGeometry, SvgPathInstance, SvgPathPaint,
        SvgPoint, SvgScene, SvgViewBox,
    };

    use super::super::{pack_rgb, render_to_buffer};

    #[test]
    fn svg_fill_renders_inside_the_mapped_view_box() {
        let scene = vec![RenderNode::svg(
            LayoutBox::new(2.0, 2.0, 20.0, 20.0),
            SvgScene::new(
                SvgViewBox::new(0.0, 0.0, 10.0, 10.0),
                vec![SvgPathInstance {
                    geometry: SvgPathGeometry::new(vec![SvgContour {
                        points: vec![
                            SvgPoint::new(1.0, 1.0),
                            SvgPoint::new(9.0, 1.0),
                            SvgPoint::new(9.0, 9.0),
                            SvgPoint::new(1.0, 9.0),
                        ],
                        closed: true,
                    }]),
                    paint: SvgPathPaint {
                        fill: Some(Color::rgb(37, 99, 235)),
                        stroke: None,
                        stroke_width: 0.0,
                    },
                }],
            ),
        )];
        let mut buffer = vec![0_u32; 24 * 24];

        render_to_buffer(&scene, &mut buffer, 24, 24, Color::WHITE);

        assert_eq!(buffer[12 * 24 + 12], pack_rgb(Color::rgb(37, 99, 235)));
        assert_eq!(buffer[1 * 24 + 1], pack_rgb(Color::WHITE));
    }

    #[test]
    fn svg_strokes_render_without_filling_the_whole_path_bounds() {
        let scene = vec![RenderNode::svg(
            LayoutBox::new(0.0, 0.0, 24.0, 24.0),
            SvgScene::new(
                SvgViewBox::new(0.0, 0.0, 24.0, 24.0),
                vec![SvgPathInstance {
                    geometry: SvgPathGeometry::new(vec![SvgContour {
                        points: vec![SvgPoint::new(4.0, 12.0), SvgPoint::new(20.0, 12.0)],
                        closed: false,
                    }]),
                    paint: SvgPathPaint {
                        fill: None,
                        stroke: Some(Color::rgb(22, 163, 74)),
                        stroke_width: 2.0,
                    },
                }],
            ),
        )];
        let mut buffer = vec![0_u32; 24 * 24];

        render_to_buffer(&scene, &mut buffer, 24, 24, Color::WHITE);

        assert_eq!(buffer[12 * 24 + 12], pack_rgb(Color::rgb(22, 163, 74)));
        assert_eq!(buffer[8 * 24 + 12], pack_rgb(Color::WHITE));
    }
}
