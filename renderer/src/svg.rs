use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::Instant;

use super::VectorFrameStats;

/// Process-wide opt-in for the detailed per-path clocks. A frame start copies
/// this into `TIMING_ACTIVE`, so changing it never affects a frame in progress.
static TIMING_REQUESTED: AtomicBool = AtomicBool::new(false);
// Whether the calling thread is currently painting a frame whose work counts
// should be collected. Worker tasks inherit it through `Scope::spawn`.
thread_local! {
    static COLLECTING: Cell<bool> = const { Cell::new(false) };
}

pub(super) fn is_collecting() -> bool {
    COLLECTING.with(Cell::get)
}

pub(super) fn with_collecting<R>(active: bool, f: impl FnOnce() -> R) -> R {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            COLLECTING.with(|value| value.set(self.0));
        }
    }
    let previous = COLLECTING.with(|value| value.replace(active));
    let _restore = Restore(previous);
    f()
}

/// Detailed clocks are active for the frame currently being painted.
static TIMING_ACTIVE: AtomicBool = AtomicBool::new(false);
/// Per-frame work counts. Reset at frame start and published once after the
/// frame's paint jobs have been joined, so the snapshot is consistent.
static PATHS_PAINTED: AtomicUsize = AtomicUsize::new(0);
static LAYER_HITS: AtomicUsize = AtomicUsize::new(0);
static LAYER_MISSES: AtomicUsize = AtomicUsize::new(0);
/// Per-frame durations, accumulated in nanoseconds by paint jobs. Sums can
/// overlap across workers; they are not wall time.
static PATH_BUILD_NS: AtomicU64 = AtomicU64::new(0);
static STROKE_NS: AtomicU64 = AtomicU64::new(0);
static PROJECTION_NS: AtomicU64 = AtomicU64::new(0);
static COVERAGE_NS: AtomicU64 = AtomicU64::new(0);

pub(super) fn set_vector_timing_enabled(enabled: bool) {
    TIMING_REQUESTED.store(enabled, Ordering::Relaxed);
}

pub(super) fn reset_frame_vector_stats() {
    for counter in [&PATHS_PAINTED, &LAYER_HITS, &LAYER_MISSES] {
        counter.store(0, Ordering::Relaxed);
    }
    for counter in [&PATH_BUILD_NS, &STROKE_NS, &PROJECTION_NS, &COVERAGE_NS] {
        counter.store(0, Ordering::Relaxed);
    }
    TIMING_ACTIVE.store(TIMING_REQUESTED.load(Ordering::Relaxed), Ordering::Relaxed);
    COLLECTING.with(|value| value.set(true));
}

pub(super) fn record_layer_lookup(hit: bool) {
    if is_collecting() {
        add(if hit { &LAYER_HITS } else { &LAYER_MISSES }, 1);
    }
}

pub(super) fn frame_layer_lookups() -> (usize, usize) {
    (
        LAYER_HITS.load(Ordering::Relaxed),
        LAYER_MISSES.load(Ordering::Relaxed),
    )
}

pub(super) fn take_frame_vector_stats() -> VectorFrameStats {
    COLLECTING.with(|value| value.set(false));
    VectorFrameStats {
        paths_painted: PATHS_PAINTED.load(Ordering::Relaxed),
        timings_enabled: TIMING_ACTIVE.load(Ordering::Relaxed),
        path_build_work_us: ns_to_us(PATH_BUILD_NS.load(Ordering::Relaxed)),
        stroke_work_us: ns_to_us(STROKE_NS.load(Ordering::Relaxed)),
        projection_work_us: ns_to_us(PROJECTION_NS.load(Ordering::Relaxed)),
        coverage_work_us: ns_to_us(COVERAGE_NS.load(Ordering::Relaxed)),
    }
}

/// Nanoseconds to whole microseconds, saturating rather than wrapping.
fn ns_to_us(nanoseconds: u64) -> usize {
    usize::try_from(nanoseconds / 1_000).unwrap_or(usize::MAX)
}

/// Saturating so diagnostic counters cannot wrap on very long frames.
// `fetch_update` is deprecated in newer nightlies in favour of `try_update`, but
// it remains available on older supported toolchains; keep it for compatibility.
#[allow(deprecated)]
fn add(counter: &AtomicUsize, value: usize) {
    let _ = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |old| {
        Some(old.saturating_add(value))
    });
}

#[allow(deprecated)]
fn add_time(counter: &AtomicU64, value: u64) {
    let _ = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |old| {
        Some(old.saturating_add(value))
    });
}

fn elapsed_ns(start: Option<Instant>) -> u64 {
    start
        .map(|start| start.elapsed().as_nanos().min(u64::MAX as u128) as u64)
        .unwrap_or(0)
}

use tiny_skia::{
    FillRule, LineCap, LineJoin, Mask, Path, PathBuilder, PathSegment, Point, Stroke, Transform,
};

use cssimpler_core::{
    CornerRadius, GradientInterpolation, LinearRgba, SvgBounds, SvgLinearGradient,
    SvgPaintServerData, SvgPathGeometry, SvgPathPaintSource, SvgScene, SvgViewBox,
};

use super::{
    ClipRect, blend_linear_over, clip_pixel_bounds, current_render_buffer_rows,
    transform::{AffineTransform, ClipState, transform_clip_rect},
};

use super::gradient::{
    PreparedResolvedGradient, ResolvedGradientStop, prepare_resolved_gradient,
    sample_prepared_gradient,
};

use cssimpler_core::LayoutBox;

// Coverage only: no RGBA pixmap or cached scene layer. Fixed size even for huge SVGs.
const COVERAGE_TILE_SIZE: usize = 256;
thread_local! {
    static COVERAGE_TILE: RefCell<Option<Mask>> = const { RefCell::new(None) };
}

const CLIP_COVERAGE_SAMPLES: [(f32, f32); 4] =
    [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)];

struct PreparedSvgLinearGradient {
    start_x: f32,
    start_y: f32,
    dx: f32,
    dy: f32,
    inverse_length_sq: f32,
    stops: PreparedResolvedGradient,
}

enum SvgResolvedPaint {
    Color(LinearRgba),
    LinearGradient(PreparedSvgLinearGradient),
}

impl SvgResolvedPaint {
    fn sample(&self, x: f32, y: f32) -> LinearRgba {
        match self {
            Self::Color(color) => *color,
            Self::LinearGradient(gradient) => {
                if gradient.inverse_length_sq <= f32::EPSILON {
                    return sample_prepared_gradient(&gradient.stops, 0.0, false);
                }

                let projection = (((x - gradient.start_x) * gradient.dx)
                    + ((y - gradient.start_y) * gradient.dy))
                    * gradient.inverse_length_sq;
                sample_prepared_gradient(&gradient.stops, projection, false)
            }
        }
    }
}

pub(crate) fn draw_svg_scene(
    buffer: &mut [u32],
    width: usize,
    height: usize,
    layout: LayoutBox,
    scene: &SvgScene,
    clip: ClipRect,
) {
    let Some(matrix) = svg_view_box_matrix(layout, scene) else {
        return;
    };
    let clip_state = ClipState::new(clip);
    if scene.view.is_some() {
        // A scene with a viewport view is clipped to the element's layout box.
        // `matrix` maps document coordinates, so clip with the untransformed
        // layout box in this non-CSS-transformed path.
        let Some(clipped) =
            clip_state.push_layout_clip(layout, CornerRadius::ZERO, AffineTransform::IDENTITY)
        else {
            return;
        };
        draw_svg_scene_with_matrix(buffer, width, height, scene, matrix, &clipped);
    } else {
        draw_svg_scene_with_matrix(buffer, width, height, scene, matrix, &clip_state);
    }
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
    let Some(view_matrix) = svg_view_box_matrix(layout, scene) else {
        return;
    };
    let full_matrix = matrix.multiply(view_matrix);
    if scene.view.is_some() {
        // `matrix` is the existing CSS/layout transform of this node; push the
        // viewport clip through it so clipping composes with CSS as before.
        let Some(clipped) = clip_state.push_layout_clip(layout, CornerRadius::ZERO, matrix) else {
            return;
        };
        draw_svg_scene_with_matrix(buffer, width, height, scene, full_matrix, &clipped);
    } else {
        draw_svg_scene_with_matrix(buffer, width, height, scene, full_matrix, clip_state);
    }
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
    let collecting = is_collecting();
    let timed = collecting && TIMING_ACTIVE.load(Ordering::Relaxed);
    let mut paths_painted = 0usize;
    let (mut build_ns, mut stroke_ns, mut projection_ns, mut coverage_ns) =
        (0u64, 0u64, 0u64, 0u64);
    for path in &scene.paths {
        let Some(source_bounds) = path.bounds() else {
            continue;
        };
        if transform_svg_bounds(source_bounds, matrix)
            .and_then(|bounds| bounds.intersect(clip_state.coarse))
            .is_none()
        {
            continue;
        }
        let fill = path.paint.fill.as_ref().map(|paint| {
            resolve_svg_paint_source(paint, scene).unwrap_or_else(|message| panic!("{message}"))
        });
        let stroke = (path.paint.stroke_width > f32::EPSILON)
            .then_some(path.paint.stroke.as_ref())
            .flatten()
            .map(|paint| {
                resolve_svg_paint_source(paint, scene).unwrap_or_else(|message| panic!("{message}"))
            });
        if fill.is_none() && stroke.is_none() {
            continue;
        }
        paths_painted = paths_painted.saturating_add(1);
        let start = timed.then(Instant::now);
        let source_path = build_svg_path(&path.geometry);
        build_ns = build_ns.saturating_add(elapsed_ns(start));
        let Some(source_path) = source_path else {
            continue;
        };
        let start = (timed && fill.is_some()).then(Instant::now);
        let fill_path = fill
            .as_ref()
            .and_then(|_| project_svg_path(&source_path, matrix));
        projection_ns = projection_ns.saturating_add(elapsed_ns(start));
        if let (Some(fill), Some(screen_path)) = (fill, fill_path) {
            let start = timed.then(Instant::now);
            draw_svg_coverage(
                buffer,
                width,
                height,
                &screen_path,
                &fill,
                inverse,
                clip_state,
            );
            coverage_ns = coverage_ns.saturating_add(elapsed_ns(start));
        }
        if let Some(stroke) = stroke {
            // The previous distance-to-segment rasterizer used round caps and joins.
            // Stroke in source space so nonuniform scaling also scales the outline.
            let stroke_style = Stroke {
                width: path.paint.stroke_width,
                line_cap: LineCap::Round,
                line_join: LineJoin::Round,
                ..Stroke::default()
            };
            let resolution_scale = (matrix.a.abs() + matrix.b.abs())
                .max(matrix.c.abs() + matrix.d.abs())
                .max(1.0);
            let start = timed.then(Instant::now);
            let outline = source_path.stroke(&stroke_style, resolution_scale);
            stroke_ns = stroke_ns.saturating_add(elapsed_ns(start));
            let start = (timed && outline.is_some()).then(Instant::now);
            let screen_path = outline.and_then(|outline| project_svg_path(&outline, matrix));
            projection_ns = projection_ns.saturating_add(elapsed_ns(start));
            if let Some(screen_path) = screen_path {
                let start = timed.then(Instant::now);
                draw_svg_coverage(
                    buffer,
                    width,
                    height,
                    &screen_path,
                    &stroke,
                    inverse,
                    clip_state,
                );
                coverage_ns = coverage_ns.saturating_add(elapsed_ns(start));
            }
        }
    }
    if collecting {
        add(&PATHS_PAINTED, paths_painted);
    }
    if timed {
        add_time(&PATH_BUILD_NS, build_ns);
        add_time(&STROKE_NS, stroke_ns);
        add_time(&PROJECTION_NS, projection_ns);
        add_time(&COVERAGE_NS, coverage_ns);
    }
}

fn build_svg_path(geometry: &SvgPathGeometry) -> Option<Path> {
    let mut builder = PathBuilder::new();
    if !geometry.commands.is_empty() {
        use cssimpler_core::PathCommand;
        for command in &geometry.commands {
            match *command {
                PathCommand::MoveTo(p) => builder.move_to(p.x, p.y),
                PathCommand::LineTo(p) => builder.line_to(p.x, p.y),
                PathCommand::QuadTo { control: c, end: p } => builder.quad_to(c.x, c.y, p.x, p.y),
                PathCommand::CubicTo {
                    control1: a,
                    control2: b,
                    end: p,
                } => builder.cubic_to(a.x, a.y, b.x, b.y, p.x, p.y),
                PathCommand::Close => builder.close(),
            }
        }
        return builder.finish();
    }
    for contour in &geometry.contours {
        if contour.points.len() < 2 {
            continue;
        }
        if contour
            .points
            .iter()
            .any(|p| !p.x.is_finite() || !p.y.is_finite())
        {
            return None;
        }
        let start = contour.points[0];
        builder.move_to(start.x, start.y);
        for point in &contour.points[1..] {
            builder.line_to(point.x, point.y);
        }
        if contour.closed {
            builder.close();
        }
    }
    builder.finish()
}

fn project_svg_path(path: &Path, matrix: AffineTransform) -> Option<Path> {
    if matrix.g == 0.0 && matrix.h == 0.0 && matrix.i == 1.0 {
        return path.clone().transform(Transform::from_row(
            matrix.a, matrix.b, matrix.c, matrix.d, matrix.e, matrix.f,
        ));
    }
    // CSS perspective is a homography. tiny-skia accepts affine transforms only,
    // so project lines and adaptively flatten the stroker's curved caps/joins.
    let mut builder = PathBuilder::new();
    let mut current = Point::from_xy(0.0, 0.0);
    let mut start = current;
    for segment in path.segments() {
        match segment {
            PathSegment::MoveTo(p) => {
                let p_screen = project_point(p, matrix)?;
                builder.move_to(p_screen.x, p_screen.y);
                start = p;
                current = p;
            }
            PathSegment::LineTo(p) => {
                let p_screen = project_point(p, matrix)?;
                builder.line_to(p_screen.x, p_screen.y);
                current = p;
            }
            PathSegment::QuadTo(control, end) => {
                project_curve(&mut builder, &[current, control, end], matrix, 0.0, 1.0, 0)?;
                current = end;
            }
            PathSegment::CubicTo(c1, c2, end) => {
                project_curve(&mut builder, &[current, c1, c2, end], matrix, 0.0, 1.0, 0)?;
                current = end;
            }
            PathSegment::Close => {
                builder.close();
                current = start;
            }
        }
    }
    builder.finish()
}

fn project_point(point: Point, matrix: AffineTransform) -> Option<Point> {
    let (x, y) = matrix.transform_point(point.x, point.y);
    (x.is_finite() && y.is_finite()).then_some(Point::from_xy(x, y))
}

fn project_curve(
    builder: &mut PathBuilder,
    controls: &[Point],
    matrix: AffineTransform,
    t0: f32,
    t1: f32,
    depth: u8,
) -> Option<()> {
    let sample = |t: f32| {
        let mut points = [Point::from_xy(0.0, 0.0); 4];
        points[..controls.len()].copy_from_slice(controls);
        for count in (1..controls.len()).rev() {
            for i in 0..count {
                points[i] = Point::from_xy(
                    points[i].x * (1.0 - t) + points[i + 1].x * t,
                    points[i].y * (1.0 - t) + points[i + 1].y * t,
                );
            }
        }
        project_point(points[0], matrix)
    };
    let start = sample(t0)?;
    let end = sample(t1)?;
    let mut flat = true;
    for fraction in [0.25, 0.5, 0.75] {
        let point = sample(t0 + (t1 - t0) * fraction)?;
        let dx = point.x - (start.x + (end.x - start.x) * fraction);
        let dy = point.y - (start.y + (end.y - start.y) * fraction);
        flat &= dx * dx + dy * dy <= 0.0625; // quarter device pixel
    }
    if flat || depth >= 12 {
        builder.line_to(end.x, end.y);
    } else {
        let mid = (t0 + t1) * 0.5;
        project_curve(builder, controls, matrix, t0, mid, depth + 1)?;
        project_curve(builder, controls, matrix, mid, t1, depth + 1)?;
    }
    Some(())
}

fn draw_svg_coverage(
    buffer: &mut [u32],
    width: usize,
    height: usize,
    path: &Path,
    paint: &SvgResolvedPaint,
    inverse: AffineTransform,
    clip_state: &ClipState,
) {
    let bounds = path.bounds();
    let Some(bounds) = (ClipRect {
        x0: bounds.left(),
        y0: bounds.top(),
        x1: bounds.right(),
        y1: bounds.bottom(),
    })
    .intersect(clip_state.coarse) else {
        return;
    };
    let Some((x0, y0, x1, y1)) = clip_pixel_bounds(bounds, width, height) else {
        return;
    };
    let rows = current_render_buffer_rows();
    let y0 = y0.max(rows.start.min(height) as i32);
    let y1 = y1.min(rows.end.min(height) as i32);
    if y0 >= y1 {
        return;
    }
    COVERAGE_TILE.with_borrow_mut(|tile| {
        let tile = tile.get_or_insert_with(|| {
            Mask::new(COVERAGE_TILE_SIZE as u32, COVERAGE_TILE_SIZE as u32)
                .expect("fixed SVG coverage tile dimensions are valid")
        });
        // Anchor tiles to device coordinates so dirty clips and worker bands
        // rasterize identical edges, independent of where their slice starts.
        let tile_size = COVERAGE_TILE_SIZE as i32;
        for tile_y in ((y0 / tile_size) * tile_size..y1).step_by(COVERAGE_TILE_SIZE) {
            let end_y = (tile_y + COVERAGE_TILE_SIZE as i32).min(y1);
            for tile_x in ((x0 / tile_size) * tile_size..x1).step_by(COVERAGE_TILE_SIZE) {
                let end_x = (tile_x + COVERAGE_TILE_SIZE as i32).min(x1);
                tile.clear();
                tile.fill_path(
                    path,
                    FillRule::Winding,
                    true,
                    Transform::from_translate(-(tile_x as f32), -(tile_y as f32)),
                );
                let tile_layout = LayoutBox::new(
                    tile_x.max(x0) as f32,
                    tile_y.max(y0) as f32,
                    (end_x - tile_x.max(x0)) as f32,
                    (end_y - tile_y.max(y0)) as f32,
                );
                let unclipped = clip_state.coarse.contains_rect(ClipRect {
                    x0: tile_layout.x,
                    y0: tile_layout.y,
                    x1: end_x as f32,
                    y1: end_y as f32,
                }) && clip_state.is_unclipped_by_regions(tile_layout);
                for y in tile_y.max(y0)..end_y {
                    let local_row = (y as usize - rows.start) * width;
                    let mask_row = (y - tile_y) as usize * COVERAGE_TILE_SIZE;
                    for x in tile_x.max(x0)..end_x {
                        let coverage = tile.data()[mask_row + (x - tile_x) as usize];
                        if coverage == 0 {
                            continue;
                        }
                        let clip_coverage = if unclipped {
                            1.0
                        } else {
                            CLIP_COVERAGE_SAMPLES
                                .iter()
                                .filter(|&&(sx, sy)| {
                                    clip_state.contains(x as f32 + sx, y as f32 + sy)
                                })
                                .count() as f32
                                / CLIP_COVERAGE_SAMPLES.len() as f32
                        };
                        if clip_coverage == 0.0 {
                            continue;
                        }
                        let color = match paint {
                            SvgResolvedPaint::Color(color) => *color,
                            SvgResolvedPaint::LinearGradient(_) => {
                                let (sx, sy) =
                                    inverse.transform_point(x as f32 + 0.5, y as f32 + 0.5);
                                if !sx.is_finite() || !sy.is_finite() {
                                    continue;
                                }
                                paint.sample(sx, sy)
                            }
                        };
                        blend_linear_over(
                            buffer,
                            local_row + x as usize,
                            with_coverage(color, f32::from(coverage) / 255.0 * clip_coverage),
                        );
                    }
                }
            }
        }
    });
}

fn resolve_svg_paint_source(
    paint: &SvgPathPaintSource,
    scene: &SvgScene,
) -> Result<SvgResolvedPaint, String> {
    match paint {
        SvgPathPaintSource::Color(color) => Ok(SvgResolvedPaint::Color(color.to_linear_rgba())),
        SvgPathPaintSource::PaintServer(reference) => {
            let Some(server) = scene
                .paint_servers
                .iter()
                .find(|server| server.id == reference.id)
            else {
                return Err(format!(
                    "missing SVG paint-server data for url(#{})",
                    reference.id
                ));
            };
            if server.kind != reference.kind {
                return Err(format!(
                    "SVG paint-server url(#{}) changed kind during rendering",
                    reference.id
                ));
            }

            match &server.data {
                SvgPaintServerData::LinearGradient(gradient) => {
                    Ok(SvgResolvedPaint::LinearGradient(
                        prepare_svg_linear_gradient(gradient, scene.view_box),
                    ))
                }
            }
        }
    }
}

fn prepare_svg_linear_gradient(
    gradient: &SvgLinearGradient,
    view_box: SvgViewBox,
) -> PreparedSvgLinearGradient {
    let start_x = view_box.min_x + gradient.x1.resolve(view_box.width);
    let start_y = view_box.min_y + gradient.y1.resolve(view_box.height);
    let end_x = view_box.min_x + gradient.x2.resolve(view_box.width);
    let end_y = view_box.min_y + gradient.y2.resolve(view_box.height);
    let dx = end_x - start_x;
    let dy = end_y - start_y;
    let length_sq = dx * dx + dy * dy;
    let stops = gradient
        .stops
        .iter()
        .map(|stop| ResolvedGradientStop {
            color: stop.color.to_linear_rgba(),
            position: stop.offset,
        })
        .collect::<Vec<_>>();

    PreparedSvgLinearGradient {
        start_x,
        start_y,
        dx,
        dy,
        inverse_length_sq: if length_sq <= f32::EPSILON {
            0.0
        } else {
            1.0 / length_sq
        },
        stops: prepare_resolved_gradient(&stops, GradientInterpolation::LinearSrgb),
    }
}

fn svg_view_box_matrix(layout: LayoutBox, scene: &SvgScene) -> Option<AffineTransform> {
    let view_box = scene.view_box;
    if layout.width <= f32::EPSILON
        || layout.height <= f32::EPSILON
        || view_box.width <= f32::EPSILON
        || view_box.height <= f32::EPSILON
    {
        return None;
    }

    let scale_x = layout.width / view_box.width;
    let scale_y = layout.height / view_box.height;
    let mut matrix = AffineTransform::translate(layout.x, layout.y)
        .multiply(scale_matrix(scale_x, scale_y))
        .multiply(AffineTransform::translate(-view_box.min_x, -view_box.min_y));
    if let Some(view) = scene.view {
        // Document -> viewBox coordinates, applied before the layout/viewBox
        // map. `VectorView` guarantees a finite positive zoom, so this composes
        // without introducing a singular matrix.
        matrix = matrix.multiply(AffineTransform {
            a: view.zoom(),
            b: 0.0,
            c: 0.0,
            d: view.zoom(),
            e: view.offset().x,
            f: view.offset().y,
            g: 0.0,
            h: 0.0,
            i: 1.0,
        });
    }
    Some(matrix)
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

#[cfg(test)]
mod tests {
    use cssimpler_core::{
        Color, LayoutBox, LengthPercentageValue, RenderNode, SvgContour, SvgGradientStop,
        SvgLinearGradient, SvgPaintServer, SvgPaintServerData, SvgPaintServerKind,
        SvgPaintServerReference, SvgPathGeometry, SvgPathInstance, SvgPathPaint,
        SvgPathPaintSource, SvgPoint, SvgScene, SvgViewBox,
    };

    use super::super::{pack_rgb, render_to_buffer, unpack_rgb};

    #[test]
    fn vector_counters_aggregate_workers_and_saturate() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let counter = AtomicUsize::new(0);
        std::thread::scope(|scope| {
            for _ in 0..4 {
                let counter = &counter;
                scope.spawn(move || {
                    for _ in 0..1000 {
                        super::add(counter, 1);
                    }
                });
            }
        });
        assert_eq!(counter.load(Ordering::Relaxed), 4000);
        super::add(&counter, usize::MAX);
        assert_eq!(counter.load(Ordering::Relaxed), usize::MAX);
        let duration = std::sync::atomic::AtomicU64::new(u64::MAX - 1);
        super::add_time(&duration, 2);
        assert_eq!(duration.load(Ordering::Relaxed), u64::MAX);
    }

    #[test]
    fn vector_scope_follows_jobs_and_restores_worker_state() {
        let pool = super::super::WorkerPool::new(3);
        super::with_collecting(true, || {
            pool.scope(|scope| {
                for _ in 0..4 {
                    scope.spawn(|| assert!(super::is_collecting()));
                }
            });
            std::thread::scope(|scope| {
                scope.spawn(|| assert!(!super::is_collecting()));
            });
        });
        assert!(!super::is_collecting());
        pool.scope(|scope| {
            for _ in 0..4 {
                scope.spawn(|| assert!(!super::is_collecting()));
            }
        });
    }

    static VECTOR_STATS_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn vector_timing_switch_is_sampled_at_frame_start() {
        let _guard = VECTOR_STATS_TEST_LOCK.lock().unwrap();
        super::set_vector_timing_enabled(true);
        super::reset_frame_vector_stats();
        // Toggling mid-frame must not change the frame already in progress.
        super::set_vector_timing_enabled(false);
        assert!(super::take_frame_vector_stats().timings_enabled);

        super::reset_frame_vector_stats();
        super::set_vector_timing_enabled(true);
        assert!(!super::take_frame_vector_stats().timings_enabled);
        super::set_vector_timing_enabled(false);
    }

    /// Instrumentation overhead harness (ignored by default; release build).
    ///
    /// Compares three collection modes on the same buffers and scene:
    /// `off` (no frame collection), `counters` (default frame diagnostics:
    /// work counts only), and `timers` (opt-in detailed clocks).
    #[test]
    #[ignore = "manual release-mode instrumentation overhead measurement"]
    fn vector_timing_overhead() {
        const ROUNDS: usize = 5;
        const TINY_PAINTS: usize = 3000;
        const MIXED_PAINTS: usize = 100;
        const MIXED_PATHS: usize = 16;

        #[derive(Clone, Copy, Debug)]
        #[repr(usize)]
        enum Mode {
            Off = 0,
            Counters = 1,
            Timers = 2,
        }

        fn timed_paints(
            scene: &SvgScene,
            buffer: &mut [u32],
            width: usize,
            height: usize,
            layout: LayoutBox,
            paints: usize,
        ) -> u128 {
            let start = std::time::Instant::now();
            for _ in 0..paints {
                super::draw_svg_scene(
                    buffer,
                    width,
                    height,
                    layout,
                    scene,
                    super::ClipRect::full(width as f32, height as f32),
                );
            }
            start.elapsed().as_micros()
        }

        fn measure(
            mode: Mode,
            scene: &SvgScene,
            buffer: &mut [u32],
            width: usize,
            height: usize,
            layout: LayoutBox,
            paints: usize,
            paths_per_paint: usize,
        ) -> u128 {
            match mode {
                Mode::Off => super::with_collecting(false, || {
                    super::set_vector_timing_enabled(false);
                    timed_paints(scene, buffer, width, height, layout, paints)
                }),
                Mode::Counters | Mode::Timers => {
                    let enabled = matches!(mode, Mode::Timers);
                    super::set_vector_timing_enabled(enabled);
                    super::reset_frame_vector_stats();
                    let micros = timed_paints(scene, buffer, width, height, layout, paints);
                    let stats = super::take_frame_vector_stats();
                    assert_eq!(stats.timings_enabled, enabled);
                    assert_eq!(stats.paths_painted, paints.saturating_mul(paths_per_paint));
                    if enabled {
                        assert!(stats.coverage_work_us > 0 || stats.path_build_work_us > 0);
                    } else {
                        assert_eq!(stats.coverage_work_us, 0);
                    }
                    micros
                }
            }
        }

        fn median(samples: &mut [u128]) -> u128 {
            samples.sort_unstable();
            samples[samples.len() / 2]
        }

        fn report(name: &str, paints: usize, samples: &mut [Vec<u128>; 3]) {
            let off = median(&mut samples[0]);
            let counters = median(&mut samples[1]);
            let timers = median(&mut samples[2]);
            let counters_delta = (counters as f64 / off.max(1) as f64 - 1.0) * 100.0;
            let timers_delta = (timers as f64 / counters.max(1) as f64 - 1.0) * 100.0;
            eprintln!(
                "{name} paints={paints} (us, {ROUNDS} rounds)\n  \
                 off={off} samples={:?}\n  counters={counters} samples={:?}\n  \
                 timers={timers} samples={:?}\n  \
                 counters-vs-off={counters_delta:+.1}% timers-vs-counters={timers_delta:+.1}%",
                samples[0], samples[1], samples[2]
            );
        }

        let _guard = VECTOR_STATS_TEST_LOCK.lock().unwrap();

        let tiny = test_scene(
            vec![polygon(
                &[(2.0, 2.0), (22.0, 2.0), (22.0, 22.0), (2.0, 22.0)],
                true,
            )],
            SvgPathPaint::default(),
            24.0,
        );
        let tiny_layout = LayoutBox::new(0.0, 0.0, 24.0, 24.0);
        let mut tiny_buffer = vec![pack_rgb(Color::WHITE); 24 * 24];

        let mut mixed_paths = Vec::new();
        for index in 0..MIXED_PATHS {
            let x = 20.0 + (index % 4) as f32 * 90.0;
            let y = 20.0 + (index / 4) as f32 * 65.0;
            mixed_paths.push(SvgPathInstance {
                geometry: SvgPathGeometry::new(vec![polygon(
                    &[
                        (x, y),
                        (x + 70.0, y + 10.0),
                        (x + 55.0, y + 50.0),
                        (x + 10.0, y + 45.0),
                    ],
                    true,
                )]),
                paint: SvgPathPaint {
                    stroke: Some(SvgPathPaintSource::Color(Color::rgb(20, 40, 80))),
                    stroke_width: 1.5,
                    ..SvgPathPaint::default()
                },
            });
        }
        let mixed = SvgScene::new(SvgViewBox::new(0.0, 0.0, 400.0, 300.0), mixed_paths);
        let mixed_layout = LayoutBox::new(0.0, 0.0, 400.0, 300.0);
        let mut mixed_buffer = vec![pack_rgb(Color::WHITE); 400 * 300];

        // Warm up every mode before sampling to page in buffers and caches.
        for mode in [Mode::Off, Mode::Counters, Mode::Timers] {
            measure(
                mode,
                &tiny,
                &mut tiny_buffer,
                24,
                24,
                tiny_layout,
                TINY_PAINTS / 10,
                1,
            );
            measure(
                mode,
                &mixed,
                &mut mixed_buffer,
                400,
                300,
                mixed_layout,
                MIXED_PAINTS / 10,
                MIXED_PATHS,
            );
        }

        let mut tiny_samples: [Vec<u128>; 3] = [Vec::new(), Vec::new(), Vec::new()];
        let mut mixed_samples: [Vec<u128>; 3] = [Vec::new(), Vec::new(), Vec::new()];
        let modes = [Mode::Off, Mode::Counters, Mode::Timers];
        for round in 0..ROUNDS {
            // Alternate order between rounds so slow drift cannot favor a mode.
            let order: [Mode; 3] = if round % 2 == 0 {
                modes
            } else {
                [Mode::Timers, Mode::Counters, Mode::Off]
            };
            for mode in order {
                let index = mode as usize;
                tiny_samples[index].push(measure(
                    mode,
                    &tiny,
                    &mut tiny_buffer,
                    24,
                    24,
                    tiny_layout,
                    TINY_PAINTS,
                    1,
                ));
                mixed_samples[index].push(measure(
                    mode,
                    &mixed,
                    &mut mixed_buffer,
                    400,
                    300,
                    mixed_layout,
                    MIXED_PAINTS,
                    MIXED_PATHS,
                ));
            }
        }

        super::set_vector_timing_enabled(false);
        report("tiny-filled-paths", TINY_PAINTS, &mut tiny_samples);
        report("mixed-16-path-scene", MIXED_PAINTS, &mut mixed_samples);
    }

    #[test]
    fn vector_frame_counts_reset_and_exclude_unrelated_renders() {
        let _guard = VECTOR_STATS_TEST_LOCK.lock().unwrap();
        let scene = test_scene(
            vec![polygon(
                &[(2.0, 2.0), (22.0, 2.0), (22.0, 22.0), (2.0, 22.0)],
                true,
            )],
            SvgPathPaint::default(),
            24.0,
        );
        let mut buffer = vec![pack_rgb(Color::WHITE); 24 * 24];
        super::set_vector_timing_enabled(false);
        super::reset_frame_vector_stats();
        super::record_layer_lookup(true);
        super::record_layer_lookup(false);
        let pool = super::super::WorkerPool::new(3);
        // An unrelated render on another thread must not contribute.
        std::thread::scope(|threads| {
            threads.spawn(|| {
                let mut pixels = vec![0; 24 * 24];
                super::draw_svg_scene(
                    &mut pixels,
                    24,
                    24,
                    LayoutBox::new(0.0, 0.0, 24.0, 24.0),
                    &scene,
                    super::ClipRect::full(24.0, 24.0),
                );
            });
            pool.scope(|scope| {
                for _ in 0..4 {
                    let scene = &scene;
                    scope.spawn(move || {
                        let mut pixels = vec![0; 24 * 24];
                        for _ in 0..100 {
                            super::draw_svg_scene(
                                &mut pixels,
                                24,
                                24,
                                LayoutBox::new(0.0, 0.0, 24.0, 24.0),
                                scene,
                                super::ClipRect::full(24.0, 24.0),
                            );
                        }
                        for _ in 0..10 {
                            super::record_layer_lookup(true);
                        }
                    });
                }
            });
        });
        let stats = super::take_frame_vector_stats();
        assert_eq!(super::frame_layer_lookups(), (41, 1));
        assert_eq!(stats.paths_painted, 400);
        assert!(!stats.timings_enabled);
        assert_eq!(stats.path_build_work_us, 0);
        assert_eq!(stats.stroke_work_us, 0);
        assert_eq!(stats.projection_work_us, 0);
        assert_eq!(stats.coverage_work_us, 0);
        super::reset_frame_vector_stats();
        super::draw_svg_scene(
            &mut buffer,
            24,
            24,
            LayoutBox::new(100.0, 100.0, 24.0, 24.0),
            &scene,
            super::ClipRect::full(24.0, 24.0),
        );
        assert_eq!(super::take_frame_vector_stats().paths_painted, 0);
        assert_eq!(super::frame_layer_lookups(), (0, 0));
    }

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
                        fill: Some(SvgPathPaintSource::Color(Color::rgb(37, 99, 235))),
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
                        stroke: Some(SvgPathPaintSource::Color(Color::rgb(22, 163, 74))),
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

    #[test]
    fn svg_linear_gradient_fill_samples_local_paint_server() {
        let scene = vec![RenderNode::svg(
            LayoutBox::new(0.0, 0.0, 10.0, 10.0),
            SvgScene::with_paint_servers(
                SvgViewBox::new(0.0, 0.0, 10.0, 10.0),
                vec![horizontal_gradient_server(
                    "paint",
                    Color::rgb(239, 68, 68),
                    Color::rgb(59, 130, 246),
                )],
                vec![SvgPathInstance {
                    geometry: SvgPathGeometry::new(vec![SvgContour {
                        points: vec![
                            SvgPoint::new(0.0, 0.0),
                            SvgPoint::new(10.0, 0.0),
                            SvgPoint::new(10.0, 10.0),
                            SvgPoint::new(0.0, 10.0),
                        ],
                        closed: true,
                    }]),
                    paint: SvgPathPaint {
                        fill: Some(paint_server_ref("paint")),
                        stroke: None,
                        stroke_width: 0.0,
                    },
                }],
            ),
        )];
        let mut buffer = vec![0_u32; 10 * 10];

        render_to_buffer(&scene, &mut buffer, 10, 10, Color::WHITE);

        let left = unpack_rgb(buffer[5 * 10 + 1]);
        let right = unpack_rgb(buffer[5 * 10 + 8]);
        assert!(
            left.r > left.b,
            "left fill sample should be red-dominant: {left:?}"
        );
        assert!(
            right.b > right.r,
            "right fill sample should be blue-dominant: {right:?}"
        );
    }

    #[test]
    fn svg_linear_gradient_stroke_samples_local_paint_server() {
        let scene = vec![RenderNode::svg(
            LayoutBox::new(0.0, 0.0, 10.0, 10.0),
            SvgScene::with_paint_servers(
                SvgViewBox::new(0.0, 0.0, 10.0, 10.0),
                vec![horizontal_gradient_server(
                    "strokePaint",
                    Color::rgb(239, 68, 68),
                    Color::rgb(34, 197, 94),
                )],
                vec![SvgPathInstance {
                    geometry: SvgPathGeometry::new(vec![SvgContour {
                        points: vec![SvgPoint::new(1.0, 5.0), SvgPoint::new(9.0, 5.0)],
                        closed: false,
                    }]),
                    paint: SvgPathPaint {
                        fill: None,
                        stroke: Some(paint_server_ref("strokePaint")),
                        stroke_width: 2.0,
                    },
                }],
            ),
        )];
        let mut buffer = vec![0_u32; 10 * 10];

        render_to_buffer(&scene, &mut buffer, 10, 10, Color::WHITE);

        let left = unpack_rgb(buffer[5 * 10 + 2]);
        let right = unpack_rgb(buffer[5 * 10 + 8]);
        assert!(
            left.r > left.g,
            "left stroke sample should be red-dominant: {left:?}"
        );
        assert!(
            right.g > right.r,
            "right stroke sample should be green-dominant: {right:?}"
        );
        assert_eq!(buffer[2 * 10 + 5], pack_rgb(Color::WHITE));
    }

    fn polygon(points: &[(f32, f32)], closed: bool) -> SvgContour {
        SvgContour {
            points: points.iter().map(|&(x, y)| SvgPoint::new(x, y)).collect(),
            closed,
        }
    }

    fn test_scene(contours: Vec<SvgContour>, paint: SvgPathPaint, size: f32) -> SvgScene {
        SvgScene::new(
            SvgViewBox::new(0.0, 0.0, size, size),
            vec![SvgPathInstance {
                geometry: SvgPathGeometry::new(contours),
                paint,
            }],
        )
    }

    #[test]
    fn svg_nonzero_winding_preserves_holes_and_closes_open_fills() {
        let scene = test_scene(
            vec![
                polygon(&[(2.0, 2.0), (22.0, 2.0), (22.0, 22.0), (2.0, 22.0)], false),
                polygon(&[(8.0, 8.0), (8.0, 16.0), (16.0, 16.0), (16.0, 8.0)], true),
            ],
            SvgPathPaint::default(),
            24.0,
        );
        let mut buffer = vec![pack_rgb(Color::WHITE); 24 * 24];
        super::draw_svg_scene(
            &mut buffer,
            24,
            24,
            LayoutBox::new(0.0, 0.0, 24.0, 24.0),
            &scene,
            super::ClipRect::full(24.0, 24.0),
        );
        assert_eq!(buffer[5 * 24 + 5], pack_rgb(Color::BLACK));
        assert_eq!(buffer[12 * 24 + 12], pack_rgb(Color::WHITE));
        assert_eq!(buffer[12 * 24 + 2], pack_rgb(Color::BLACK));
    }

    #[test]
    fn svg_open_strokes_have_round_caps_without_a_closing_segment() {
        let scene = test_scene(
            vec![polygon(&[(6.0, 6.0), (18.0, 6.0), (18.0, 18.0)], false)],
            SvgPathPaint {
                fill: None,
                stroke: Some(SvgPathPaintSource::Color(Color::BLACK)),
                stroke_width: 4.0,
            },
            24.0,
        );
        let mut buffer = vec![pack_rgb(Color::WHITE); 24 * 24];
        super::draw_svg_scene(
            &mut buffer,
            24,
            24,
            LayoutBox::new(0.0, 0.0, 24.0, 24.0),
            &scene,
            super::ClipRect::full(24.0, 24.0),
        );
        assert_eq!(buffer[6 * 24 + 5], pack_rgb(Color::BLACK));
        assert_eq!(buffer[18 * 24 + 18], pack_rgb(Color::BLACK));
        assert_eq!(buffer[12 * 24 + 12], pack_rgb(Color::WHITE));
    }

    #[test]
    fn svg_tiled_coverage_matches_one_mask_without_seams_or_stale_pixels() {
        let (width, height) = (600, 360);
        let mut actual = vec![pack_rgb(Color::WHITE); width * height];
        let mut expected = actual.clone();
        // The second path also checks that tile reuse clears previous coverage.
        for (contour, color) in [
            (
                polygon(
                    &[(1.2, 2.7), (598.8, 2.7), (598.8, 355.8), (1.2, 355.8)],
                    true,
                ),
                Color::rgba(10, 40, 200, 180),
            ),
            (
                polygon(
                    &[(250.3, 12.8), (275.9, 12.8), (275.9, 300.1), (250.3, 300.1)],
                    true,
                ),
                Color::rgba(200, 30, 10, 120),
            ),
        ] {
            let source = super::build_svg_path(&SvgPathGeometry::new(vec![contour])).unwrap();
            let paint = super::SvgResolvedPaint::Color(color.to_linear_rgba());
            super::draw_svg_coverage(
                &mut actual,
                width,
                height,
                &source,
                &paint,
                super::AffineTransform::IDENTITY,
                &super::ClipState::new(super::ClipRect::full(width as f32, height as f32)),
            );
            let mut reference = tiny_skia::Mask::new(width as u32, height as u32).unwrap();
            reference.fill_path(
                &source,
                tiny_skia::FillRule::Winding,
                true,
                tiny_skia::Transform::identity(),
            );
            for (index, &coverage) in reference.data().iter().enumerate() {
                if coverage > 0 {
                    super::blend_linear_over(
                        &mut expected,
                        index,
                        super::with_coverage(color.to_linear_rgba(), f32::from(coverage) / 255.0),
                    );
                }
            }
        }
        for (index, (actual, expected)) in actual.iter().zip(&expected).enumerate() {
            assert_eq!(
                actual,
                expected,
                "at ({}, {})",
                index % width,
                index / width
            );
        }
    }

    #[test]
    fn svg_worker_rows_and_dirty_clip_match_full_paint() {
        let scene = test_scene(
            vec![polygon(&[(1.2, 2.7), (598.8, 350.4), (12.3, 355.8)], true)],
            SvgPathPaint {
                fill: Some(SvgPathPaintSource::Color(Color::BLACK)),
                stroke: Some(SvgPathPaintSource::Color(Color::rgba(200, 40, 10, 120))),
                stroke_width: 3.0,
            },
            600.0,
        );
        let layout = LayoutBox::new(0.0, 0.0, 600.0, 600.0);
        let mut full = vec![pack_rgb(Color::WHITE); 600 * 360];
        super::draw_svg_scene(
            &mut full,
            600,
            360,
            layout,
            &scene,
            super::ClipRect::full(600.0, 360.0),
        );
        let mut band = vec![pack_rgb(Color::WHITE); 600 * 20];
        super::super::with_render_buffer_rows(super::super::BufferRows::new(250, 270), || {
            super::draw_svg_scene(
                &mut band,
                600,
                360,
                layout,
                &scene,
                super::ClipRect {
                    x0: 200.0,
                    y0: 255.0,
                    x1: 350.0,
                    y1: 265.0,
                },
            );
        });
        for y in 250..270 {
            for x in 0..600 {
                let expected = if (200..350).contains(&x) && (255..265).contains(&y) {
                    full[y * 600 + x]
                } else {
                    pack_rgb(Color::WHITE)
                };
                assert_eq!(band[(y - 250) * 600 + x], expected, "at ({x}, {y})");
            }
        }
    }

    #[test]
    fn svg_nonuniform_transform_scales_stroke_outline_before_rasterization() {
        let scene = test_scene(
            vec![polygon(&[(4.0, 2.0), (4.0, 8.0)], false)],
            SvgPathPaint {
                fill: None,
                stroke: Some(SvgPathPaintSource::Color(Color::BLACK)),
                stroke_width: 2.0,
            },
            10.0,
        );
        let mut buffer = vec![pack_rgb(Color::WHITE); 40 * 40];
        super::draw_svg_scene_transformed(
            &mut buffer,
            40,
            40,
            LayoutBox::new(0.0, 0.0, 10.0, 10.0),
            &scene,
            super::AffineTransform {
                a: 3.0,
                d: 2.0,
                e: 5.0,
                f: 3.0,
                ..super::AffineTransform::IDENTITY
            },
            &super::ClipState::new(super::ClipRect::full(40.0, 40.0)),
        );
        assert_eq!(buffer[12 * 40 + 14], pack_rgb(Color::BLACK));
        assert_eq!(buffer[12 * 40 + 19], pack_rgb(Color::BLACK));
        assert_eq!(buffer[12 * 40 + 13], pack_rgb(Color::WHITE));
        assert_eq!(buffer[12 * 40 + 20], pack_rgb(Color::WHITE));
    }

    #[test]
    fn svg_perspective_and_rounded_clip_keep_fill_and_stroke_visible() {
        let scene = test_scene(
            vec![polygon(
                &[(4.0, 4.0), (28.0, 4.0), (28.0, 28.0), (4.0, 28.0)],
                true,
            )],
            SvgPathPaint {
                fill: Some(SvgPathPaintSource::Color(Color::rgb(200, 20, 10))),
                stroke: Some(SvgPathPaintSource::Color(Color::BLACK)),
                stroke_width: 3.0,
            },
            32.0,
        );
        let mut buffer = vec![pack_rgb(Color::WHITE); 32 * 32];
        let clip = super::ClipState::new(super::ClipRect::full(32.0, 32.0))
            .push_layout_clip(
                LayoutBox::new(0.0, 0.0, 32.0, 32.0),
                cssimpler_core::CornerRadius::all(12.0),
                super::AffineTransform::IDENTITY,
            )
            .unwrap();
        super::draw_svg_scene_transformed(
            &mut buffer,
            32,
            32,
            LayoutBox::new(0.0, 0.0, 32.0, 32.0),
            &scene,
            super::AffineTransform {
                g: 0.01,
                ..super::AffineTransform::IDENTITY
            },
            &clip,
        );
        assert_eq!(buffer[12 * 32 + 12], pack_rgb(Color::rgb(200, 20, 10)));
        assert_eq!(buffer[12 * 32 + 3], pack_rgb(Color::BLACK));
        assert_eq!(buffer[2 * 32 + 2], pack_rgb(Color::WHITE));
        assert_eq!(buffer[12 * 32 + 26], pack_rgb(Color::WHITE));
    }

    #[test]
    fn svg_translucency_retains_linear_blending_and_separate_alpha_target() {
        let color = Color::rgba(240, 10, 50, 128);
        let scene = test_scene(
            vec![polygon(
                &[(1.0, 1.0), (7.0, 1.0), (7.0, 7.0), (1.0, 7.0)],
                true,
            )],
            SvgPathPaint {
                fill: Some(SvgPathPaintSource::Color(color)),
                stroke: None,
                stroke_width: 0.0,
            },
            8.0,
        );
        let mut buffer = vec![0; 64];
        let mut alpha = vec![0; 64];
        super::super::with_render_alpha_target(Some(&mut alpha), || {
            super::draw_svg_scene(
                &mut buffer,
                8,
                8,
                LayoutBox::new(0.0, 0.0, 8.0, 8.0),
                &scene,
                super::ClipRect::full(8.0, 8.0),
            );
        });
        assert_eq!(alpha[4 * 8 + 4], 128);
        assert_eq!(unpack_rgb(buffer[4 * 8 + 4]), Color::rgb(240, 10, 50));
        assert_eq!(alpha[0], 0);
        let mut opaque = vec![pack_rgb(Color::WHITE); 64];
        super::draw_svg_scene(
            &mut opaque,
            8,
            8,
            LayoutBox::new(0.0, 0.0, 8.0, 8.0),
            &scene,
            super::ClipRect::full(8.0, 8.0),
        );
        let blended = unpack_rgb(opaque[4 * 8 + 4]);
        // Linear-light blending is much brighter than an sRGB byte average.
        assert!(blended.g > 180 && blended.b > 180, "{blended:?}");
    }

    #[test]
    fn svg_parallel_repaint_matches_serial_color_and_alpha() {
        let mut svg = test_scene(
            vec![polygon(&[(3.2, 2.7), (598.8, 350.4), (12.3, 355.8)], true)],
            SvgPathPaint {
                fill: Some(paint_server_ref("paint")),
                stroke: Some(SvgPathPaintSource::Color(Color::rgba(10, 40, 200, 170))),
                stroke_width: 3.5,
            },
            600.0,
        );
        svg.paint_servers.push(horizontal_gradient_server(
            "paint",
            Color::rgba(240, 30, 10, 100),
            Color::rgba(10, 30, 240, 200),
        ));
        let scene = vec![RenderNode::svg(LayoutBox::new(0.0, 0.0, 600.0, 600.0), svg)];
        let bounds = super::super::cache_scene_subtree_bounds(&scene);
        let mut serial = vec![0; 600 * 360];
        let mut parallel = serial.clone();
        let mut serial_alpha = vec![0; serial.len()];
        let mut parallel_alpha = serial_alpha.clone();
        super::super::render_to_buffer_serial(
            &scene,
            &bounds.roots,
            &mut serial,
            600,
            360,
            Color::rgba(0, 0, 0, 0),
            super::super::GlassRenderMode::Fallback,
            Some(&mut serial_alpha),
        );
        super::super::render_to_buffer_parallel(
            &scene,
            &bounds,
            &mut parallel,
            600,
            360,
            Color::rgba(0, 0, 0, 0),
            4,
            super::super::GlassRenderMode::Fallback,
            Some(&mut parallel_alpha),
        );
        assert!(
            serial.iter().zip(&parallel).all(|(a, b)| a == b),
            "serial and parallel colors differ"
        );
        assert!(
            serial_alpha == parallel_alpha,
            "serial and parallel alpha differ"
        );
    }

    fn horizontal_gradient_server(id: &str, start: Color, end: Color) -> SvgPaintServer {
        SvgPaintServer {
            id: id.to_string(),
            kind: SvgPaintServerKind::LinearGradient,
            data: SvgPaintServerData::LinearGradient(SvgLinearGradient {
                x1: LengthPercentageValue::ZERO,
                y1: LengthPercentageValue::ZERO,
                x2: LengthPercentageValue::from_px(10.0),
                y2: LengthPercentageValue::ZERO,
                stops: vec![
                    SvgGradientStop {
                        color: start,
                        offset: 0.0,
                    },
                    SvgGradientStop {
                        color: end,
                        offset: 1.0,
                    },
                ],
            }),
        }
    }

    fn paint_server_ref(id: &str) -> SvgPathPaintSource {
        SvgPathPaintSource::PaintServer(SvgPaintServerReference {
            id: id.to_string(),
            kind: SvgPaintServerKind::LinearGradient,
        })
    }
}
