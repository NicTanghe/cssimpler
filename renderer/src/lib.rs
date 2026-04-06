use std::cell::Cell;
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

mod fonts;
mod gradient;
mod scrollbar;
mod shadow;
mod shapes;

use self::{
    gradient::draw_background_layer,
    shadow::{
        draw_shadow, draw_shadow_effect, shadow_bounds, shadow_effect_bounds, text_stroke_bounds,
    },
    shapes::{
        clip_pixel_bounds, draw_axis_aligned_opaque_rect, draw_axis_aligned_opaque_ring,
        draw_rounded_rect, draw_rounded_ring, inset_corner_radius, inset_layout, layout_clip,
        non_empty_layout_clip, offset_layout, snap_clip_to_pixel_grid, union_optional_bounds,
    },
};
use cssimpler_core::{
    Color, ElementInteractionState, ElementPath, EventHandler, LayoutBox, LinearRgba, RenderKind,
    RenderNode,
};
use minifb::{Key, MouseButton, MouseMode, Window, WindowOptions};

#[cfg(test)]
use self::{
    shadow::cached_shadow_mask,
    shapes::{expand_corner_radius, expand_layout},
};

const MAX_INCREMENTAL_DIRTY_REGIONS: usize = 32;
const MAX_INCREMENTAL_DIRTY_AREA_RATIO: f32 = 0.85;
const DIRTY_BRANCH_COLLAPSE_THRESHOLD: usize = 3;
const DIRTY_BRANCH_COLLAPSE_MAX_AREA_RATIO: f32 = 2.5;
const DIRTY_REGION_COALESCE_MAX_EXPANSION_RATIO: f32 = 1.35;
const MIN_PARALLEL_RENDER_PIXELS: usize = 640 * 480;
const MIN_INCREMENTAL_PIXELS_PER_WORKER: usize = 160 * 160;
const MIN_PARALLEL_RENDER_ROWS_PER_WORKER: usize = 80;
const MAX_RENDER_WORKERS: usize = 12;
const SCENE_TRAVERSAL_COST_PER_NODE: usize = 96;
const DOUBLE_CLICK_THRESHOLD: Duration = Duration::from_millis(500);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FramePaintMode {
    #[default]
    Idle,
    Full,
    Incremental,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FramePaintReason {
    #[default]
    Idle,
    FullRedraw,
    DirtyRegionLimit,
    DirtyAreaLimit,
    FragmentedDamage,
    IncrementalDamage,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameTimingStats {
    pub update_us: u64,
    pub scene_prep_us: u64,
    pub paint_us: u64,
    pub present_us: u64,
    pub total_us: u64,
    pub render_workers: usize,
    pub dirty_regions: usize,
    pub dirty_jobs: usize,
    pub damage_pixels: usize,
    pub painted_pixels: usize,
    pub scene_passes: usize,
    pub paint_mode: FramePaintMode,
    pub paint_reason: FramePaintReason,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct PaintStats {
    workers: usize,
    dirty_regions: usize,
    dirty_jobs: usize,
    damage_pixels: usize,
    painted_pixels: usize,
    scene_passes: usize,
    mode: FramePaintMode,
    reason: FramePaintReason,
}

#[derive(Clone, Copy, Debug)]
struct DirtyRenderJob {
    clip: ClipRect,
    pixel_count: usize,
}

static FRAME_TIMING_STATS: OnceLock<Mutex<FrameTimingStats>> = OnceLock::new();
static WORKER_BUFFER_POOL: OnceLock<Mutex<Vec<Vec<u32>>>> = OnceLock::new();

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BufferRows {
    start: usize,
    end: usize,
}

impl BufferRows {
    const fn new(start: usize, end: usize) -> Self {
        Self {
            start,
            end: if end < start { start } else { end },
        }
    }

    const fn full() -> Self {
        Self::new(0, usize::MAX)
    }

    fn len(self) -> usize {
        self.end.saturating_sub(self.start)
    }

    fn pixel_len(self, width: usize) -> usize {
        width.saturating_mul(self.len())
    }
}

thread_local! {
    static RENDER_BUFFER_ROWS: Cell<BufferRows> = const { Cell::new(BufferRows::full()) };
}

pub fn latest_frame_timing_stats() -> FrameTimingStats {
    *frame_timing_stats_store()
        .lock()
        .expect("frame timing stats mutex should not be poisoned")
}

fn record_frame_timing_stats(stats: FrameTimingStats) {
    *frame_timing_stats_store()
        .lock()
        .expect("frame timing stats mutex should not be poisoned") = stats;
}

fn frame_timing_stats_store() -> &'static Mutex<FrameTimingStats> {
    FRAME_TIMING_STATS.get_or_init(|| Mutex::new(FrameTimingStats::default()))
}

fn worker_buffer_pool() -> &'static Mutex<Vec<Vec<u32>>> {
    WORKER_BUFFER_POOL.get_or_init(|| Mutex::new(Vec::new()))
}

fn acquire_worker_buffers(lengths: &[usize]) -> Vec<Vec<u32>> {
    let mut pool = worker_buffer_pool()
        .lock()
        .expect("worker buffer pool mutex should not be poisoned");
    let mut buffers = Vec::with_capacity(lengths.len());
    for &len in lengths {
        let mut buffer = pool.pop().unwrap_or_default();
        buffer.resize(len, 0);
        if buffer.capacity() > len.saturating_mul(2).max(1) {
            buffer.shrink_to(len);
        }
        buffers.push(buffer);
    }
    buffers
}

fn release_worker_buffers(buffers: Vec<Vec<u32>>) {
    let mut pool = worker_buffer_pool()
        .lock()
        .expect("worker buffer pool mutex should not be poisoned");
    pool.extend(buffers);
}

fn with_render_buffer_rows<T>(rows: BufferRows, render: impl FnOnce() -> T) -> T {
    struct RenderBufferRowsReset<'a> {
        cell: &'a Cell<BufferRows>,
        previous: BufferRows,
    }

    impl Drop for RenderBufferRowsReset<'_> {
        fn drop(&mut self) {
            self.cell.set(self.previous);
        }
    }

    RENDER_BUFFER_ROWS.with(|cell| {
        let previous = cell.replace(rows);
        let _reset = RenderBufferRowsReset { cell, previous };
        render()
    })
}

fn current_render_buffer_rows() -> BufferRows {
    RENDER_BUFFER_ROWS.with(|cell| cell.get())
}

fn dirty_job_group_rows(jobs: &[DirtyRenderJob]) -> Option<BufferRows> {
    let first = jobs.first()?;
    let mut start = first.clip.y0.max(0.0) as usize;
    let mut end = first.clip.y1.max(0.0) as usize;
    for job in jobs.iter().skip(1) {
        start = start.min(job.clip.y0.max(0.0) as usize);
        end = end.max(job.clip.y1.max(0.0) as usize);
    }
    (start < end).then_some(BufferRows::new(start, end))
}

fn dirty_job_group_clip(jobs: &[DirtyRenderJob]) -> Option<ClipRect> {
    let mut clip = jobs.first()?.clip;
    for job in jobs.iter().skip(1) {
        clip = clip.union(job.clip);
    }
    Some(clip)
}

#[cfg(test)]
fn clear_shadow_mask_cache_for_tests() {
    shadow::clear_shadow_mask_cache_for_tests();
}

#[derive(Clone, Copy, Debug)]
pub struct FrameInfo {
    pub frame_index: u64,
    pub delta: Duration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewportSize {
    pub width: usize,
    pub height: usize,
}

impl ViewportSize {
    pub const fn new(width: usize, height: usize) -> Self {
        Self {
            width: if width == 0 { 1 } else { width },
            height: if height == 0 { 1 } else { height },
        }
    }
}

impl Default for ViewportSize {
    fn default() -> Self {
        Self::new(1, 1)
    }
}

fn drawable_viewport_size(width: usize, height: usize) -> Option<ViewportSize> {
    if width == 0 || height == 0 {
        None
    } else {
        Some(ViewportSize::new(width, height))
    }
}

#[derive(Clone, Debug)]
pub struct WindowConfig {
    pub title: String,
    pub width: usize,
    pub height: usize,
    pub clear_color: Color,
    pub frame_time: Duration,
    pub middle_button_auto_scroll: bool,
}

impl WindowConfig {
    pub fn new(title: impl Into<String>, width: usize, height: usize) -> Self {
        Self {
            title: title.into(),
            width,
            height,
            clear_color: Color::rgb(248, 250, 252),
            frame_time: Duration::from_millis(16),
            middle_button_auto_scroll: true,
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

pub trait SceneProvider {
    fn update(&mut self, frame: FrameInfo);

    fn scene(&self) -> &[RenderNode];

    fn set_viewport(&mut self, viewport: ViewportSize) {
        let _ = viewport;
    }

    fn capture_scene(&mut self) -> Vec<RenderNode> {
        self.scene().to_vec()
    }

    fn set_element_interaction(&mut self, interaction: ElementInteractionState) -> bool {
        let _ = interaction;
        false
    }
}

struct ClosureSceneProvider<F> {
    render_scene: F,
    scene: Vec<RenderNode>,
}

impl<F> ClosureSceneProvider<F> {
    fn new(render_scene: F) -> Self {
        Self {
            render_scene,
            scene: Vec::new(),
        }
    }
}

struct ViewportClosureSceneProvider<F> {
    render_scene: F,
    scene: Vec<RenderNode>,
    viewport: ViewportSize,
}

impl<F> ViewportClosureSceneProvider<F> {
    fn new(render_scene: F) -> Self {
        Self {
            render_scene,
            scene: Vec::new(),
            viewport: ViewportSize::default(),
        }
    }
}

impl<F> SceneProvider for ClosureSceneProvider<F>
where
    F: FnMut(FrameInfo) -> Vec<RenderNode>,
{
    fn update(&mut self, frame: FrameInfo) {
        self.scene = (self.render_scene)(frame);
    }

    fn scene(&self) -> &[RenderNode] {
        &self.scene
    }

    fn capture_scene(&mut self) -> Vec<RenderNode> {
        std::mem::take(&mut self.scene)
    }
}

impl<F> SceneProvider for ViewportClosureSceneProvider<F>
where
    F: FnMut(FrameInfo, ViewportSize) -> Vec<RenderNode>,
{
    fn update(&mut self, frame: FrameInfo) {
        self.scene = (self.render_scene)(frame, self.viewport);
    }

    fn scene(&self) -> &[RenderNode] {
        &self.scene
    }

    fn set_viewport(&mut self, viewport: ViewportSize) {
        self.viewport = viewport;
    }

    fn capture_scene(&mut self) -> Vec<RenderNode> {
        std::mem::take(&mut self.scene)
    }
}

pub fn run<F>(config: WindowConfig, render_scene: F) -> Result<()>
where
    F: FnMut(FrameInfo) -> Vec<RenderNode>,
{
    run_with_scene_provider(config, ClosureSceneProvider::new(render_scene))
}

pub fn run_with_viewport<F>(config: WindowConfig, render_scene: F) -> Result<()>
where
    F: FnMut(FrameInfo, ViewportSize) -> Vec<RenderNode>,
{
    run_with_scene_provider(config, ViewportClosureSceneProvider::new(render_scene))
}

pub fn run_with_scene_provider<P>(config: WindowConfig, mut scene_provider: P) -> Result<()>
where
    P: SceneProvider,
{
    let startup_begin = Instant::now();
    let initial_viewport = ViewportSize::new(config.width, config.height);
    scene_provider.set_viewport(initial_viewport);
    scene_provider.update(FrameInfo {
        frame_index: 0,
        delta: Duration::ZERO,
    });
    let mut initial_scene = scene_provider.capture_scene();
    let mut scrollbar_controller = scrollbar::ScrollbarController::default();
    scrollbar_controller.apply_to_scene(&mut initial_scene);
    let initial_indicator = scrollbar_controller.auto_scroll_indicator();

    let mut window = Window::new(&config.title, config.width, config.height, window_options())?;
    window.set_target_fps(frame_time_to_fps(config.frame_time));

    let mut buffer_width = config.width.max(1);
    let mut buffer_height = config.height.max(1);
    let mut buffer = vec![pack_rgb(config.clear_color); buffer_width * buffer_height];
    let initial_paint_start = Instant::now();
    let initial_paint_stats = render_to_buffer_internal(
        &initial_scene,
        &mut buffer,
        buffer_width,
        buffer_height,
        config.clear_color,
    );
    let initial_paint_us = duration_to_us(initial_paint_start.elapsed());
    if let Some(indicator) = initial_indicator {
        scrollbar::draw_auto_scroll_indicator(indicator, &mut buffer, buffer_width, buffer_height);
    }
    let initial_present_start = Instant::now();
    window.update_with_buffer(&buffer, buffer_width, buffer_height)?;
    let initial_present_us = duration_to_us(initial_present_start.elapsed());
    record_frame_timing_stats(FrameTimingStats {
        paint_us: initial_paint_us,
        present_us: initial_present_us,
        total_us: duration_to_us(startup_begin.elapsed()),
        render_workers: initial_paint_stats.workers,
        dirty_regions: initial_paint_stats.dirty_regions,
        dirty_jobs: initial_paint_stats.dirty_jobs,
        damage_pixels: initial_paint_stats.damage_pixels,
        painted_pixels: initial_paint_stats.painted_pixels,
        scene_passes: initial_paint_stats.scene_passes,
        paint_mode: initial_paint_stats.mode,
        paint_reason: initial_paint_stats.reason,
        ..FrameTimingStats::default()
    });

    let mut last_frame = Instant::now();
    let mut frame_index = 1_u64;
    let mut previous_left_down = false;
    let mut previous_right_down = false;
    let mut previous_middle_down = false;
    let mut previous_mouse_position = None;
    let mut suppress_left_pointer_until_release = false;
    let mut left_press_target: Option<ElementPath> = None;
    let mut last_click: Option<(Instant, ElementPath)> = None;
    let mut element_interaction = ElementInteractionState::default();
    let mut previous_presented_scene: Option<Vec<RenderNode>> = Some(initial_scene);
    let mut previous_presented_indicator: Option<scrollbar::AutoScrollIndicator> =
        initial_indicator;

    while window.is_open() && !window.is_key_down(Key::Escape) {
        let frame_begin = Instant::now();
        let mut frame_stats = FrameTimingStats::default();
        let now = Instant::now();
        let delta = now.saturating_duration_since(last_frame);
        last_frame = now;

        let left_down = window.get_mouse_down(MouseButton::Left);
        let right_down = window.get_mouse_down(MouseButton::Right);
        let middle_down = window.get_mouse_down(MouseButton::Middle);
        let suppress_pointer_for_system_drag = should_suspend_updates(
            left_down,
            window.is_key_down(Key::LeftSuper),
            window.is_key_down(Key::RightSuper),
        );
        if suppress_pointer_for_system_drag {
            suppress_left_pointer_until_release = true;
        } else if !left_down {
            suppress_left_pointer_until_release = false;
        }

        let interactive_left_down =
            left_down && !suppress_pointer_for_system_drag && !suppress_left_pointer_until_release;

        let (window_width, window_height) = window.get_size();
        let Some(viewport) = drawable_viewport_size(window_width, window_height) else {
            let _ = scrollbar_controller.cancel_middle_button_auto_scroll();
            suppress_left_pointer_until_release = false;
            previous_left_down = false;
            previous_right_down = false;
            previous_middle_down = false;
            previous_mouse_position = None;
            left_press_target = None;
            window.update();
            continue;
        };
        scene_provider.set_viewport(viewport);
        let frame = FrameInfo { frame_index, delta };
        let update_start = Instant::now();
        scene_provider.update(frame);
        frame_stats.update_us += duration_to_us(update_start.elapsed());

        let scene_prep_start = Instant::now();
        let mut scene = scene_provider.capture_scene();
        scrollbar_controller.apply_to_scene(&mut scene);
        let mouse_position = window.get_mouse_pos(MouseMode::Clamp);
        let previous_hovered = element_interaction.hovered.clone();
        let click_started = interactive_left_down && !previous_left_down;
        let right_press_started = right_down && !previous_right_down;
        let middle_click_started = middle_down && !previous_middle_down;
        let auto_scroll_canceled_click =
            click_started && scrollbar_controller.cancel_middle_button_auto_scroll();
        if config.middle_button_auto_scroll {
            if middle_click_started {
                let _ =
                    scrollbar_controller.toggle_middle_button_auto_scroll(&scene, mouse_position);
            }
        } else {
            let _ = scrollbar_controller.cancel_middle_button_auto_scroll();
        }
        let _ =
            scrollbar_controller.step_middle_button_auto_scroll(&mut scene, mouse_position, delta);
        scrollbar_controller.handle_wheel(&mut scene, mouse_position, window.get_scroll_wheel());
        let scrollbar_consumed_click = scrollbar_controller.handle_pointer(
            &mut scene,
            mouse_position,
            interactive_left_down,
            click_started,
        );
        let normal_click_started =
            click_started && !auto_scroll_canceled_click && !scrollbar_consumed_click;
        settle_element_interaction(
            &mut scene_provider,
            frame,
            &mut scene,
            &mut scrollbar_controller,
            mouse_position,
            interactive_left_down,
            normal_click_started,
            &mut element_interaction,
        );

        let current_hovered = element_interaction.hovered.clone();
        let mouse_moved = mouse_position != previous_mouse_position;
        let mut event_triggered_rerender = dispatch_hover_transition_events(
            &scene,
            previous_hovered.as_ref(),
            current_hovered.as_ref(),
        );

        if mouse_moved && let Some((mouse_x, mouse_y)) = mouse_position {
            event_triggered_rerender |=
                dispatch_mouse_event(&scene, mouse_x, mouse_y, MouseEventKind::MouseMove);
        }

        if normal_click_started {
            left_press_target = current_hovered.clone();
            if let Some((mouse_x, mouse_y)) = mouse_position {
                event_triggered_rerender |=
                    dispatch_mouse_event(&scene, mouse_x, mouse_y, MouseEventKind::MouseDown);
            }
        } else if click_started {
            left_press_target = None;
        }

        if previous_left_down && !interactive_left_down {
            if let Some((mouse_x, mouse_y)) = mouse_position {
                event_triggered_rerender |=
                    dispatch_mouse_event(&scene, mouse_x, mouse_y, MouseEventKind::MouseUp);
            }

            let release_target = current_hovered.clone();
            if let Some(click_target) = left_press_target.take() {
                if release_target.as_ref() == Some(&click_target) {
                    if let Some((mouse_x, mouse_y)) = mouse_position {
                        event_triggered_rerender |=
                            dispatch_mouse_event(&scene, mouse_x, mouse_y, MouseEventKind::Click);
                        let is_double_click =
                            last_click
                                .as_ref()
                                .is_some_and(|(instant, previous_target)| {
                                    *previous_target == click_target
                                        && now.saturating_duration_since(*instant)
                                            <= DOUBLE_CLICK_THRESHOLD
                                });
                        if is_double_click {
                            event_triggered_rerender |= dispatch_mouse_event(
                                &scene,
                                mouse_x,
                                mouse_y,
                                MouseEventKind::DblClick,
                            );
                        }
                    }
                    last_click = Some((now, click_target));
                } else {
                    last_click = None;
                }
            }
        }

        if right_press_started && let Some((mouse_x, mouse_y)) = mouse_position {
            event_triggered_rerender |=
                dispatch_mouse_event(&scene, mouse_x, mouse_y, MouseEventKind::MouseDown);
            event_triggered_rerender |=
                dispatch_mouse_event(&scene, mouse_x, mouse_y, MouseEventKind::ContextMenu);
        }

        if previous_right_down
            && !right_down
            && let Some((mouse_x, mouse_y)) = mouse_position
        {
            event_triggered_rerender |=
                dispatch_mouse_event(&scene, mouse_x, mouse_y, MouseEventKind::MouseUp);
        }

        if event_triggered_rerender {
            let rerender_start = Instant::now();
            scene_provider.update(frame);
            frame_stats.update_us += duration_to_us(rerender_start.elapsed());
            scene = scene_provider.capture_scene();
            scrollbar_controller.apply_to_scene(&mut scene);
            scrollbar_controller.handle_pointer(
                &mut scene,
                mouse_position,
                interactive_left_down,
                false,
            );
            settle_element_interaction(
                &mut scene_provider,
                frame,
                &mut scene,
                &mut scrollbar_controller,
                mouse_position,
                interactive_left_down,
                false,
                &mut element_interaction,
            );
        }
        frame_stats.scene_prep_us = duration_to_us(scene_prep_start.elapsed());

        let auto_scroll_indicator = scrollbar_controller.auto_scroll_indicator();

        let resized = buffer_width != viewport.width || buffer_height != viewport.height;
        resize_buffer(
            &mut buffer,
            &mut buffer_width,
            &mut buffer_height,
            viewport.width,
            viewport.height,
            config.clear_color,
        );
        if should_present_frame(
            previous_presented_scene.as_deref(),
            &scene,
            previous_presented_indicator,
            auto_scroll_indicator,
            resized,
        ) {
            let paint_start = Instant::now();
            let paint_stats = if resized {
                render_to_buffer_internal(
                    &scene,
                    &mut buffer,
                    buffer_width,
                    buffer_height,
                    config.clear_color,
                )
            } else if let Some(previous_scene) = previous_presented_scene.as_deref() {
                render_scene_update_internal(
                    previous_scene,
                    &scene,
                    &mut buffer,
                    buffer_width,
                    buffer_height,
                    config.clear_color,
                )
            } else {
                render_to_buffer_internal(
                    &scene,
                    &mut buffer,
                    buffer_width,
                    buffer_height,
                    config.clear_color,
                )
            };
            frame_stats.paint_us = duration_to_us(paint_start.elapsed());
            frame_stats.render_workers = paint_stats.workers;
            frame_stats.dirty_regions = paint_stats.dirty_regions;
            frame_stats.dirty_jobs = paint_stats.dirty_jobs;
            frame_stats.damage_pixels = paint_stats.damage_pixels;
            frame_stats.painted_pixels = paint_stats.painted_pixels;
            frame_stats.scene_passes = paint_stats.scene_passes;
            frame_stats.paint_mode = paint_stats.mode;
            frame_stats.paint_reason = paint_stats.reason;

            let present_start = Instant::now();
            redraw_auto_scroll_indicator_regions(
                previous_presented_indicator,
                auto_scroll_indicator,
                &scene,
                &mut buffer,
                buffer_width,
                buffer_height,
                config.clear_color,
            );
            window.update_with_buffer(&buffer, buffer_width, buffer_height)?;
            frame_stats.present_us = duration_to_us(present_start.elapsed());
            previous_presented_scene = Some(scene);
            previous_presented_indicator = auto_scroll_indicator;
        } else {
            let present_start = Instant::now();
            window.update();
            frame_stats.present_us = duration_to_us(present_start.elapsed());
        }

        previous_left_down = interactive_left_down;
        previous_right_down = right_down;
        previous_middle_down = middle_down;
        previous_mouse_position = mouse_position;
        frame_stats.total_us = duration_to_us(frame_begin.elapsed());
        record_frame_timing_stats(frame_stats);
        frame_index += 1;
    }

    Ok(())
}

fn window_options() -> WindowOptions {
    WindowOptions {
        resize: true,
        ..WindowOptions::default()
    }
}

pub fn render_to_buffer(
    scene: &[RenderNode],
    buffer: &mut [u32],
    width: usize,
    height: usize,
    clear_color: Color,
) {
    let _ = render_to_buffer_internal(scene, buffer, width, height, clear_color);
}

fn render_to_buffer_internal(
    scene: &[RenderNode],
    buffer: &mut [u32],
    width: usize,
    height: usize,
    clear_color: Color,
) -> PaintStats {
    render_to_buffer_internal_with_cached_bounds(scene, None, buffer, width, height, clear_color)
}

fn render_to_buffer_internal_with_cached_bounds(
    scene: &[RenderNode],
    cached_bounds: Option<&CachedSceneBounds>,
    buffer: &mut [u32],
    width: usize,
    height: usize,
    clear_color: Color,
) -> PaintStats {
    let worker_count = full_redraw_worker_count(width, height);
    if worker_count <= 1 {
        render_to_buffer_serial(scene, buffer, width, height, clear_color);
    } else {
        render_to_buffer_parallel(
            scene,
            cached_bounds,
            buffer,
            width,
            height,
            clear_color,
            worker_count,
        );
    }

    PaintStats {
        workers: worker_count.max(1),
        dirty_regions: 0,
        dirty_jobs: 0,
        damage_pixels: width.saturating_mul(height),
        painted_pixels: width.saturating_mul(height),
        scene_passes: worker_count.max(1),
        mode: FramePaintMode::Full,
        reason: FramePaintReason::FullRedraw,
    }
}

fn render_to_buffer_serial(
    scene: &[RenderNode],
    buffer: &mut [u32],
    width: usize,
    height: usize,
    clear_color: Color,
) {
    buffer.fill(pack_rgb(clear_color));
    let clip = ClipRect::full(width as f32, height as f32);

    for node in scene {
        draw_node(node, buffer, width, height, clip, CullMode::Layout);
    }
}

fn render_to_buffer_parallel(
    scene: &[RenderNode],
    cached_bounds: Option<&CachedSceneBounds>,
    buffer: &mut [u32],
    width: usize,
    height: usize,
    clear_color: Color,
    worker_count: usize,
) {
    let owned_bounds;
    let cached_bounds = if let Some(cached_bounds) = cached_bounds {
        cached_bounds
    } else {
        owned_bounds = cache_scene_subtree_bounds(scene);
        &owned_bounds
    };
    let clear = pack_rgb(clear_color);
    let band_count = worker_count.max(1).min(height.max(1));
    let rows_per_worker = height.div_ceil(band_count);
    let bands = (0..height)
        .step_by(rows_per_worker)
        .map(|row_start| BufferRows::new(row_start, (row_start + rows_per_worker).min(height)))
        .collect::<Vec<_>>();
    let band_root_indices = bands
        .iter()
        .map(|rows| {
            root_indices_intersecting_clip(
                &cached_bounds.roots,
                ClipRect {
                    x0: 0.0,
                    y0: rows.start as f32,
                    x1: width as f32,
                    y1: rows.end as f32,
                },
            )
        })
        .collect::<Vec<_>>();
    let worker_buffer_lengths = bands
        .iter()
        .map(|rows| rows.pixel_len(width))
        .collect::<Vec<_>>();
    let mut worker_buffers = acquire_worker_buffers(&worker_buffer_lengths);

    thread::scope(|scope| {
        let mut handles = Vec::new();
        for ((worker_buffer, rows), root_indices) in worker_buffers
            .iter_mut()
            .zip(bands.iter().copied())
            .zip(band_root_indices.iter())
        {
            let band_clip = ClipRect {
                x0: 0.0,
                y0: rows.start as f32,
                x1: width as f32,
                y1: rows.end as f32,
            };
            handles.push(scope.spawn(move || {
                with_render_buffer_rows(rows, || {
                    worker_buffer.fill(clear);
                    draw_cached_root_indices(
                        scene,
                        &cached_bounds.roots,
                        root_indices,
                        worker_buffer,
                        width,
                        height,
                        band_clip,
                    );
                });
            }));
        }

        for handle in handles {
            handle.join().expect("render worker should not panic");
        }
    });

    for (worker_buffer, rows) in worker_buffers.iter().zip(bands.iter().copied()) {
        let start = rows.start * width;
        let end = rows.end * width;
        buffer[start..end].copy_from_slice(worker_buffer);
    }

    release_worker_buffers(worker_buffers);
}

#[cfg_attr(not(test), allow(dead_code))]
fn render_scene_update(
    previous_scene: &[RenderNode],
    scene: &[RenderNode],
    buffer: &mut [u32],
    width: usize,
    height: usize,
    clear_color: Color,
) {
    let _ = render_scene_update_internal(previous_scene, scene, buffer, width, height, clear_color);
}

fn render_scene_update_internal(
    previous_scene: &[RenderNode],
    scene: &[RenderNode],
    buffer: &mut [u32],
    width: usize,
    height: usize,
    clear_color: Color,
) -> PaintStats {
    let scene_diff = prepare_scene_diff(previous_scene, scene);
    let mut dirty_regions = scene_diff.dirty_regions;
    if dirty_regions.is_empty() {
        return PaintStats::default();
    }

    coalesce_dirty_regions(&mut dirty_regions);
    let full_clip = ClipRect::full(width as f32, height as f32);
    let mut snapped_dirty_regions = dirty_regions
        .into_iter()
        .filter_map(|dirty_region| {
            dirty_region
                .intersect(full_clip)
                .and_then(|clip| snap_clip_to_pixel_grid(clip, width, height))
        })
        .collect::<Vec<_>>();
    if snapped_dirty_regions.is_empty() {
        return PaintStats::default();
    }

    coalesce_dirty_regions(&mut snapped_dirty_regions);
    let dirty_region_count = snapped_dirty_regions.len();
    let dirty_pixels = clip_rects_pixel_count(&snapped_dirty_regions, width, height);
    let planned_dirty_jobs = build_incremental_render_jobs(&snapped_dirty_regions, width, height);
    let incremental_worker_count = incremental_render_worker_count(&planned_dirty_jobs);
    let incremental_dirty_jobs = incremental_scene_pass_count(
        dirty_region_count,
        planned_dirty_jobs.len(),
        incremental_worker_count,
    );
    if let Some(reason) = should_full_redraw(
        dirty_region_count,
        dirty_pixels,
        scene_diff.current_bounds.node_count,
        incremental_dirty_jobs,
        width,
        height,
    ) {
        let mut stats = render_to_buffer_internal_with_cached_bounds(
            scene,
            Some(&scene_diff.current_bounds),
            buffer,
            width,
            height,
            clear_color,
        );
        stats.dirty_regions = dirty_region_count;
        stats.dirty_jobs = incremental_dirty_jobs;
        stats.damage_pixels = dirty_pixels;
        stats.reason = reason;
        return stats;
    }

    if incremental_worker_count > 1 {
        render_scene_update_parallel(
            scene,
            &scene_diff.current_bounds.roots,
            buffer,
            width,
            height,
            clear_color,
            &planned_dirty_jobs,
            incremental_worker_count,
        );
        return PaintStats {
            workers: incremental_worker_count,
            dirty_regions: dirty_region_count,
            dirty_jobs: incremental_dirty_jobs,
            damage_pixels: dirty_pixels,
            painted_pixels: dirty_pixels,
            scene_passes: incremental_dirty_jobs,
            mode: FramePaintMode::Incremental,
            reason: FramePaintReason::IncrementalDamage,
        };
    }

    for dirty_region in snapped_dirty_regions {
        clear_clip(buffer, width, height, dirty_region, clear_color);
        let root_indices =
            root_indices_intersecting_clip(&scene_diff.current_bounds.roots, dirty_region);
        draw_cached_root_indices(
            scene,
            &scene_diff.current_bounds.roots,
            &root_indices,
            buffer,
            width,
            height,
            dirty_region,
        );
    }

    PaintStats {
        workers: 1,
        dirty_regions: dirty_region_count,
        dirty_jobs: incremental_dirty_jobs,
        damage_pixels: dirty_pixels,
        painted_pixels: dirty_pixels,
        scene_passes: incremental_dirty_jobs,
        mode: FramePaintMode::Incremental,
        reason: FramePaintReason::IncrementalDamage,
    }
}

fn render_scene_update_parallel(
    scene: &[RenderNode],
    cached_bounds: &[CachedSubtreeBounds],
    buffer: &mut [u32],
    width: usize,
    height: usize,
    clear_color: Color,
    dirty_jobs: &[DirtyRenderJob],
    worker_count: usize,
) {
    let worker_count = worker_count.max(1).min(dirty_jobs.len().max(1));
    let job_groups = distribute_dirty_render_jobs(dirty_jobs, worker_count);
    let group_rows = job_groups
        .iter()
        .map(|jobs| {
            dirty_job_group_rows(jobs)
                .expect("dirty render groups should only contain non-empty row spans")
        })
        .collect::<Vec<_>>();
    let group_root_indices = job_groups
        .iter()
        .map(|jobs| {
            let clip = dirty_job_group_clip(jobs)
                .expect("dirty render groups should only contain at least one clip");
            root_indices_intersecting_clip(cached_bounds, clip)
        })
        .collect::<Vec<_>>();
    let clear = pack_rgb(clear_color);
    let worker_buffer_lengths = group_rows
        .iter()
        .map(|rows| rows.pixel_len(width))
        .collect::<Vec<_>>();
    let mut worker_buffers = acquire_worker_buffers(&worker_buffer_lengths);

    thread::scope(|scope| {
        let mut handles = Vec::new();
        for (((worker_buffer, jobs), rows), root_indices) in worker_buffers
            .iter_mut()
            .zip(job_groups.iter())
            .zip(group_rows.iter().copied())
            .zip(group_root_indices.iter())
        {
            handles.push(scope.spawn(move || {
                with_render_buffer_rows(rows, || {
                    for job in jobs {
                        clear_clip_packed(worker_buffer, width, height, job.clip, clear);
                        draw_cached_root_indices(
                            scene,
                            cached_bounds,
                            root_indices,
                            worker_buffer,
                            width,
                            height,
                            job.clip,
                        );
                    }
                });
            }));
        }

        for handle in handles {
            handle
                .join()
                .expect("incremental render worker should not panic");
        }
    });

    for ((worker_buffer, jobs), rows) in worker_buffers
        .iter()
        .zip(job_groups.iter())
        .zip(group_rows.iter().copied())
    {
        for job in jobs {
            let Some((x0, y0, x1, y1)) = clip_pixel_bounds(job.clip, width, height) else {
                continue;
            };

            for y in y0..y1 {
                let destination_row_start = y as usize * width;
                let source_row_start = (y as usize - rows.start) * width;
                let destination_start = destination_row_start + x0 as usize;
                let destination_end = destination_row_start + x1 as usize;
                let source_start = source_row_start + x0 as usize;
                let source_end = source_row_start + x1 as usize;
                buffer[destination_start..destination_end]
                    .copy_from_slice(&worker_buffer[source_start..source_end]);
            }
        }
    }

    release_worker_buffers(worker_buffers);
}

fn build_incremental_render_jobs(
    dirty_regions: &[ClipRect],
    width: usize,
    height: usize,
) -> Vec<DirtyRenderJob> {
    let mut jobs = Vec::new();

    for &dirty_region in dirty_regions {
        let Some((x0, y0, x1, y1)) = clip_pixel_bounds(dirty_region, width, height) else {
            continue;
        };

        let band_height = MIN_PARALLEL_RENDER_ROWS_PER_WORKER.max(1);
        let region_width = (x1 - x0).max(0) as usize;
        for band_y0 in (y0 as usize..y1 as usize).step_by(band_height) {
            let band_y1 = (band_y0 + band_height).min(y1 as usize);
            let pixel_count = region_width.saturating_mul(band_y1.saturating_sub(band_y0));
            if pixel_count == 0 {
                continue;
            }
            jobs.push(DirtyRenderJob {
                clip: ClipRect {
                    x0: x0 as f32,
                    y0: band_y0 as f32,
                    x1: x1 as f32,
                    y1: band_y1 as f32,
                },
                pixel_count,
            });
        }
    }

    jobs
}

fn distribute_dirty_render_jobs(
    dirty_jobs: &[DirtyRenderJob],
    worker_count: usize,
) -> Vec<Vec<DirtyRenderJob>> {
    let worker_count = worker_count.max(1).min(dirty_jobs.len().max(1));
    let mut groups = vec![Vec::new(); worker_count];
    let mut loads = vec![0_usize; worker_count];
    let mut jobs = dirty_jobs.to_vec();
    jobs.sort_by(|left, right| right.pixel_count.cmp(&left.pixel_count));

    for job in jobs {
        let Some((target_index, _)) = loads.iter().enumerate().min_by_key(|(_, load)| **load)
        else {
            break;
        };
        loads[target_index] = loads[target_index].saturating_add(job.pixel_count);
        groups[target_index].push(job);
    }

    groups
        .into_iter()
        .filter(|group| !group.is_empty())
        .collect()
}

fn incremental_render_worker_count(dirty_jobs: &[DirtyRenderJob]) -> usize {
    if dirty_jobs.len() <= 1 {
        return 1;
    }

    let total_dirty_pixels = dirty_jobs.iter().map(|job| job.pixel_count).sum::<usize>();
    if total_dirty_pixels < MIN_INCREMENTAL_PIXELS_PER_WORKER {
        return 1;
    }

    let max_workers_from_pixels = total_dirty_pixels
        .div_ceil(MIN_INCREMENTAL_PIXELS_PER_WORKER)
        .max(1);
    thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
        .min(MAX_RENDER_WORKERS)
        .min(dirty_jobs.len())
        .min(max_workers_from_pixels)
        .max(1)
}

fn draw_cached_root_indices(
    scene: &[RenderNode],
    cached_bounds: &[CachedSubtreeBounds],
    root_indices: &[usize],
    buffer: &mut [u32],
    width: usize,
    height: usize,
    clip: ClipRect,
) {
    for &root_index in root_indices {
        let Some(node) = scene.get(root_index) else {
            continue;
        };
        let Some(bounds) = cached_bounds.get(root_index) else {
            continue;
        };
        draw_node_with_cached_bounds(node, bounds, buffer, width, height, clip);
    }
}

fn incremental_scene_pass_count(
    dirty_region_count: usize,
    dirty_job_count: usize,
    worker_count: usize,
) -> usize {
    if worker_count > 1 {
        dirty_job_count.max(1)
    } else {
        dirty_region_count.max(1)
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

fn should_present_frame(
    previous_scene: Option<&[RenderNode]>,
    scene: &[RenderNode],
    previous_indicator: Option<scrollbar::AutoScrollIndicator>,
    indicator: Option<scrollbar::AutoScrollIndicator>,
    resized: bool,
) -> bool {
    should_present_scene(previous_scene, scene, resized) || previous_indicator != indicator
}

fn redraw_auto_scroll_indicator_regions(
    previous_indicator: Option<scrollbar::AutoScrollIndicator>,
    indicator: Option<scrollbar::AutoScrollIndicator>,
    scene: &[RenderNode],
    buffer: &mut [u32],
    width: usize,
    height: usize,
    clear_color: Color,
) {
    let mut bounds = Vec::new();
    if let Some(previous_indicator) = previous_indicator {
        bounds.push(scrollbar::auto_scroll_indicator_bounds(previous_indicator));
    }
    if let Some(indicator) = indicator {
        let indicator_bounds = scrollbar::auto_scroll_indicator_bounds(indicator);
        if !bounds.iter().any(|existing| *existing == indicator_bounds) {
            bounds.push(indicator_bounds);
        }
    }

    let full_clip = ClipRect::full(width as f32, height as f32);
    for bounds in bounds {
        let Some(clip) = full_clip
            .intersect(layout_clip(bounds))
            .and_then(|clip| snap_clip_to_pixel_grid(clip, width, height))
        else {
            continue;
        };
        clear_clip(buffer, width, height, clip, clear_color);
        for node in scene {
            draw_node(node, buffer, width, height, clip, CullMode::Subtree);
        }
    }

    if let Some(indicator) = indicator {
        scrollbar::draw_auto_scroll_indicator(indicator, buffer, width, height);
    }
}

fn scenes_match_visuals(left: &[RenderNode], right: &[RenderNode]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right.iter())
            .all(|(left, right)| render_nodes_match_visuals(left, right))
}

fn render_nodes_match_visuals(left: &RenderNode, right: &RenderNode) -> bool {
    render_nodes_match_own_visuals(left, right)
        && scenes_match_visuals(&left.children, &right.children)
}

fn render_nodes_match_own_visuals(left: &RenderNode, right: &RenderNode) -> bool {
    left.kind == right.kind
        && left.layout == right.layout
        && left.style == right.style
        && left.content_inset == right.content_inset
        && left.scrollbars == right.scrollbars
}

fn should_suspend_updates(left_down: bool, left_super_down: bool, right_super_down: bool) -> bool {
    left_down && (left_super_down || right_super_down)
}

#[derive(Clone, Copy)]
enum CullMode {
    Layout,
    Subtree,
}

#[derive(Clone, Debug)]
struct CachedSceneBounds {
    roots: Vec<CachedSubtreeBounds>,
    node_count: usize,
}

#[derive(Clone, Debug)]
struct CachedSubtreeBounds {
    own_bounds: Option<ClipRect>,
    bounds: Option<ClipRect>,
    children: Vec<CachedSubtreeBounds>,
}

fn root_indices_intersecting_clip(
    cached_bounds: &[CachedSubtreeBounds],
    clip: ClipRect,
) -> Vec<usize> {
    cached_bounds
        .iter()
        .enumerate()
        .filter_map(|(index, bounds)| {
            bounds
                .bounds
                .and_then(|node_bounds| clip.intersect(node_bounds))
                .map(|_| index)
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq)]
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

    fn union(self, other: Self) -> Self {
        Self {
            x0: self.x0.min(other.x0),
            y0: self.y0.min(other.y0),
            x1: self.x1.max(other.x1),
            y1: self.y1.max(other.y1),
        }
    }

    fn overlaps_or_touches(self, other: Self) -> bool {
        self.x0 <= other.x1 && self.x1 >= other.x0 && self.y0 <= other.y1 && self.y1 >= other.y0
    }

    fn area(self) -> f32 {
        if self.is_empty() {
            0.0
        } else {
            (self.x1 - self.x0) * (self.y1 - self.y0)
        }
    }

    fn is_empty(self) -> bool {
        self.x0 >= self.x1 || self.y0 >= self.y1
    }
}

fn draw_node(
    node: &RenderNode,
    buffer: &mut [u32],
    width: usize,
    height: usize,
    clip: ClipRect,
    cull_mode: CullMode,
) {
    if clip.is_empty() || !node_intersects_clip(node, clip, cull_mode) {
        return;
    }

    draw_node_contents(node, None, buffer, width, height, clip, cull_mode);
}

fn draw_node_with_cached_bounds(
    node: &RenderNode,
    cached_bounds: &CachedSubtreeBounds,
    buffer: &mut [u32],
    width: usize,
    height: usize,
    clip: ClipRect,
) {
    if clip.is_empty()
        || cached_bounds
            .bounds
            .and_then(|bounds| clip.intersect(bounds))
            .is_none()
    {
        return;
    }

    draw_node_contents(
        node,
        Some(cached_bounds),
        buffer,
        width,
        height,
        clip,
        CullMode::Subtree,
    );
}

fn draw_node_contents(
    node: &RenderNode,
    cached_bounds: Option<&CachedSubtreeBounds>,
    buffer: &mut [u32],
    width: usize,
    height: usize,
    clip: ClipRect,
    cull_mode: CullMode,
) {
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

    if !matches!(node.kind, RenderKind::Text(_)) {
        for shadow in &node.style.filter_drop_shadows {
            draw_shadow_effect(
                buffer,
                width,
                height,
                node.layout,
                node.style.corner_radius,
                *shadow,
                node.style.foreground,
                clip,
            );
        }
    }

    draw_background_and_border(node, buffer, width, height, clip);

    if let RenderKind::Text(content) = &node.kind {
        let text_layout = scrollbar::text_layout(node);
        let text_clip = scrollbar::text_clip(node, clip);
        fonts::draw_text(
            buffer,
            width,
            height,
            text_layout,
            content,
            node.text_layout.as_ref(),
            &node.style,
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

    if let Some(cached_bounds) = cached_bounds {
        for (child, child_bounds) in node.children.iter().zip(&cached_bounds.children) {
            draw_node_with_cached_bounds(child, child_bounds, buffer, width, height, child_clip);
        }
    } else {
        for child in &node.children {
            draw_node(child, buffer, width, height, child_clip, cull_mode);
        }
    }

    scrollbar::draw_scrollbars(node, buffer, width, height, clip);
}

fn node_intersects_clip(node: &RenderNode, clip: ClipRect, cull_mode: CullMode) -> bool {
    match cull_mode {
        CullMode::Layout => clip.intersect(layout_clip(node.layout)).is_some(),
        CullMode::Subtree => subtree_visual_bounds(node)
            .and_then(|bounds| clip.intersect(bounds))
            .is_some(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MouseEventKind {
    Click,
    ContextMenu,
    DblClick,
    MouseDown,
    MouseEnter,
    MouseLeave,
    MouseMove,
    MouseOut,
    MouseOver,
    MouseUp,
}

#[cfg_attr(not(test), allow(dead_code))]
fn dispatch_click(scene: &[RenderNode], x: f32, y: f32) -> bool {
    dispatch_mouse_event(scene, x, y, MouseEventKind::Click)
}

fn dispatch_mouse_event(scene: &[RenderNode], x: f32, y: f32, event: MouseEventKind) -> bool {
    let Some(handler) = hit_test_scene_for_event(scene, x, y, event) else {
        return false;
    };

    handler();
    true
}

fn dispatch_mouse_event_at_path(
    scene: &[RenderNode],
    path: &ElementPath,
    event: MouseEventKind,
) -> bool {
    let Some(handler) = find_handler_for_path(scene, path, event) else {
        return false;
    };

    handler();
    true
}

fn dispatch_hover_transition_events(
    scene: &[RenderNode],
    previous_hovered: Option<&ElementPath>,
    hovered: Option<&ElementPath>,
) -> bool {
    if previous_hovered == hovered {
        return false;
    }

    let previous_chain = previous_hovered.map(element_path_chain).unwrap_or_default();
    let hovered_chain = hovered.map(element_path_chain).unwrap_or_default();
    let shared_prefix_len = shared_path_prefix_len(&previous_chain, &hovered_chain);
    let mut triggered = false;

    if let Some(previous_hovered) = previous_hovered {
        triggered |=
            dispatch_mouse_event_at_path(scene, previous_hovered, MouseEventKind::MouseOut);
    }
    for path in previous_chain[shared_prefix_len..].iter().rev() {
        triggered |= dispatch_mouse_event_at_path(scene, path, MouseEventKind::MouseLeave);
    }

    if let Some(hovered) = hovered {
        triggered |= dispatch_mouse_event_at_path(scene, hovered, MouseEventKind::MouseOver);
    }
    for path in &hovered_chain[shared_prefix_len..] {
        triggered |= dispatch_mouse_event_at_path(scene, path, MouseEventKind::MouseEnter);
    }

    triggered
}

fn hit_test_scene_for_event(
    scene: &[RenderNode],
    x: f32,
    y: f32,
    event: MouseEventKind,
) -> Option<EventHandler> {
    scene
        .iter()
        .rev()
        .find_map(|node| hit_test_node_for_event(node, x, y, ClipRect::unbounded(), event))
}

fn hit_test_element_path(scene: &[RenderNode], x: f32, y: f32) -> Option<ElementPath> {
    scene
        .iter()
        .enumerate()
        .rev()
        .find_map(|(root_index, node)| {
            hit_test_element_path_node(
                node,
                x,
                y,
                ClipRect::unbounded(),
                &ElementPath::root(root_index),
            )
        })
}

fn hit_test_node_for_event(
    node: &RenderNode,
    x: f32,
    y: f32,
    clip: ClipRect,
    event: MouseEventKind,
) -> Option<EventHandler> {
    if !clip.contains(x, y) || !layout_contains(node.layout, x, y) {
        return None;
    }

    let child_clip = if node.style.overflow.clips_any_axis() {
        clip.intersect(layout_clip(node.layout))?
    } else {
        clip
    };

    for child in node.children.iter().rev() {
        if let Some(handler) = hit_test_node_for_event(child, x, y, child_clip, event) {
            return Some(handler);
        }
    }

    event_handler(node, event)
}

fn find_handler_for_path(
    scene: &[RenderNode],
    path: &ElementPath,
    event: MouseEventKind,
) -> Option<EventHandler> {
    scene
        .iter()
        .find_map(|node| find_handler_for_path_node(node, path, event))
}

fn find_handler_for_path_node(
    node: &RenderNode,
    path: &ElementPath,
    event: MouseEventKind,
) -> Option<EventHandler> {
    if node.element_path.as_ref() == Some(path)
        && let Some(handler) = event_handler(node, event)
    {
        return Some(handler);
    }

    node.children
        .iter()
        .find_map(|child| find_handler_for_path_node(child, path, event))
}

fn event_handler(node: &RenderNode, event: MouseEventKind) -> Option<EventHandler> {
    match event {
        MouseEventKind::Click => node.handlers.click,
        MouseEventKind::ContextMenu => node.handlers.contextmenu,
        MouseEventKind::DblClick => node.handlers.dblclick,
        MouseEventKind::MouseDown => node.handlers.mousedown,
        MouseEventKind::MouseEnter => node.handlers.mouseenter,
        MouseEventKind::MouseLeave => node.handlers.mouseleave,
        MouseEventKind::MouseMove => node.handlers.mousemove,
        MouseEventKind::MouseOut => node.handlers.mouseout,
        MouseEventKind::MouseOver => node.handlers.mouseover,
        MouseEventKind::MouseUp => node.handlers.mouseup,
    }
}

fn element_path_chain(path: &ElementPath) -> Vec<ElementPath> {
    let mut chain = Vec::with_capacity(path.children.len() + 1);
    let mut current = ElementPath::root(path.root);
    chain.push(current.clone());

    for &child_index in &path.children {
        current = current.with_child(child_index);
        chain.push(current.clone());
    }

    chain
}

fn shared_path_prefix_len(left: &[ElementPath], right: &[ElementPath]) -> usize {
    left.iter()
        .zip(right)
        .take_while(|(left, right)| left == right)
        .count()
}

fn hit_test_element_path_node(
    node: &RenderNode,
    x: f32,
    y: f32,
    clip: ClipRect,
    path: &ElementPath,
) -> Option<ElementPath> {
    if !clip.contains(x, y) || !layout_contains(node.layout, x, y) {
        return None;
    }

    let child_clip = if node.style.overflow.clips_any_axis() {
        clip.intersect(layout_clip(node.layout))?
    } else {
        clip
    };

    for (index, child) in node.children.iter().enumerate().rev() {
        let child_path = path.with_child(index);
        if let Some(hit) = hit_test_element_path_node(child, x, y, child_clip, &child_path) {
            return Some(hit);
        }
    }

    node.element_path.clone().or_else(|| Some(path.clone()))
}

fn settle_element_interaction<P>(
    scene_provider: &mut P,
    frame: FrameInfo,
    scene: &mut Vec<RenderNode>,
    scrollbar_controller: &mut scrollbar::ScrollbarController,
    mouse_position: Option<(f32, f32)>,
    interactive_left_down: bool,
    press_started: bool,
    interaction: &mut ElementInteractionState,
) where
    P: SceneProvider,
{
    let mut press_started = press_started;

    for _ in 0..4 {
        let hovered = mouse_position
            .and_then(|(mouse_x, mouse_y)| hit_test_element_path(scene, mouse_x, mouse_y));
        let next = next_element_interaction_state(
            interaction,
            hovered,
            interactive_left_down,
            press_started,
        );
        press_started = false;

        if next == *interaction {
            break;
        }

        *interaction = next.clone();
        if !scene_provider.set_element_interaction(next) {
            break;
        }

        scene_provider.update(frame);
        *scene = scene_provider.capture_scene();
        scrollbar_controller.apply_to_scene(scene);
        scrollbar_controller.handle_pointer(scene, mouse_position, interactive_left_down, false);
    }
}

fn next_element_interaction_state(
    previous: &ElementInteractionState,
    hovered: Option<ElementPath>,
    left_down: bool,
    press_started: bool,
) -> ElementInteractionState {
    let active = if press_started {
        hovered.clone()
    } else if left_down {
        previous.active.clone()
    } else {
        None
    };

    ElementInteractionState { hovered, active }
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
        if !draw_axis_aligned_opaque_ring(
            buffer,
            width,
            height,
            node.layout,
            node.style.corner_radius,
            Some((inner_layout, inner_radius)),
            node.style.border.color,
            clip,
        ) {
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
    }

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

    if let Some(background) = node.style.background {
        if !draw_axis_aligned_opaque_rect(
            buffer,
            width,
            height,
            fill_layout,
            fill_radius,
            background,
            clip,
        ) {
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

    for layer in node.style.background_layers.iter().rev() {
        draw_background_layer(buffer, width, height, fill_layout, fill_radius, layer, clip);
    }
}

struct SceneDiff {
    dirty_regions: Vec<ClipRect>,
    current_bounds: CachedSceneBounds,
}

fn prepare_scene_diff(previous_scene: &[RenderNode], scene: &[RenderNode]) -> SceneDiff {
    let previous_bounds = cache_scene_subtree_bounds(previous_scene);
    let current_bounds = cache_scene_subtree_bounds(scene);
    let mut dirty_regions = Vec::new();
    let _ = collect_scene_dirty_regions(
        previous_scene,
        &previous_bounds.roots,
        scene,
        &current_bounds.roots,
        &mut dirty_regions,
    );
    SceneDiff {
        dirty_regions,
        current_bounds,
    }
}

#[cfg_attr(not(test), allow(dead_code))]
fn dirty_regions_between_scenes(
    previous_scene: &[RenderNode],
    scene: &[RenderNode],
) -> Vec<ClipRect> {
    prepare_scene_diff(previous_scene, scene).dirty_regions
}

fn collect_scene_dirty_regions(
    previous_scene: &[RenderNode],
    previous_bounds: &[CachedSubtreeBounds],
    scene: &[RenderNode],
    current_bounds: &[CachedSubtreeBounds],
    dirty_regions: &mut Vec<ClipRect>,
) -> bool {
    let count = previous_scene.len().max(scene.len());
    let mut matched = previous_scene.len() == scene.len();

    for index in 0..count {
        match (
            previous_scene.get(index),
            previous_bounds.get(index),
            scene.get(index),
            current_bounds.get(index),
        ) {
            (Some(previous), Some(previous_bounds), Some(current), Some(current_bounds)) => {
                matched &= collect_node_dirty_regions(
                    previous,
                    previous_bounds,
                    current,
                    current_bounds,
                    dirty_regions,
                );
            }
            (Some(_previous), Some(previous_bounds), None, None) => {
                push_cached_subtree_dirty_region(previous_bounds, dirty_regions);
                matched = false;
            }
            (None, None, Some(_current), Some(current_bounds)) => {
                push_cached_subtree_dirty_region(current_bounds, dirty_regions);
                matched = false;
            }
            _ => matched = false,
        }
    }

    matched
}

fn collect_node_dirty_regions(
    previous: &RenderNode,
    previous_bounds: &CachedSubtreeBounds,
    current: &RenderNode,
    current_bounds: &CachedSubtreeBounds,
    dirty_regions: &mut Vec<ClipRect>,
) -> bool {
    let own_matches = render_nodes_match_own_visuals(previous, current);
    let can_tighten_own_dirty = can_tighten_own_dirty_region(previous, current);

    if own_matches {
        let start_len = dirty_regions.len();
        let children_match = collect_scene_dirty_regions(
            &previous.children,
            &previous_bounds.children,
            &current.children,
            &current_bounds.children,
            dirty_regions,
        );
        if children_match {
            return true;
        }
        maybe_collapse_branch_dirty_regions(
            previous_bounds.bounds,
            current_bounds.bounds,
            dirty_regions,
            start_len,
        );
        return false;
    }

    if !can_tighten_own_dirty {
        push_dirty_region(
            union_optional_bounds(previous_bounds.bounds, current_bounds.bounds),
            dirty_regions,
        );
        return false;
    }

    let start_len = dirty_regions.len();
    let children_match = collect_scene_dirty_regions(
        &previous.children,
        &previous_bounds.children,
        &current.children,
        &current_bounds.children,
        dirty_regions,
    );
    push_dirty_region(
        union_optional_bounds(previous_bounds.own_bounds, current_bounds.own_bounds),
        dirty_regions,
    );
    if !children_match {
        maybe_collapse_branch_dirty_regions(
            previous_bounds.bounds,
            current_bounds.bounds,
            dirty_regions,
            start_len,
        );
    }
    false
}

fn push_cached_subtree_dirty_region(
    cached_bounds: &CachedSubtreeBounds,
    dirty_regions: &mut Vec<ClipRect>,
) {
    push_dirty_region(cached_bounds.bounds, dirty_regions);
}

fn can_tighten_own_dirty_region(previous: &RenderNode, current: &RenderNode) -> bool {
    previous.kind == current.kind
        && previous.layout == current.layout
        && previous.content_inset == current.content_inset
        && previous.scrollbars == current.scrollbars
        && previous.style.overflow == current.style.overflow
}

fn push_dirty_region(region: Option<ClipRect>, dirty_regions: &mut Vec<ClipRect>) {
    if let Some(region) = region
        && !region.is_empty()
    {
        dirty_regions.push(region);
    }
}

fn maybe_collapse_branch_dirty_regions(
    previous_bounds: Option<ClipRect>,
    current_bounds: Option<ClipRect>,
    dirty_regions: &mut Vec<ClipRect>,
    start_len: usize,
) {
    let descendant_count = dirty_regions.len().saturating_sub(start_len);
    if descendant_count < DIRTY_BRANCH_COLLAPSE_THRESHOLD {
        return;
    }

    let Some(branch_bounds) = union_optional_bounds(previous_bounds, current_bounds) else {
        return;
    };

    let descendant_area: f32 = dirty_regions[start_len..]
        .iter()
        .filter_map(|region| region.intersect(branch_bounds))
        .map(ClipRect::area)
        .sum();

    if descendant_area <= f32::EPSILON {
        return;
    }

    if branch_bounds.area() > descendant_area * DIRTY_BRANCH_COLLAPSE_MAX_AREA_RATIO {
        return;
    }

    dirty_regions.truncate(start_len);
    dirty_regions.push(branch_bounds);
}

fn coalesce_dirty_regions(dirty_regions: &mut Vec<ClipRect>) {
    let mut index = 0;
    while index < dirty_regions.len() {
        let mut merged = false;
        let mut other_index = index + 1;
        while other_index < dirty_regions.len() {
            if should_merge_dirty_regions(dirty_regions[index], dirty_regions[other_index]) {
                dirty_regions[index] = dirty_regions[index].union(dirty_regions[other_index]);
                dirty_regions.swap_remove(other_index);
                merged = true;
            } else {
                other_index += 1;
            }
        }

        if !merged {
            index += 1;
        }
    }
}

fn should_merge_dirty_regions(left: ClipRect, right: ClipRect) -> bool {
    if !left.overlaps_or_touches(right) {
        return false;
    }

    let union = left.union(right);
    let combined_area = left.area() + right.area();
    if combined_area <= f32::EPSILON {
        return false;
    }

    union.area() <= combined_area * DIRTY_REGION_COALESCE_MAX_EXPANSION_RATIO
}

fn should_full_redraw(
    dirty_region_count: usize,
    dirty_pixels: usize,
    scene_node_count: usize,
    incremental_scene_passes: usize,
    width: usize,
    height: usize,
) -> Option<FramePaintReason> {
    if dirty_region_count > MAX_INCREMENTAL_DIRTY_REGIONS {
        return Some(FramePaintReason::DirtyRegionLimit);
    }

    let full_pixels = width.saturating_mul(height);
    if dirty_pixels > (full_pixels as f32 * MAX_INCREMENTAL_DIRTY_AREA_RATIO) as usize {
        return Some(FramePaintReason::DirtyAreaLimit);
    }

    let traversal_cost = scene_node_count
        .max(1)
        .saturating_mul(SCENE_TRAVERSAL_COST_PER_NODE);
    let incremental_cost = dirty_pixels
        .saturating_mul(2)
        .saturating_add(traversal_cost.saturating_mul(incremental_scene_passes.max(1)));
    let full_cost = full_pixels.saturating_mul(2).saturating_add(
        traversal_cost.saturating_mul(full_redraw_worker_count(width, height).max(1)),
    );
    if incremental_cost >= full_cost {
        return Some(FramePaintReason::FragmentedDamage);
    }

    None
}

fn clip_rects_pixel_count(clips: &[ClipRect], width: usize, height: usize) -> usize {
    clips
        .iter()
        .filter_map(|clip| clip_pixel_bounds(*clip, width, height))
        .map(|(x0, y0, x1, y1)| (x1 - x0).max(0).saturating_mul((y1 - y0).max(0)) as usize)
        .sum()
}

fn subtree_visual_bounds(node: &RenderNode) -> Option<ClipRect> {
    let mut bounds = node_visual_bounds(node);
    let parent_clip = non_empty_layout_clip(node.layout);

    for child in &node.children {
        let Some(mut child_bounds) = subtree_visual_bounds(child) else {
            continue;
        };

        if node.style.overflow.clips_any_axis() {
            let Some(clip) = parent_clip else {
                continue;
            };
            let Some(clipped_bounds) = child_bounds.intersect(clip) else {
                continue;
            };
            child_bounds = clipped_bounds;
        }

        bounds = union_optional_bounds(bounds, Some(child_bounds));
    }

    bounds
}

fn cache_scene_subtree_bounds(scene: &[RenderNode]) -> CachedSceneBounds {
    let mut node_count = 0;
    let roots = scene
        .iter()
        .map(|node| cache_subtree_bounds(node, &mut node_count))
        .collect();
    CachedSceneBounds { roots, node_count }
}

fn cache_subtree_bounds(node: &RenderNode, node_count: &mut usize) -> CachedSubtreeBounds {
    *node_count = node_count.saturating_add(1);
    let own_bounds = node_visual_bounds(node);
    let mut bounds = own_bounds;
    let parent_clip = non_empty_layout_clip(node.layout);
    let mut children = Vec::with_capacity(node.children.len());

    for child in &node.children {
        let mut cached_child = cache_subtree_bounds(child, node_count);
        if node.style.overflow.clips_any_axis() {
            cached_child.bounds = cached_child
                .bounds
                .and_then(|child_bounds| parent_clip.and_then(|clip| child_bounds.intersect(clip)));
        }
        bounds = union_optional_bounds(bounds, cached_child.bounds);
        children.push(cached_child);
    }

    CachedSubtreeBounds {
        own_bounds,
        bounds,
        children,
    }
}

fn node_visual_bounds(node: &RenderNode) -> Option<ClipRect> {
    let mut bounds = non_empty_layout_clip(node.layout);

    if matches!(node.kind, RenderKind::Text(_)) && node.style.text_stroke.width > 0.0 {
        bounds = union_optional_bounds(
            bounds,
            text_stroke_bounds(node.layout, node.style.text_stroke),
        );
    }

    for shadow in &node.style.shadows {
        bounds = union_optional_bounds(bounds, shadow_bounds(node.layout, *shadow));
    }

    match &node.kind {
        RenderKind::Text(_) => {
            for shadow in &node.style.text_shadows {
                bounds = union_optional_bounds(bounds, shadow_effect_bounds(node.layout, *shadow));
            }
            for shadow in &node.style.filter_drop_shadows {
                bounds = union_optional_bounds(bounds, shadow_effect_bounds(node.layout, *shadow));
            }
        }
        RenderKind::Container => {
            for shadow in &node.style.filter_drop_shadows {
                bounds = union_optional_bounds(bounds, shadow_effect_bounds(node.layout, *shadow));
            }
        }
    }

    bounds
}

fn clear_clip(buffer: &mut [u32], width: usize, height: usize, clip: ClipRect, clear_color: Color) {
    clear_clip_packed(buffer, width, height, clip, pack_rgb(clear_color));
}

fn clear_clip_packed(buffer: &mut [u32], width: usize, height: usize, clip: ClipRect, clear: u32) {
    let Some((x0, y0, x1, y1)) = clip_pixel_bounds(clip, width, height) else {
        return;
    };
    let rows = current_render_buffer_rows();
    let row_start = (y0 as usize).max(rows.start);
    let row_end = (y1 as usize).min(rows.end);

    for y in row_start..row_end {
        let local_row_start = (y - rows.start) * width;
        let start = local_row_start + x0 as usize;
        let end = local_row_start + x1 as usize;
        buffer[start..end].fill(clear);
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

fn full_redraw_worker_count(width: usize, height: usize) -> usize {
    let total_pixels = width.saturating_mul(height);
    if total_pixels < MIN_PARALLEL_RENDER_PIXELS {
        return 1;
    }

    let max_workers_from_rows = height.div_ceil(MIN_PARALLEL_RENDER_ROWS_PER_WORKER).max(1);
    thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
        .min(MAX_RENDER_WORKERS)
        .min(max_workers_from_rows)
        .max(1)
}

fn duration_to_us(duration: Duration) -> u64 {
    duration.as_micros().min(u128::from(u64::MAX)) as u64
}

#[derive(Clone, Copy)]
struct PreparedBlendColor {
    packed: u32,
    linear: LinearRgba,
}

impl PreparedBlendColor {
    fn new(color: Color) -> Self {
        Self {
            packed: pack_rgb(color),
            linear: color.to_linear_rgba(),
        }
    }
}

#[cfg_attr(not(test), allow(dead_code))]
fn blend_pixel(buffer: &mut [u32], width: usize, height: usize, x: i32, y: i32, color: Color) {
    if color.a == 0 {
        return;
    }

    let Some(index) = buffer_pixel_index(width, height, x, y) else {
        return;
    };
    if color.a == 255 {
        buffer[index] = pack_rgb(color);
        return;
    }

    blend_linear_over(buffer, index, color.to_linear_rgba());
}

fn blend_prepared_pixel(
    buffer: &mut [u32],
    width: usize,
    height: usize,
    x: i32,
    y: i32,
    color: PreparedBlendColor,
) {
    if color.linear.a <= 0.0 {
        return;
    }

    let Some(index) = buffer_pixel_index(width, height, x, y) else {
        return;
    };
    if color.linear.a >= 1.0 {
        buffer[index] = color.packed;
        return;
    }

    blend_linear_over(buffer, index, color.linear);
}

fn blend_mask_row(
    buffer_row: &mut [u32],
    coverages: &[u8],
    color: PreparedBlendColor,
    base_alpha: u8,
) {
    let len = buffer_row.len().min(coverages.len());
    if base_alpha == 0 || len == 0 {
        return;
    }

    let buffer_row = &mut buffer_row[..len];
    let coverages = &coverages[..len];

    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if std::is_x86_feature_detected!("sse2") {
            // SAFETY: The call is gated behind a runtime SSE2 feature check.
            unsafe {
                blend_mask_row_sse2(buffer_row, coverages, color, base_alpha);
            }
            return;
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: NEON is part of the AArch64 baseline ISA.
        unsafe {
            blend_mask_row_neon(buffer_row, coverages, color, base_alpha);
        }
        return;
    }

    #[cfg(target_arch = "arm")]
    {
        if std::arch::is_arm_feature_detected!("neon") {
            // SAFETY: The call is gated behind a runtime NEON feature check.
            unsafe {
                blend_mask_row_neon(buffer_row, coverages, color, base_alpha);
            }
            return;
        }
    }

    blend_mask_row_scalar(buffer_row, coverages, color, base_alpha);
}

fn blend_mask_row_scalar(
    buffer_row: &mut [u32],
    coverages: &[u8],
    color: PreparedBlendColor,
    base_alpha: u8,
) {
    for (pixel, &coverage) in buffer_row.iter_mut().zip(coverages) {
        let alpha = scale_alpha(coverage, base_alpha);
        if alpha == 0 {
            continue;
        }
        if alpha == u8::MAX {
            *pixel = color.packed;
            continue;
        }

        let alpha = alpha as f32 / 255.0;
        let inverse_alpha = 1.0 - alpha;
        let destination = unpack_rgb(*pixel).to_linear_rgba();
        *pixel = pack_linear_rgb(LinearRgba {
            r: color.linear.r * alpha + destination.r * inverse_alpha,
            g: color.linear.g * alpha + destination.g * inverse_alpha,
            b: color.linear.b * alpha + destination.b * inverse_alpha,
            a: 1.0,
        });
    }
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
unsafe fn blend_mask_row_sse2(
    buffer_row: &mut [u32],
    coverages: &[u8],
    color: PreparedBlendColor,
    base_alpha: u8,
) {
    #[cfg(target_arch = "x86")]
    use std::arch::x86::{_mm_cmpeq_epi8, _mm_loadu_si128, _mm_movemask_epi8, _mm_setzero_si128};
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::{
        _mm_cmpeq_epi8, _mm_loadu_si128, _mm_movemask_epi8, _mm_setzero_si128,
    };

    let mut index = 0;
    let zero = unsafe { _mm_setzero_si128() };
    while index + 16 <= coverages.len() {
        let chunk = unsafe { _mm_loadu_si128(coverages.as_ptr().add(index).cast()) };
        let zero_mask = unsafe { _mm_movemask_epi8(_mm_cmpeq_epi8(chunk, zero)) };
        if zero_mask == 0xFFFF {
            index += 16;
            continue;
        }

        let chunk_end = index + 16;
        blend_mask_row_scalar(
            &mut buffer_row[index..chunk_end],
            &coverages[index..chunk_end],
            color,
            base_alpha,
        );
        index = chunk_end;
    }

    if index < coverages.len() {
        blend_mask_row_scalar(
            &mut buffer_row[index..],
            &coverages[index..],
            color,
            base_alpha,
        );
    }
}

#[cfg(target_arch = "aarch64")]
unsafe fn blend_mask_row_neon(
    buffer_row: &mut [u32],
    coverages: &[u8],
    color: PreparedBlendColor,
    base_alpha: u8,
) {
    use std::arch::aarch64::{
        vceqq_u8, vdupq_n_u8, vgetq_lane_u64, vld1q_u8, vreinterpretq_u64_u8,
    };

    let mut index = 0;
    let zero = unsafe { vdupq_n_u8(0) };
    while index + 16 <= coverages.len() {
        let chunk = unsafe { vld1q_u8(coverages.as_ptr().add(index)) };
        let zero_mask = unsafe { vreinterpretq_u64_u8(vceqq_u8(chunk, zero)) };
        let low = unsafe { vgetq_lane_u64(zero_mask, 0) };
        let high = unsafe { vgetq_lane_u64(zero_mask, 1) };
        if low == u64::MAX && high == u64::MAX {
            index += 16;
            continue;
        }

        let chunk_end = index + 16;
        blend_mask_row_scalar(
            &mut buffer_row[index..chunk_end],
            &coverages[index..chunk_end],
            color,
            base_alpha,
        );
        index = chunk_end;
    }

    if index < coverages.len() {
        blend_mask_row_scalar(
            &mut buffer_row[index..],
            &coverages[index..],
            color,
            base_alpha,
        );
    }
}

#[cfg(target_arch = "arm")]
#[target_feature(enable = "neon")]
unsafe fn blend_mask_row_neon(
    buffer_row: &mut [u32],
    coverages: &[u8],
    color: PreparedBlendColor,
    base_alpha: u8,
) {
    use std::arch::arm::{vceqq_u8, vdupq_n_u8, vgetq_lane_u64, vld1q_u8, vreinterpretq_u64_u8};

    let mut index = 0;
    let zero = unsafe { vdupq_n_u8(0) };
    while index + 16 <= coverages.len() {
        let chunk = unsafe { vld1q_u8(coverages.as_ptr().add(index)) };
        let zero_mask = unsafe { vreinterpretq_u64_u8(vceqq_u8(chunk, zero)) };
        let low = unsafe { vgetq_lane_u64(zero_mask, 0) };
        let high = unsafe { vgetq_lane_u64(zero_mask, 1) };
        if low == u64::MAX && high == u64::MAX {
            index += 16;
            continue;
        }

        let chunk_end = index + 16;
        blend_mask_row_scalar(
            &mut buffer_row[index..chunk_end],
            &coverages[index..chunk_end],
            color,
            base_alpha,
        );
        index = chunk_end;
    }

    if index < coverages.len() {
        blend_mask_row_scalar(
            &mut buffer_row[index..],
            &coverages[index..],
            color,
            base_alpha,
        );
    }
}

fn buffer_pixel_index(width: usize, height: usize, x: i32, y: i32) -> Option<usize> {
    if x < 0 || y < 0 || x >= width as i32 || y >= height as i32 {
        return None;
    }

    let rows = current_render_buffer_rows();
    let y = y as usize;
    if y < rows.start || y >= rows.end {
        return None;
    }

    Some((y - rows.start) * width + x as usize)
}

fn blend_linear_over(buffer: &mut [u32], index: usize, source: LinearRgba) {
    let destination = unpack_rgb(buffer[index]).to_linear_rgba();
    let alpha = source.a;
    let inverse_alpha = 1.0 - alpha;
    let blended = LinearRgba {
        r: source.r * alpha + destination.r * inverse_alpha,
        g: source.g * alpha + destination.g * inverse_alpha,
        b: source.b * alpha + destination.b * inverse_alpha,
        a: 1.0,
    };

    buffer[index] = pack_linear_rgb(blended);
}

fn scale_alpha(coverage: u8, alpha: u8) -> u8 {
    if alpha == u8::MAX {
        coverage
    } else {
        ((u16::from(coverage) * u16::from(alpha) + 127) / 255) as u8
    }
}

fn pack_linear_rgb(color: LinearRgba) -> u32 {
    pack_rgb(Color::from_linear_rgba(LinearRgba { a: 1.0, ..color }))
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
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;

    use cssimpler_core::fonts::{FontFamily, TextStyle, TextTransform, register_font_file};
    use cssimpler_core::{
        AnglePercentageValue, BackgroundLayer, BoxShadow, CircleRadius, Color, ConicGradient,
        CornerRadius, ElementPath, GradientDirection, GradientHorizontal, GradientInterpolation,
        GradientPoint, GradientStop, Insets, LayoutBox, LengthPercentageValue, LinearGradient,
        Overflow, RadialGradient, RadialShape, RenderNode, ShadowEffect, TextStrokeStyle,
        VisualStyle,
    };

    use crate::{
        ClipRect, DirtyRenderJob, FramePaintMode, FramePaintReason, MouseEventKind, ViewportSize,
        WindowConfig, blend_pixel, build_incremental_render_jobs, coalesce_dirty_regions,
        dirty_regions_between_scenes, dispatch_click, dispatch_hover_transition_events,
        dispatch_mouse_event, distribute_dirty_render_jobs, drawable_viewport_size,
        hit_test_element_path, pack_rgb, render_scene_update, render_scene_update_internal,
        render_to_buffer, resize_buffer, scenes_match_visuals, should_present_frame,
        should_present_scene, should_suspend_updates, window_options,
    };

    static CLICK_COUNT: AtomicUsize = AtomicUsize::new(0);
    static CLICK_TARGET: AtomicUsize = AtomicUsize::new(0);
    static EVENT_FLAGS: AtomicUsize = AtomicUsize::new(0);
    const FLAG_PARENT_ENTER: usize = 1 << 0;
    const FLAG_CHILD_ENTER: usize = 1 << 1;
    const FLAG_CHILD_OVER: usize = 1 << 2;
    const FLAG_CHILD_OUT: usize = 1 << 3;
    const FLAG_CHILD_LEAVE: usize = 1 << 4;
    const FLAG_PARENT_LEAVE: usize = 1 << 5;
    const FLAG_MOUSE_MOVE: usize = 1 << 6;
    const FLAG_MOUSE_DOWN: usize = 1 << 7;
    const FLAG_MOUSE_UP: usize = 1 << 8;
    const FLAG_CONTEXT_MENU: usize = 1 << 9;
    const FLAG_DBLCLICK: usize = 1 << 10;

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

    fn mark_parent_enter() {
        EVENT_FLAGS.fetch_or(FLAG_PARENT_ENTER, Ordering::SeqCst);
    }

    fn mark_child_enter() {
        EVENT_FLAGS.fetch_or(FLAG_CHILD_ENTER, Ordering::SeqCst);
    }

    fn mark_child_over() {
        EVENT_FLAGS.fetch_or(FLAG_CHILD_OVER, Ordering::SeqCst);
    }

    fn mark_child_out() {
        EVENT_FLAGS.fetch_or(FLAG_CHILD_OUT, Ordering::SeqCst);
    }

    fn mark_child_leave() {
        EVENT_FLAGS.fetch_or(FLAG_CHILD_LEAVE, Ordering::SeqCst);
    }

    fn mark_parent_leave() {
        EVENT_FLAGS.fetch_or(FLAG_PARENT_LEAVE, Ordering::SeqCst);
    }

    fn mark_mouse_move() {
        EVENT_FLAGS.fetch_or(FLAG_MOUSE_MOVE, Ordering::SeqCst);
    }

    fn mark_mouse_down() {
        EVENT_FLAGS.fetch_or(FLAG_MOUSE_DOWN, Ordering::SeqCst);
    }

    fn mark_mouse_up() {
        EVENT_FLAGS.fetch_or(FLAG_MOUSE_UP, Ordering::SeqCst);
    }

    fn mark_context_menu() {
        EVENT_FLAGS.fetch_or(FLAG_CONTEXT_MENU, Ordering::SeqCst);
    }

    fn mark_dblclick() {
        EVENT_FLAGS.fetch_or(FLAG_DBLCLICK, Ordering::SeqCst);
    }

    fn bundled_font_family() -> String {
        let asset_path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../examples/assets/powerline-demo.ttf");
        let families = register_font_file(&asset_path)
            .expect("bundled powerline demo font should register during renderer tests");
        families
            .into_iter()
            .next()
            .expect("bundled powerline font should expose at least one family name")
    }

    fn text_scene_with_content(style: TextStyle, content: &str) -> Vec<RenderNode> {
        vec![
            RenderNode::container(LayoutBox::new(0.0, 0.0, 320.0, 120.0))
                .with_style(VisualStyle {
                    background: Some(Color::rgb(245, 247, 250)),
                    ..VisualStyle::default()
                })
                .with_child(
                    RenderNode::text(LayoutBox::new(20.0, 28.0, 280.0, 56.0), content).with_style(
                        VisualStyle {
                            foreground: Color::rgb(17, 37, 61),
                            text: style,
                            ..VisualStyle::default()
                        },
                    ),
                ),
        ]
    }

    fn text_scene(style: TextStyle) -> Vec<RenderNode> {
        text_scene_with_content(style, "WWW iii 0123456789")
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
    fn multithreaded_full_redraw_matches_the_single_threaded_result() {
        let _shadow_cache_guard = super::shadow::lock_shadow_mask_cache_for_tests();
        let scene = vec![
            RenderNode::container(LayoutBox::new(8.0, 8.0, 96.0, 72.0))
                .with_style(VisualStyle {
                    background_layers: vec![BackgroundLayer::LinearGradient(LinearGradient {
                        direction: GradientDirection::Horizontal(GradientHorizontal::Right),
                        interpolation: GradientInterpolation::Oklab,
                        repeating: false,
                        stops: vec![
                            GradientStop {
                                color: Color::rgb(14, 165, 233),
                                position: LengthPercentageValue::from_fraction(0.0),
                            },
                            GradientStop {
                                color: Color::rgb(244, 114, 182),
                                position: LengthPercentageValue::from_fraction(1.0),
                            },
                        ],
                    })],
                    shadows: vec![BoxShadow {
                        color: Color::rgba(15, 23, 42, 120),
                        offset_x: 6.0,
                        offset_y: 8.0,
                        blur_radius: 12.0,
                        spread: 2.0,
                    }],
                    corner_radius: CornerRadius::all(16.0),
                    ..VisualStyle::default()
                })
                .with_child(
                    RenderNode::container(LayoutBox::new(20.0, 20.0, 40.0, 24.0)).with_style(
                        VisualStyle {
                            background: Some(Color::rgba(255, 255, 255, 180)),
                            filter_drop_shadows: vec![ShadowEffect {
                                color: Some(Color::rgba(56, 189, 248, 110)),
                                offset_x: 0.0,
                                offset_y: 0.0,
                                blur_radius: 8.0,
                                spread: 0.0,
                            }],
                            corner_radius: CornerRadius::all(12.0),
                            ..VisualStyle::default()
                        },
                    ),
                ),
        ];
        let mut single = vec![0_u32; 128 * 96];
        let mut threaded = vec![0_u32; 128 * 96];

        super::render_to_buffer_serial(&scene, &mut single, 128, 96, Color::BLACK);
        super::render_to_buffer_parallel(&scene, None, &mut threaded, 128, 96, Color::BLACK, 4);

        assert_eq!(single, threaded);
    }

    #[test]
    fn identical_box_shadow_masks_are_reused_across_integer_position_changes() {
        let _cache_guard = super::shadow::lock_shadow_mask_cache_for_tests();
        super::clear_shadow_mask_cache_for_tests();
        let shadow = BoxShadow {
            color: Color::rgba(15, 23, 42, 120),
            offset_x: 6.0,
            offset_y: 8.0,
            blur_radius: 12.0,
            spread: 2.0,
        };
        let radius = CornerRadius::all(16.0);
        let first_layout = super::offset_layout(
            super::expand_layout(LayoutBox::new(10.25, 20.0, 96.0, 72.0), shadow.spread),
            shadow.offset_x,
            shadow.offset_y,
        );
        let second_layout = super::offset_layout(
            super::expand_layout(LayoutBox::new(74.25, 52.0, 96.0, 72.0), shadow.spread),
            shadow.offset_x,
            shadow.offset_y,
        );
        let shadow_radius = super::expand_corner_radius(radius, shadow.spread);

        let (first, _, _) =
            super::cached_shadow_mask(first_layout, shadow_radius, shadow.blur_radius);
        let (second, _, _) =
            super::cached_shadow_mask(second_layout, shadow_radius, shadow.blur_radius);

        assert!(std::sync::Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn blend_pixel_respects_local_worker_row_offsets() {
        let mut buffer = vec![0_u32; 3 * 2];

        super::with_render_buffer_rows(super::BufferRows::new(4, 6), || {
            blend_pixel(&mut buffer, 3, 8, 1, 4, Color::rgb(40, 120, 220));
            blend_pixel(&mut buffer, 3, 8, 2, 5, Color::rgb(220, 38, 38));
        });

        assert_eq!(buffer[1], pack_rgb(Color::rgb(40, 120, 220)));
        assert_eq!(buffer[5], pack_rgb(Color::rgb(220, 38, 38)));
    }

    #[test]
    fn blend_mask_row_matches_scalar_reference() {
        let mut accelerated = vec![
            pack_rgb(Color::rgb(15, 23, 42)),
            pack_rgb(Color::rgb(226, 232, 240)),
            pack_rgb(Color::rgb(30, 41, 59)),
            pack_rgb(Color::rgb(148, 163, 184)),
            pack_rgb(Color::rgb(8, 47, 73)),
            pack_rgb(Color::rgb(125, 211, 252)),
            pack_rgb(Color::rgb(15, 118, 110)),
            pack_rgb(Color::rgb(244, 114, 182)),
            pack_rgb(Color::rgb(24, 24, 27)),
            pack_rgb(Color::rgb(244, 244, 245)),
            pack_rgb(Color::rgb(76, 29, 149)),
            pack_rgb(Color::rgb(251, 191, 36)),
            pack_rgb(Color::rgb(63, 63, 70)),
            pack_rgb(Color::rgb(214, 211, 209)),
            pack_rgb(Color::rgb(17, 24, 39)),
            pack_rgb(Color::rgb(187, 247, 208)),
            pack_rgb(Color::rgb(67, 56, 202)),
            pack_rgb(Color::rgb(254, 215, 170)),
            pack_rgb(Color::rgb(2, 6, 23)),
        ];
        let mut expected = accelerated.clone();
        let coverages = [
            0, 0, 8, 0, 24, 96, 0, 160, 0, 255, 0, 64, 0, 0, 192, 0, 32, 0, 255,
        ];
        let color = super::PreparedBlendColor::new(Color::rgba(56, 189, 248, 212));

        super::blend_mask_row_scalar(&mut expected, &coverages, color, 180);
        super::blend_mask_row(&mut accelerated, &coverages, color, 180);

        assert_eq!(accelerated, expected);
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
    fn opaque_rect_fast_path_matches_fractional_center_coverage() {
        let scene = vec![
            RenderNode::container(LayoutBox::new(0.75, 0.25, 2.0, 2.0)).with_style(VisualStyle {
                background: Some(Color::rgb(40, 120, 220)),
                ..VisualStyle::default()
            }),
        ];
        let mut buffer = vec![0_u32; 4 * 4];

        render_to_buffer(&scene, &mut buffer, 4, 4, Color::WHITE);

        let accent = pack_rgb(Color::rgb(40, 120, 220));
        let white = pack_rgb(Color::WHITE);
        assert_eq!(
            buffer,
            vec![
                white, accent, accent, white, //
                white, accent, accent, white, //
                white, white, white, white, //
                white, white, white, white,
            ]
        );
    }

    #[test]
    fn opaque_rect_fast_path_respects_fractional_overflow_clip_edges() {
        let scene = vec![
            RenderNode::container(LayoutBox::new(1.8, 0.0, 2.0, 2.0))
                .with_style(VisualStyle {
                    overflow: Overflow::CLIP,
                    ..VisualStyle::default()
                })
                .with_child(
                    RenderNode::container(LayoutBox::new(0.0, 0.0, 4.0, 2.0)).with_style(
                        VisualStyle {
                            background: Some(Color::rgb(40, 120, 220)),
                            ..VisualStyle::default()
                        },
                    ),
                ),
        ];
        let mut buffer = vec![0_u32; 5 * 2];

        render_to_buffer(&scene, &mut buffer, 5, 2, Color::WHITE);

        let accent = pack_rgb(Color::rgb(40, 120, 220));
        let white = pack_rgb(Color::WHITE);
        assert_eq!(
            buffer,
            vec![
                white, accent, accent, accent, white, //
                white, accent, accent, accent, white,
            ]
        );
    }

    #[test]
    fn opaque_rectangular_border_fast_path_matches_expected_ring() {
        let scene = vec![
            RenderNode::container(LayoutBox::new(1.25, 1.25, 4.0, 4.0)).with_style(VisualStyle {
                border: cssimpler_core::BorderStyle {
                    color: Color::rgb(15, 23, 42),
                    widths: Insets::all(1.0),
                },
                ..VisualStyle::default()
            }),
        ];
        let mut buffer = vec![0_u32; 7 * 7];

        render_to_buffer(&scene, &mut buffer, 7, 7, Color::WHITE);

        let border = pack_rgb(Color::rgb(15, 23, 42));
        let white = pack_rgb(Color::WHITE);
        assert_eq!(
            buffer,
            vec![
                white, white, white, white, white, white, white, //
                white, border, border, border, border, white, white, //
                white, border, white, white, border, white, white, //
                white, border, white, white, border, white, white, //
                white, border, border, border, border, white, white, //
                white, white, white, white, white, white, white, //
                white, white, white, white, white, white, white,
            ]
        );
    }

    #[test]
    fn opaque_border_fast_path_respects_fractional_overflow_clip_edges() {
        let scene = vec![
            RenderNode::container(LayoutBox::new(1.8, 0.0, 2.0, 4.0))
                .with_style(VisualStyle {
                    overflow: Overflow::CLIP,
                    ..VisualStyle::default()
                })
                .with_child(
                    RenderNode::container(LayoutBox::new(0.0, 0.0, 4.0, 4.0)).with_style(
                        VisualStyle {
                            border: cssimpler_core::BorderStyle {
                                color: Color::rgb(15, 23, 42),
                                widths: Insets::all(1.0),
                            },
                            ..VisualStyle::default()
                        },
                    ),
                ),
        ];
        let mut buffer = vec![0_u32; 5 * 4];

        render_to_buffer(&scene, &mut buffer, 5, 4, Color::WHITE);

        let border = pack_rgb(Color::rgb(15, 23, 42));
        let white = pack_rgb(Color::WHITE);
        assert_eq!(
            buffer,
            vec![
                white, border, border, border, white, //
                white, white, white, border, white, //
                white, white, white, border, white, //
                white, border, border, border, white,
            ]
        );
    }

    #[test]
    fn linear_gradients_interpolate_in_oklab_by_default() {
        let scene = vec![
            RenderNode::container(LayoutBox::new(0.0, 0.0, 3.0, 1.0)).with_style(VisualStyle {
                background_layers: vec![BackgroundLayer::LinearGradient(LinearGradient {
                    direction: GradientDirection::Horizontal(GradientHorizontal::Right),
                    interpolation: GradientInterpolation::Oklab,
                    repeating: false,
                    stops: vec![
                        GradientStop {
                            color: Color::BLACK,
                            position: LengthPercentageValue::from_fraction(0.0),
                        },
                        GradientStop {
                            color: Color::WHITE,
                            position: LengthPercentageValue::from_fraction(1.0),
                        },
                    ],
                })],
                ..VisualStyle::default()
            }),
        ];
        let mut buffer = vec![0_u32; 3];

        render_to_buffer(&scene, &mut buffer, 3, 1, Color::BLACK);

        assert_eq!(buffer[1], pack_rgb(Color::rgb(99, 99, 99)));
    }

    #[test]
    fn linear_gradients_can_still_interpolate_in_linear_color_space() {
        let scene = vec![
            RenderNode::container(LayoutBox::new(0.0, 0.0, 3.0, 1.0)).with_style(VisualStyle {
                background_layers: vec![BackgroundLayer::LinearGradient(LinearGradient {
                    direction: GradientDirection::Horizontal(GradientHorizontal::Right),
                    interpolation: GradientInterpolation::LinearSrgb,
                    repeating: false,
                    stops: vec![
                        GradientStop {
                            color: Color::BLACK,
                            position: LengthPercentageValue::from_fraction(0.0),
                        },
                        GradientStop {
                            color: Color::WHITE,
                            position: LengthPercentageValue::from_fraction(1.0),
                        },
                    ],
                })],
                ..VisualStyle::default()
            }),
        ];
        let mut buffer = vec![0_u32; 3];

        render_to_buffer(&scene, &mut buffer, 3, 1, Color::BLACK);

        assert_eq!(buffer[1], pack_rgb(Color::rgb(188, 188, 188)));
    }

    #[test]
    fn layered_backgrounds_draw_the_first_layer_on_top() {
        let scene = vec![
            RenderNode::container(LayoutBox::new(0.0, 0.0, 3.0, 1.0)).with_style(VisualStyle {
                background_layers: vec![
                    BackgroundLayer::LinearGradient(LinearGradient {
                        direction: GradientDirection::Horizontal(GradientHorizontal::Right),
                        interpolation: GradientInterpolation::Oklab,
                        repeating: false,
                        stops: vec![
                            GradientStop {
                                color: Color::rgb(220, 38, 38),
                                position: LengthPercentageValue::from_fraction(0.0),
                            },
                            GradientStop {
                                color: Color::rgb(220, 38, 38),
                                position: LengthPercentageValue::from_fraction(1.0),
                            },
                        ],
                    }),
                    BackgroundLayer::LinearGradient(LinearGradient {
                        direction: GradientDirection::Horizontal(GradientHorizontal::Right),
                        interpolation: GradientInterpolation::Oklab,
                        repeating: false,
                        stops: vec![
                            GradientStop {
                                color: Color::rgb(37, 99, 235),
                                position: LengthPercentageValue::from_fraction(0.0),
                            },
                            GradientStop {
                                color: Color::rgb(37, 99, 235),
                                position: LengthPercentageValue::from_fraction(1.0),
                            },
                        ],
                    }),
                ],
                ..VisualStyle::default()
            }),
        ];
        let mut buffer = vec![0_u32; 3];

        render_to_buffer(&scene, &mut buffer, 3, 1, Color::BLACK);

        assert_eq!(buffer[1], pack_rgb(Color::rgb(220, 38, 38)));
    }

    #[test]
    fn linear_gradients_support_length_based_stop_positions() {
        let scene = vec![
            RenderNode::container(LayoutBox::new(0.0, 0.0, 5.0, 1.0)).with_style(VisualStyle {
                background_layers: vec![BackgroundLayer::LinearGradient(LinearGradient {
                    direction: GradientDirection::Horizontal(GradientHorizontal::Right),
                    interpolation: GradientInterpolation::Oklab,
                    repeating: false,
                    stops: vec![
                        GradientStop {
                            color: Color::BLACK,
                            position: LengthPercentageValue::from_px(0.0),
                        },
                        GradientStop {
                            color: Color::WHITE,
                            position: LengthPercentageValue::from_px(2.0),
                        },
                    ],
                })],
                ..VisualStyle::default()
            }),
        ];
        let mut buffer = vec![0_u32; 5];

        render_to_buffer(&scene, &mut buffer, 5, 1, Color::BLACK);

        assert_ne!(buffer[1], pack_rgb(Color::WHITE));
        assert_eq!(buffer[2], pack_rgb(Color::WHITE));
        assert_eq!(buffer[4], pack_rgb(Color::WHITE));
    }

    #[test]
    fn radial_gradients_render_from_the_center_outward() {
        let scene = vec![
            RenderNode::container(LayoutBox::new(0.0, 0.0, 5.0, 5.0)).with_style(VisualStyle {
                background_layers: vec![BackgroundLayer::RadialGradient(RadialGradient {
                    shape: RadialShape::Circle(CircleRadius::Explicit(2.0)),
                    center: GradientPoint::CENTER,
                    interpolation: GradientInterpolation::Oklab,
                    repeating: false,
                    stops: vec![
                        GradientStop {
                            color: Color::BLACK,
                            position: LengthPercentageValue::from_px(0.0),
                        },
                        GradientStop {
                            color: Color::WHITE,
                            position: LengthPercentageValue::from_px(2.0),
                        },
                    ],
                })],
                ..VisualStyle::default()
            }),
        ];
        let mut buffer = vec![0_u32; 25];

        render_to_buffer(&scene, &mut buffer, 5, 5, Color::BLACK);

        assert_eq!(buffer[2 * 5 + 2], pack_rgb(Color::BLACK));
        assert_eq!(buffer[2 * 5 + 4], pack_rgb(Color::WHITE));
    }

    #[test]
    fn conic_gradients_support_multiple_angle_sectors() {
        let scene = vec![
            RenderNode::container(LayoutBox::new(0.0, 0.0, 5.0, 5.0)).with_style(VisualStyle {
                background_layers: vec![BackgroundLayer::ConicGradient(ConicGradient {
                    angle: 0.0,
                    center: GradientPoint::CENTER,
                    interpolation: GradientInterpolation::Oklab,
                    repeating: false,
                    stops: vec![
                        GradientStop {
                            color: Color::rgb(255, 0, 0),
                            position: AnglePercentageValue::from_degrees(0.0),
                        },
                        GradientStop {
                            color: Color::rgb(255, 0, 0),
                            position: AnglePercentageValue::from_degrees(90.0),
                        },
                        GradientStop {
                            color: Color::rgb(0, 255, 0),
                            position: AnglePercentageValue::from_degrees(90.0),
                        },
                        GradientStop {
                            color: Color::rgb(0, 255, 0),
                            position: AnglePercentageValue::from_degrees(180.0),
                        },
                        GradientStop {
                            color: Color::rgb(0, 0, 255),
                            position: AnglePercentageValue::from_degrees(180.0),
                        },
                        GradientStop {
                            color: Color::rgb(0, 0, 255),
                            position: AnglePercentageValue::from_degrees(270.0),
                        },
                        GradientStop {
                            color: Color::rgb(255, 255, 255),
                            position: AnglePercentageValue::from_degrees(270.0),
                        },
                        GradientStop {
                            color: Color::rgb(255, 255, 255),
                            position: AnglePercentageValue::from_turns(1.0),
                        },
                    ],
                })],
                ..VisualStyle::default()
            }),
        ];
        let mut buffer = vec![0_u32; 25];

        render_to_buffer(&scene, &mut buffer, 5, 5, Color::BLACK);

        assert_eq!(buffer[4], pack_rgb(Color::rgb(255, 0, 0)));
        assert_eq!(buffer[4 * 5 + 4], pack_rgb(Color::rgb(0, 255, 0)));
        assert_eq!(buffer[4 * 5], pack_rgb(Color::rgb(0, 0, 255)));
        assert_eq!(buffer[0], pack_rgb(Color::rgb(255, 255, 255)));
    }

    #[test]
    fn gallery_style_conic_gradient_changes_when_the_interpolation_mode_changes() {
        fn render_conic(interpolation: GradientInterpolation) -> Vec<u32> {
            let scene = vec![
                RenderNode::container(LayoutBox::new(0.0, 0.0, 96.0, 96.0)).with_style(
                    VisualStyle {
                        background_layers: vec![BackgroundLayer::ConicGradient(ConicGradient {
                            angle: 220.0,
                            center: GradientPoint {
                                x: LengthPercentageValue::from_fraction(0.64),
                                y: LengthPercentageValue::from_fraction(0.56),
                            },
                            interpolation,
                            repeating: false,
                            stops: vec![
                                GradientStop {
                                    color: Color::rgb(56, 189, 248),
                                    position: AnglePercentageValue::from_degrees(0.0),
                                },
                                GradientStop {
                                    color: Color::rgb(45, 212, 191),
                                    position: AnglePercentageValue::from_degrees(100.0),
                                },
                                GradientStop {
                                    color: Color::rgb(245, 158, 11),
                                    position: AnglePercentageValue::from_degrees(220.0),
                                },
                                GradientStop {
                                    color: Color::rgb(244, 114, 182),
                                    position: AnglePercentageValue::from_degrees(300.0),
                                },
                                GradientStop {
                                    color: Color::rgb(56, 189, 248),
                                    position: AnglePercentageValue::from_degrees(360.0),
                                },
                            ],
                        })],
                        ..VisualStyle::default()
                    },
                ),
            ];
            let mut buffer = vec![0_u32; 96 * 96];
            render_to_buffer(&scene, &mut buffer, 96, 96, Color::BLACK);
            buffer
        }

        let oklab = render_conic(GradientInterpolation::Oklab);
        let linear = render_conic(GradientInterpolation::LinearSrgb);
        let different_pixels = oklab
            .iter()
            .zip(&linear)
            .filter(|(left, right)| left != right)
            .count();

        assert!(different_pixels > 0);
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
        let _shadow_cache_guard = super::shadow::lock_shadow_mask_cache_for_tests();
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
    fn alpha_blending_uses_linear_color_space() {
        let mut buffer = vec![pack_rgb(Color::BLACK)];

        blend_pixel(&mut buffer, 1, 1, 0, 0, Color::rgba(255, 255, 255, 128));

        assert_eq!(buffer[0], pack_rgb(Color::rgb(188, 188, 188)));
    }

    #[test]
    fn rendered_text_pixels_change_when_font_family_changes() {
        let _text_cache_guard = super::fonts::lock_text_mask_cache_for_tests();
        let bundled_family = bundled_font_family();
        let baseline_style = TextStyle {
            size_px: 28.0,
            ..TextStyle::default()
        };
        let bundled_style = TextStyle {
            families: vec![FontFamily::Named(bundled_family)],
            size_px: 28.0,
            ..TextStyle::default()
        };
        let baseline_scene = text_scene(baseline_style);
        let bundled_scene = text_scene(bundled_style);
        let mut baseline_buffer = vec![0_u32; 320 * 120];
        let mut bundled_buffer = vec![0_u32; 320 * 120];

        render_to_buffer(
            &baseline_scene,
            &mut baseline_buffer,
            320,
            120,
            Color::WHITE,
        );
        render_to_buffer(&bundled_scene, &mut bundled_buffer, 320, 120, Color::WHITE);

        let different_pixels = baseline_buffer
            .iter()
            .zip(&bundled_buffer)
            .filter(|(left, right)| left != right)
            .count();

        assert_ne!(baseline_buffer, bundled_buffer);
        assert!(different_pixels > 100);
    }

    #[test]
    fn text_transform_renders_the_same_pixels_as_pretransformed_content() {
        let _text_cache_guard = super::fonts::lock_text_mask_cache_for_tests();
        let transformed_scene = text_scene_with_content(
            TextStyle {
                text_transform: TextTransform::Uppercase,
                ..TextStyle::default()
            },
            "Straße",
        );
        let literal_scene = text_scene_with_content(TextStyle::default(), "STRASSE");
        let mut transformed_buffer = vec![0_u32; 320 * 120];
        let mut literal_buffer = vec![0_u32; 320 * 120];

        render_to_buffer(
            &transformed_scene,
            &mut transformed_buffer,
            320,
            120,
            Color::WHITE,
        );
        render_to_buffer(&literal_scene, &mut literal_buffer, 320, 120, Color::WHITE);

        assert_eq!(transformed_buffer, literal_buffer);
    }

    #[test]
    fn text_stroke_renders_outline_pixels_without_extra_nodes() {
        let _text_cache_guard = super::fonts::lock_text_mask_cache_for_tests();
        let stroked_scene = vec![
            RenderNode::container(LayoutBox::new(0.0, 0.0, 320.0, 120.0))
                .with_style(VisualStyle {
                    background: Some(Color::rgb(245, 247, 250)),
                    ..VisualStyle::default()
                })
                .with_child(
                    RenderNode::text(LayoutBox::new(20.0, 28.0, 280.0, 56.0), "Outline")
                        .with_style(VisualStyle {
                            foreground: Color::rgb(17, 37, 61),
                            text: TextStyle::default(),
                            text_stroke: TextStrokeStyle {
                                width: 2.0,
                                color: Some(Color::rgb(245, 158, 11)),
                            },
                            ..VisualStyle::default()
                        }),
                ),
        ];
        let baseline_outline_scene = vec![
            RenderNode::container(LayoutBox::new(0.0, 0.0, 320.0, 120.0))
                .with_style(VisualStyle {
                    background: Some(Color::rgb(245, 247, 250)),
                    ..VisualStyle::default()
                })
                .with_child(
                    RenderNode::text(LayoutBox::new(20.0, 28.0, 280.0, 56.0), "Outline")
                        .with_style(VisualStyle {
                            foreground: Color::rgb(17, 37, 61),
                            text: TextStyle::default(),
                            ..VisualStyle::default()
                        }),
                ),
        ];
        let mut baseline_buffer = vec![0_u32; 320 * 120];
        let mut stroked_buffer = vec![0_u32; 320 * 120];

        render_to_buffer(
            &baseline_outline_scene,
            &mut baseline_buffer,
            320,
            120,
            Color::WHITE,
        );
        render_to_buffer(&stroked_scene, &mut stroked_buffer, 320, 120, Color::WHITE);

        let different_pixels = baseline_buffer
            .iter()
            .zip(&stroked_buffer)
            .filter(|(left, right)| left != right)
            .count();

        assert!(different_pixels > 0);
        assert!(stroked_buffer.contains(&pack_rgb(Color::rgb(245, 158, 11))));
    }

    #[test]
    fn text_shadow_renders_glow_outside_text_bounds() {
        let _text_cache_guard = super::fonts::lock_text_mask_cache_for_tests();
        let scene = vec![
            RenderNode::container(LayoutBox::new(0.0, 0.0, 320.0, 120.0))
                .with_style(VisualStyle {
                    background: Some(Color::rgb(245, 247, 250)),
                    ..VisualStyle::default()
                })
                .with_child(
                    RenderNode::text(LayoutBox::new(40.0, 36.0, 180.0, 40.0), "Glow").with_style(
                        VisualStyle {
                            foreground: Color::rgb(17, 37, 61),
                            text: TextStyle::default(),
                            text_shadows: vec![ShadowEffect {
                                color: Some(Color::rgba(37, 99, 235, 180)),
                                offset_x: 0.0,
                                offset_y: 0.0,
                                blur_radius: 4.0,
                                spread: 2.0,
                            }],
                            ..VisualStyle::default()
                        },
                    ),
                ),
        ];
        let mut buffer = vec![0_u32; 320 * 120];

        render_to_buffer(&scene, &mut buffer, 320, 120, Color::WHITE);

        assert_ne!(buffer[34 * 320 + 36], pack_rgb(Color::WHITE));
    }

    #[test]
    fn visible_overflow_text_shadow_can_paint_beyond_the_text_layout_box() {
        let _text_cache_guard = super::fonts::lock_text_mask_cache_for_tests();
        let background = Color::rgb(245, 247, 250);
        let scene = vec![
            RenderNode::container(LayoutBox::new(0.0, 0.0, 240.0, 120.0))
                .with_style(VisualStyle {
                    background: Some(background),
                    ..VisualStyle::default()
                })
                .with_child(
                    RenderNode::text(LayoutBox::new(80.0, 40.0, 84.0, 36.0), "Glow").with_style(
                        VisualStyle {
                            foreground: Color::rgb(34, 197, 94),
                            text: TextStyle::default(),
                            text_shadows: vec![ShadowEffect {
                                color: Some(Color::rgba(34, 197, 94, 200)),
                                offset_x: 0.0,
                                offset_y: 0.0,
                                blur_radius: 6.0,
                                spread: 3.0,
                            }],
                            ..VisualStyle::default()
                        },
                    ),
                ),
        ];
        let mut buffer = vec![0_u32; 240 * 120];

        render_to_buffer(&scene, &mut buffer, 240, 120, Color::WHITE);

        assert_ne!(buffer[48 * 240 + 76], pack_rgb(background));
    }

    #[test]
    fn clipped_text_shadow_stays_inside_the_text_layout_box() {
        let _text_cache_guard = super::fonts::lock_text_mask_cache_for_tests();
        let background = Color::rgb(245, 247, 250);
        let scene = vec![
            RenderNode::container(LayoutBox::new(0.0, 0.0, 240.0, 120.0))
                .with_style(VisualStyle {
                    background: Some(background),
                    ..VisualStyle::default()
                })
                .with_child(
                    RenderNode::text(LayoutBox::new(80.0, 40.0, 84.0, 36.0), "Glow").with_style(
                        VisualStyle {
                            foreground: Color::rgb(34, 197, 94),
                            text: TextStyle::default(),
                            text_shadows: vec![ShadowEffect {
                                color: Some(Color::rgba(34, 197, 94, 200)),
                                offset_x: 0.0,
                                offset_y: 0.0,
                                blur_radius: 6.0,
                                spread: 3.0,
                            }],
                            overflow: Overflow::CLIP,
                            ..VisualStyle::default()
                        },
                    ),
                ),
        ];
        let mut buffer = vec![0_u32; 240 * 120];

        render_to_buffer(&scene, &mut buffer, 240, 120, Color::WHITE);

        assert_eq!(buffer[48 * 240 + 76], pack_rgb(background));
    }

    #[test]
    fn filter_drop_shadow_renders_for_supported_container_layers() {
        let _shadow_cache_guard = super::shadow::lock_shadow_mask_cache_for_tests();
        let scene = vec![
            RenderNode::container(LayoutBox::new(48.0, 32.0, 72.0, 44.0)).with_style(VisualStyle {
                background: Some(Color::rgb(241, 245, 249)),
                filter_drop_shadows: vec![ShadowEffect {
                    color: Some(Color::rgba(15, 23, 42, 180)),
                    offset_x: 8.0,
                    offset_y: 10.0,
                    blur_radius: 6.0,
                    spread: 0.0,
                }],
                ..VisualStyle::default()
            }),
        ];
        let mut buffer = vec![0_u32; 200 * 140];

        render_to_buffer(&scene, &mut buffer, 200, 140, Color::WHITE);

        assert_ne!(buffer[86 * 200 + 126], pack_rgb(Color::WHITE));
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
    fn dispatch_mouse_event_supports_requested_handlers() {
        EVENT_FLAGS.store(0, Ordering::SeqCst);
        let scene = vec![
            RenderNode::container(LayoutBox::new(4.0, 6.0, 40.0, 24.0))
                .on_contextmenu(mark_context_menu)
                .on_dblclick(mark_dblclick)
                .on_mousedown(mark_mouse_down)
                .on_mousemove(mark_mouse_move)
                .on_mouseup(mark_mouse_up),
        ];

        assert!(dispatch_mouse_event(
            &scene,
            12.0,
            12.0,
            MouseEventKind::ContextMenu
        ));
        assert!(dispatch_mouse_event(
            &scene,
            12.0,
            12.0,
            MouseEventKind::DblClick
        ));
        assert!(dispatch_mouse_event(
            &scene,
            12.0,
            12.0,
            MouseEventKind::MouseDown
        ));
        assert!(dispatch_mouse_event(
            &scene,
            12.0,
            12.0,
            MouseEventKind::MouseMove
        ));
        assert!(dispatch_mouse_event(
            &scene,
            12.0,
            12.0,
            MouseEventKind::MouseUp
        ));
        assert_eq!(
            EVENT_FLAGS.load(Ordering::SeqCst),
            FLAG_CONTEXT_MENU | FLAG_DBLCLICK | FLAG_MOUSE_DOWN | FLAG_MOUSE_MOVE | FLAG_MOUSE_UP
        );
    }

    #[test]
    fn hover_transition_dispatches_enter_leave_and_over_out_handlers() {
        EVENT_FLAGS.store(0, Ordering::SeqCst);
        let root_path = ElementPath::root(0);
        let child_path = root_path.with_child(0);
        let scene = vec![
            RenderNode::container(LayoutBox::new(0.0, 0.0, 80.0, 60.0))
                .with_element_path(root_path.clone())
                .on_mouseenter(mark_parent_enter)
                .on_mouseleave(mark_parent_leave)
                .with_child(
                    RenderNode::container(LayoutBox::new(12.0, 10.0, 30.0, 20.0))
                        .with_element_path(child_path.clone())
                        .on_mouseenter(mark_child_enter)
                        .on_mouseover(mark_child_over)
                        .on_mouseout(mark_child_out)
                        .on_mouseleave(mark_child_leave),
                ),
        ];

        assert!(dispatch_hover_transition_events(
            &scene,
            None,
            Some(&child_path)
        ));
        assert_eq!(
            EVENT_FLAGS.load(Ordering::SeqCst),
            FLAG_PARENT_ENTER | FLAG_CHILD_ENTER | FLAG_CHILD_OVER
        );

        EVENT_FLAGS.store(0, Ordering::SeqCst);
        assert!(dispatch_hover_transition_events(
            &scene,
            Some(&child_path),
            Some(&root_path)
        ));
        assert_eq!(
            EVENT_FLAGS.load(Ordering::SeqCst),
            FLAG_CHILD_OUT | FLAG_CHILD_LEAVE
        );

        EVENT_FLAGS.store(0, Ordering::SeqCst);
        assert!(dispatch_hover_transition_events(
            &scene,
            Some(&root_path),
            None
        ));
        assert_eq!(EVENT_FLAGS.load(Ordering::SeqCst), FLAG_PARENT_LEAVE);
    }

    #[test]
    fn hit_test_element_path_returns_the_deepest_visible_node() {
        let scene = vec![
            RenderNode::container(LayoutBox::new(0.0, 0.0, 80.0, 60.0)).with_child(
                RenderNode::container(LayoutBox::new(12.0, 10.0, 30.0, 20.0))
                    .with_child(RenderNode::container(LayoutBox::new(18.0, 14.0, 8.0, 6.0))),
            ),
        ];

        assert_eq!(
            hit_test_element_path(&scene, 20.0, 18.0),
            Some(ElementPath {
                root: 0,
                children: vec![0, 0],
            })
        );
        assert_eq!(
            hit_test_element_path(&scene, 6.0, 6.0),
            Some(ElementPath::root(0))
        );
    }

    #[test]
    fn hit_test_element_path_respects_parent_clipping() {
        let scene = vec![
            RenderNode::container(LayoutBox::new(0.0, 0.0, 20.0, 20.0))
                .with_style(VisualStyle {
                    overflow: Overflow::CLIP,
                    ..VisualStyle::default()
                })
                .with_child(RenderNode::container(LayoutBox::new(
                    10.0, 10.0, 20.0, 20.0,
                ))),
        ];

        assert_eq!(
            hit_test_element_path(&scene, 12.0, 12.0),
            Some(ElementPath {
                root: 0,
                children: vec![0],
            })
        );
        assert_eq!(hit_test_element_path(&scene, 28.0, 28.0), None);
    }

    #[test]
    fn resize_buffer_tracks_the_latest_window_size_without_scaling() {
        let mut width = 320;
        let mut height = 180;
        let mut buffer = vec![0_u32; width * height];

        resize_buffer(&mut buffer, &mut width, &mut height, 640, 360, Color::WHITE);

        assert_eq!(width, 640);
        assert_eq!(height, 360);
        assert_eq!(buffer.len(), 640 * 360);
    }

    #[test]
    fn window_options_enable_native_resizing() {
        assert!(window_options().resize);
    }

    #[test]
    fn visual_scene_comparison_ignores_click_handlers() {
        let left = vec![
            RenderNode::container(LayoutBox::new(4.0, 6.0, 40.0, 24.0))
                .on_click(increment_click_count),
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
        let scene = vec![RenderNode::container(LayoutBox::new(4.0, 6.0, 40.0, 24.0))];

        assert!(should_present_scene(Some(&scene), &scene, true));
    }

    #[test]
    fn super_dragging_suspends_updates() {
        assert!(should_suspend_updates(true, true, false));
        assert!(should_suspend_updates(true, false, true));
        assert!(should_suspend_updates(true, true, true));
        assert!(!should_suspend_updates(false, true, false));
        assert!(!should_suspend_updates(true, false, false));
    }

    #[test]
    fn drawable_viewport_size_skips_minimized_windows() {
        assert_eq!(drawable_viewport_size(0, 0), None);
        assert_eq!(drawable_viewport_size(0, 720), None);
        assert_eq!(drawable_viewport_size(1280, 0), None);
        assert_eq!(
            drawable_viewport_size(1280, 720),
            Some(ViewportSize::new(1280, 720))
        );
    }

    #[test]
    fn window_config_enables_middle_button_auto_scroll_by_default() {
        let config = WindowConfig::new("cssimpler", 960, 540);

        assert!(config.middle_button_auto_scroll);
    }

    #[test]
    fn auto_scroll_indicator_changes_force_a_present() {
        let scene = vec![RenderNode::container(LayoutBox::new(4.0, 6.0, 40.0, 24.0))];

        assert!(should_present_frame(
            Some(&scene),
            &scene,
            None,
            Some(crate::scrollbar::AutoScrollIndicator { x: 24.0, y: 18.0 }),
            false,
        ));
    }

    #[test]
    fn incremental_render_matches_full_render_when_a_node_moves() {
        let previous = vec![
            RenderNode::container(LayoutBox::new(2.0, 2.0, 4.0, 4.0)).with_style(VisualStyle {
                background: Some(Color::rgb(40, 120, 220)),
                ..VisualStyle::default()
            }),
        ];
        let next = vec![
            RenderNode::container(LayoutBox::new(10.0, 2.0, 4.0, 4.0)).with_style(VisualStyle {
                background: Some(Color::rgb(40, 120, 220)),
                ..VisualStyle::default()
            }),
        ];
        let mut incremental = vec![0_u32; 20 * 20];
        let mut full = vec![0_u32; 20 * 20];

        render_to_buffer(&previous, &mut incremental, 20, 20, Color::WHITE);
        render_scene_update(&previous, &next, &mut incremental, 20, 20, Color::WHITE);
        render_to_buffer(&next, &mut full, 20, 20, Color::WHITE);

        assert_eq!(incremental, full);
    }

    #[test]
    fn dirty_regions_collapse_to_a_branch_when_many_children_change_inside_it() {
        let previous = vec![
            RenderNode::container(LayoutBox::new(0.0, 0.0, 100.0, 100.0))
                .with_child(
                    RenderNode::container(LayoutBox::new(0.0, 0.0, 50.0, 50.0)).with_style(
                        VisualStyle {
                            background: Some(Color::rgb(20, 20, 20)),
                            ..VisualStyle::default()
                        },
                    ),
                )
                .with_child(
                    RenderNode::container(LayoutBox::new(50.0, 0.0, 50.0, 50.0)).with_style(
                        VisualStyle {
                            background: Some(Color::rgb(20, 20, 20)),
                            ..VisualStyle::default()
                        },
                    ),
                )
                .with_child(
                    RenderNode::container(LayoutBox::new(0.0, 50.0, 50.0, 50.0)).with_style(
                        VisualStyle {
                            background: Some(Color::rgb(20, 20, 20)),
                            ..VisualStyle::default()
                        },
                    ),
                )
                .with_child(
                    RenderNode::container(LayoutBox::new(50.0, 50.0, 50.0, 50.0)).with_style(
                        VisualStyle {
                            background: Some(Color::rgb(20, 20, 20)),
                            ..VisualStyle::default()
                        },
                    ),
                ),
        ];

        let next = vec![
            RenderNode::container(LayoutBox::new(0.0, 0.0, 100.0, 100.0))
                .with_child(
                    RenderNode::container(LayoutBox::new(0.0, 0.0, 50.0, 50.0)).with_style(
                        VisualStyle {
                            background: Some(Color::rgb(220, 80, 80)),
                            ..VisualStyle::default()
                        },
                    ),
                )
                .with_child(
                    RenderNode::container(LayoutBox::new(50.0, 0.0, 50.0, 50.0)).with_style(
                        VisualStyle {
                            background: Some(Color::rgb(80, 220, 80)),
                            ..VisualStyle::default()
                        },
                    ),
                )
                .with_child(
                    RenderNode::container(LayoutBox::new(0.0, 50.0, 50.0, 50.0)).with_style(
                        VisualStyle {
                            background: Some(Color::rgb(80, 80, 220)),
                            ..VisualStyle::default()
                        },
                    ),
                )
                .with_child(
                    RenderNode::container(LayoutBox::new(50.0, 50.0, 50.0, 50.0)).with_style(
                        VisualStyle {
                            background: Some(Color::rgb(20, 20, 20)),
                            ..VisualStyle::default()
                        },
                    ),
                ),
        ];

        let dirty_regions = dirty_regions_between_scenes(&previous, &next);

        assert_eq!(dirty_regions.len(), 1);
        let region = dirty_regions[0];
        assert_eq!(region.x0, 0.0);
        assert_eq!(region.y0, 0.0);
        assert_eq!(region.x1, 100.0);
        assert_eq!(region.y1, 100.0);
    }

    #[test]
    fn dirty_regions_do_not_collapse_to_a_huge_parent_when_children_are_sparse() {
        let previous =
            vec![
                RenderNode::container(LayoutBox::new(0.0, 0.0, 1000.0, 1000.0))
                    .with_child(
                        RenderNode::container(LayoutBox::new(0.0, 0.0, 100.0, 100.0)).with_style(
                            VisualStyle {
                                background: Some(Color::rgb(20, 20, 20)),
                                ..VisualStyle::default()
                            },
                        ),
                    )
                    .with_child(
                        RenderNode::container(LayoutBox::new(450.0, 450.0, 100.0, 100.0))
                            .with_style(VisualStyle {
                                background: Some(Color::rgb(20, 20, 20)),
                                ..VisualStyle::default()
                            }),
                    )
                    .with_child(
                        RenderNode::container(LayoutBox::new(900.0, 900.0, 100.0, 100.0))
                            .with_style(VisualStyle {
                                background: Some(Color::rgb(20, 20, 20)),
                                ..VisualStyle::default()
                            }),
                    ),
            ];

        let next =
            vec![
                RenderNode::container(LayoutBox::new(0.0, 0.0, 1000.0, 1000.0))
                    .with_child(
                        RenderNode::container(LayoutBox::new(0.0, 0.0, 100.0, 100.0)).with_style(
                            VisualStyle {
                                background: Some(Color::rgb(220, 80, 80)),
                                ..VisualStyle::default()
                            },
                        ),
                    )
                    .with_child(
                        RenderNode::container(LayoutBox::new(450.0, 450.0, 100.0, 100.0))
                            .with_style(VisualStyle {
                                background: Some(Color::rgb(80, 220, 80)),
                                ..VisualStyle::default()
                            }),
                    )
                    .with_child(
                        RenderNode::container(LayoutBox::new(900.0, 900.0, 100.0, 100.0))
                            .with_style(VisualStyle {
                                background: Some(Color::rgb(80, 80, 220)),
                                ..VisualStyle::default()
                            }),
                    ),
            ];

        let dirty_regions = dirty_regions_between_scenes(&previous, &next);

        assert_eq!(dirty_regions.len(), 3);
    }

    #[test]
    fn self_only_visual_changes_can_tighten_dirty_regions_to_own_bounds() {
        let previous = vec![
            RenderNode::container(LayoutBox::new(20.0, 20.0, 80.0, 60.0))
                .with_style(VisualStyle {
                    background: Some(Color::rgb(20, 20, 20)),
                    ..VisualStyle::default()
                })
                .with_child(
                    RenderNode::container(LayoutBox::new(140.0, 0.0, 40.0, 40.0)).with_style(
                        VisualStyle {
                            background: Some(Color::rgb(40, 120, 220)),
                            ..VisualStyle::default()
                        },
                    ),
                ),
        ];
        let next = vec![
            RenderNode::container(LayoutBox::new(20.0, 20.0, 80.0, 60.0))
                .with_style(VisualStyle {
                    background: Some(Color::rgb(220, 80, 80)),
                    ..VisualStyle::default()
                })
                .with_child(
                    RenderNode::container(LayoutBox::new(140.0, 0.0, 40.0, 40.0)).with_style(
                        VisualStyle {
                            background: Some(Color::rgb(40, 120, 220)),
                            ..VisualStyle::default()
                        },
                    ),
                ),
        ];

        let dirty_regions = dirty_regions_between_scenes(&previous, &next);

        assert_eq!(dirty_regions.len(), 1);
        assert_eq!(
            dirty_regions[0],
            ClipRect {
                x0: 20.0,
                y0: 20.0,
                x1: 100.0,
                y1: 80.0,
            }
        );
    }

    #[test]
    fn overflow_changes_keep_dirty_regions_at_the_subtree_union() {
        let previous = vec![
            RenderNode::container(LayoutBox::new(20.0, 20.0, 80.0, 60.0))
                .with_style(VisualStyle {
                    background: Some(Color::rgb(20, 20, 20)),
                    ..VisualStyle::default()
                })
                .with_child(
                    RenderNode::container(LayoutBox::new(140.0, 0.0, 40.0, 40.0)).with_style(
                        VisualStyle {
                            background: Some(Color::rgb(40, 120, 220)),
                            ..VisualStyle::default()
                        },
                    ),
                ),
        ];
        let next = vec![
            RenderNode::container(LayoutBox::new(20.0, 20.0, 80.0, 60.0))
                .with_style(VisualStyle {
                    background: Some(Color::rgb(20, 20, 20)),
                    overflow: Overflow::CLIP,
                    ..VisualStyle::default()
                })
                .with_child(
                    RenderNode::container(LayoutBox::new(140.0, 0.0, 40.0, 40.0)).with_style(
                        VisualStyle {
                            background: Some(Color::rgb(40, 120, 220)),
                            ..VisualStyle::default()
                        },
                    ),
                ),
        ];

        let dirty_regions = dirty_regions_between_scenes(&previous, &next);

        assert_eq!(dirty_regions.len(), 1);
        assert_eq!(
            dirty_regions[0],
            ClipRect {
                x0: 20.0,
                y0: 0.0,
                x1: 180.0,
                y1: 80.0,
            }
        );
    }

    #[test]
    fn coalescing_keeps_diagonal_regions_separate_when_the_bounding_box_bloats() {
        let mut dirty_regions = vec![
            ClipRect {
                x0: 0.0,
                y0: 0.0,
                x1: 40.0,
                y1: 40.0,
            },
            ClipRect {
                x0: 30.0,
                y0: 30.0,
                x1: 70.0,
                y1: 70.0,
            },
        ];

        coalesce_dirty_regions(&mut dirty_regions);

        assert_eq!(dirty_regions.len(), 2);
    }

    #[test]
    fn incremental_render_jobs_split_a_tall_dirty_region_into_bands() {
        let jobs = build_incremental_render_jobs(
            &[ClipRect {
                x0: 10.0,
                y0: 12.0,
                x1: 110.0,
                y1: 252.0,
            }],
            160,
            320,
        );

        assert_eq!(jobs.len(), 3);
        assert_eq!(jobs[0].pixel_count, 100 * 80);
        assert_eq!(jobs[1].pixel_count, 100 * 80);
        assert_eq!(jobs[2].pixel_count, 100 * 80);
    }

    #[test]
    fn incremental_render_job_distribution_keeps_worker_loads_close() {
        let jobs = vec![
            DirtyRenderJob {
                clip: ClipRect {
                    x0: 0.0,
                    y0: 0.0,
                    x1: 120.0,
                    y1: 80.0,
                },
                pixel_count: 9600,
            },
            DirtyRenderJob {
                clip: ClipRect {
                    x0: 0.0,
                    y0: 80.0,
                    x1: 120.0,
                    y1: 160.0,
                },
                pixel_count: 9600,
            },
            DirtyRenderJob {
                clip: ClipRect {
                    x0: 120.0,
                    y0: 0.0,
                    x1: 200.0,
                    y1: 80.0,
                },
                pixel_count: 6400,
            },
            DirtyRenderJob {
                clip: ClipRect {
                    x0: 120.0,
                    y0: 80.0,
                    x1: 200.0,
                    y1: 160.0,
                },
                pixel_count: 6400,
            },
        ];

        let groups = distribute_dirty_render_jobs(&jobs, 2);
        let loads = groups
            .iter()
            .map(|group| group.iter().map(|job| job.pixel_count).sum::<usize>())
            .collect::<Vec<_>>();

        assert_eq!(groups.len(), 2);
        assert!(loads.iter().all(|&load| load >= 12800));
        assert!(loads.iter().all(|&load| load <= 16000));
    }

    #[test]
    fn clip_plans_only_include_roots_that_intersect_their_damage() {
        let scene = vec![
            RenderNode::container(LayoutBox::new(0.0, 0.0, 60.0, 60.0)).with_style(VisualStyle {
                background: Some(Color::rgb(220, 38, 38)),
                ..VisualStyle::default()
            }),
            RenderNode::container(LayoutBox::new(80.0, 0.0, 60.0, 60.0)).with_style(VisualStyle {
                background: Some(Color::rgb(37, 99, 235)),
                ..VisualStyle::default()
            }),
            RenderNode::container(LayoutBox::new(160.0, 0.0, 60.0, 60.0)).with_style(VisualStyle {
                background: Some(Color::rgb(22, 163, 74)),
                ..VisualStyle::default()
            }),
        ];
        let cached_bounds = super::cache_scene_subtree_bounds(&scene);
        let root_indices = super::root_indices_intersecting_clip(
            &cached_bounds.roots,
            ClipRect {
                x0: 70.0,
                y0: 0.0,
                x1: 150.0,
                y1: 80.0,
            },
        );
        let group_clip = super::dirty_job_group_clip(&[
            DirtyRenderJob {
                clip: ClipRect {
                    x0: 70.0,
                    y0: 0.0,
                    x1: 110.0,
                    y1: 40.0,
                },
                pixel_count: 1600,
            },
            DirtyRenderJob {
                clip: ClipRect {
                    x0: 100.0,
                    y0: 40.0,
                    x1: 150.0,
                    y1: 80.0,
                },
                pixel_count: 2000,
            },
        ])
        .expect("dirty job groups should produce a union clip");

        assert_eq!(root_indices, vec![1]);
        assert_eq!(
            group_clip,
            ClipRect {
                x0: 70.0,
                y0: 0.0,
                x1: 150.0,
                y1: 80.0,
            }
        );
    }

    #[test]
    fn incremental_render_can_parallelize_a_single_large_dirty_region() {
        let _shadow_cache_guard = super::shadow::lock_shadow_mask_cache_for_tests();
        let background =
            RenderNode::container(LayoutBox::new(0.0, 0.0, 400.0, 400.0)).with_style(VisualStyle {
                background: Some(Color::rgb(18, 24, 38)),
                ..VisualStyle::default()
            });
        let previous = vec![
            background.clone(),
            RenderNode::container(LayoutBox::new(24.0, 40.0, 320.0, 240.0)).with_style(
                VisualStyle {
                    background: Some(Color::rgb(40, 120, 220)),
                    shadows: vec![BoxShadow {
                        color: Color::rgba(40, 120, 220, 120),
                        offset_x: 0.0,
                        offset_y: 0.0,
                        blur_radius: 8.0,
                        spread: 0.0,
                    }],
                    ..VisualStyle::default()
                },
            ),
        ];
        let next = vec![
            background,
            RenderNode::container(LayoutBox::new(24.0, 40.0, 320.0, 240.0)).with_style(
                VisualStyle {
                    background: Some(Color::rgb(220, 120, 40)),
                    shadows: vec![BoxShadow {
                        color: Color::rgba(220, 120, 40, 120),
                        offset_x: 0.0,
                        offset_y: 0.0,
                        blur_radius: 8.0,
                        spread: 0.0,
                    }],
                    ..VisualStyle::default()
                },
            ),
        ];
        let mut incremental = vec![0_u32; 400 * 400];
        let mut full = vec![0_u32; 400 * 400];

        render_to_buffer(&previous, &mut incremental, 400, 400, Color::WHITE);
        let stats = render_scene_update_internal(
            &previous,
            &next,
            &mut incremental,
            400,
            400,
            Color::WHITE,
        );
        render_to_buffer(&next, &mut full, 400, 400, Color::WHITE);

        assert_eq!(incremental, full);
        assert_eq!(stats.mode, FramePaintMode::Incremental);
        assert_eq!(stats.reason, FramePaintReason::IncrementalDamage);
        assert_eq!(stats.damage_pixels, stats.painted_pixels);
        assert!(stats.damage_pixels > 0);
        assert_eq!(stats.scene_passes, stats.dirty_jobs);
        if thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1)
            > 1
        {
            assert!(stats.workers > 1);
        }
    }

    #[test]
    fn fragmented_sparse_damage_can_fall_back_to_a_full_redraw() {
        let background =
            RenderNode::container(LayoutBox::new(0.0, 0.0, 480.0, 640.0)).with_style(VisualStyle {
                background: Some(Color::rgb(15, 23, 42)),
                ..VisualStyle::default()
            });
        let mut previous = vec![background.clone()];
        let mut next = vec![background];

        for index in 0..180 {
            let column = index % 12;
            let row = index / 12;
            let x = 260.0 + column as f32 * 16.0;
            let y = 12.0 + row as f32 * 18.0;
            let node =
                RenderNode::container(LayoutBox::new(x, y, 8.0, 8.0)).with_style(VisualStyle {
                    background: Some(Color::rgb(30, 41, 59)),
                    ..VisualStyle::default()
                });
            previous.push(node.clone());
            next.push(node);
        }

        for index in 0..20 {
            let x = 8.0 + index as f32 * 12.0;
            previous.push(
                RenderNode::container(LayoutBox::new(x, 0.0, 6.0, 640.0)).with_style(VisualStyle {
                    background: Some(Color::rgb(37, 99, 235)),
                    ..VisualStyle::default()
                }),
            );
            next.push(
                RenderNode::container(LayoutBox::new(x, 0.0, 6.0, 640.0)).with_style(VisualStyle {
                    background: Some(Color::rgb(249, 115, 22)),
                    ..VisualStyle::default()
                }),
            );
        }

        let mut incremental = vec![0_u32; 480 * 640];
        let mut full = vec![0_u32; 480 * 640];

        render_to_buffer(&previous, &mut incremental, 480, 640, Color::WHITE);
        let stats = render_scene_update_internal(
            &previous,
            &next,
            &mut incremental,
            480,
            640,
            Color::WHITE,
        );
        render_to_buffer(&next, &mut full, 480, 640, Color::WHITE);

        assert_eq!(incremental, full);
        assert_eq!(stats.mode, FramePaintMode::Full);
        assert_eq!(stats.reason, FramePaintReason::FragmentedDamage);
        assert_eq!(stats.dirty_regions, 20);
        assert!(stats.dirty_jobs >= stats.dirty_regions);
        assert!(stats.damage_pixels < stats.painted_pixels);
        assert_eq!(stats.scene_passes, stats.workers);
    }

    #[test]
    fn incremental_render_clears_shadow_pixels_and_redraws_the_background() {
        let _shadow_cache_guard = super::shadow::lock_shadow_mask_cache_for_tests();
        let background =
            RenderNode::container(LayoutBox::new(0.0, 0.0, 20.0, 20.0)).with_style(VisualStyle {
                background: Some(Color::rgb(226, 232, 240)),
                ..VisualStyle::default()
            });
        let previous = vec![
            background.clone(),
            RenderNode::container(LayoutBox::new(6.0, 6.0, 4.0, 4.0)).with_style(VisualStyle {
                background: Some(Color::rgb(15, 23, 42)),
                shadows: vec![BoxShadow {
                    color: Color::rgba(15, 23, 42, 140),
                    offset_x: 3.0,
                    offset_y: 3.0,
                    blur_radius: 2.0,
                    spread: 0.0,
                }],
                ..VisualStyle::default()
            }),
        ];
        let next = vec![
            background,
            RenderNode::container(LayoutBox::new(6.0, 6.0, 4.0, 4.0)).with_style(VisualStyle {
                background: Some(Color::rgb(15, 23, 42)),
                ..VisualStyle::default()
            }),
        ];
        let mut incremental = vec![0_u32; 20 * 20];
        let mut full = vec![0_u32; 20 * 20];

        render_to_buffer(&previous, &mut incremental, 20, 20, Color::WHITE);
        render_scene_update(&previous, &next, &mut incremental, 20, 20, Color::WHITE);
        render_to_buffer(&next, &mut full, 20, 20, Color::WHITE);

        assert_eq!(incremental, full);
    }

    #[test]
    fn incremental_render_matches_full_render_over_a_cached_gradient_background() {
        let _cache_guard = crate::gradient::lock_gradient_cache_for_tests();
        crate::gradient::clear_gradient_layer_cache_for_tests();

        let background =
            RenderNode::container(LayoutBox::new(0.0, 0.0, 96.0, 64.0)).with_style(VisualStyle {
                corner_radius: CornerRadius::all(12.0),
                background_layers: vec![
                    BackgroundLayer::RadialGradient(RadialGradient {
                        shape: RadialShape::Circle(CircleRadius::Explicit(46.0)),
                        center: GradientPoint::CENTER,
                        interpolation: GradientInterpolation::Oklab,
                        repeating: false,
                        stops: vec![
                            GradientStop {
                                color: Color::rgba(255, 255, 255, 168),
                                position: LengthPercentageValue::from_fraction(0.0),
                            },
                            GradientStop {
                                color: Color::rgba(255, 255, 255, 0),
                                position: LengthPercentageValue::from_fraction(1.0),
                            },
                        ],
                    }),
                    BackgroundLayer::LinearGradient(LinearGradient {
                        direction: GradientDirection::Horizontal(GradientHorizontal::Right),
                        interpolation: GradientInterpolation::Oklab,
                        repeating: false,
                        stops: vec![
                            GradientStop {
                                color: Color::rgb(15, 23, 42),
                                position: LengthPercentageValue::from_fraction(0.0),
                            },
                            GradientStop {
                                color: Color::rgb(59, 130, 246),
                                position: LengthPercentageValue::from_fraction(1.0),
                            },
                        ],
                    }),
                ],
                ..VisualStyle::default()
            });
        let previous = vec![
            background.clone(),
            RenderNode::container(LayoutBox::new(14.0, 18.0, 18.0, 18.0)).with_style(VisualStyle {
                background: Some(Color::rgba(15, 23, 42, 220)),
                ..VisualStyle::default()
            }),
        ];
        let next = vec![
            background,
            RenderNode::container(LayoutBox::new(58.0, 18.0, 18.0, 18.0)).with_style(VisualStyle {
                background: Some(Color::rgba(15, 23, 42, 220)),
                ..VisualStyle::default()
            }),
        ];
        let mut incremental = vec![0_u32; 96 * 64];
        let mut full = vec![0_u32; 96 * 64];

        render_to_buffer(&previous, &mut incremental, 96, 64, Color::WHITE);
        render_scene_update(&previous, &next, &mut incremental, 96, 64, Color::WHITE);

        crate::gradient::clear_gradient_layer_cache_for_tests();
        render_to_buffer(&next, &mut full, 96, 64, Color::WHITE);

        assert_eq!(incremental, full);
    }

    #[test]
    fn incremental_render_matches_full_render_over_a_static_gradient_background() {
        let _cache_guard = crate::gradient::lock_gradient_cache_for_tests();
        crate::gradient::clear_gradient_layer_cache_for_tests();

        let background =
            RenderNode::container(LayoutBox::new(0.0, 0.0, 600.0, 420.0)).with_style(VisualStyle {
                corner_radius: CornerRadius::all(20.0),
                background_layers: vec![
                    BackgroundLayer::RadialGradient(RadialGradient {
                        shape: RadialShape::Circle(CircleRadius::Explicit(240.0)),
                        center: GradientPoint::CENTER,
                        interpolation: GradientInterpolation::Oklab,
                        repeating: false,
                        stops: vec![
                            GradientStop {
                                color: Color::rgba(255, 255, 255, 156),
                                position: LengthPercentageValue::from_fraction(0.0),
                            },
                            GradientStop {
                                color: Color::rgba(255, 255, 255, 0),
                                position: LengthPercentageValue::from_fraction(1.0),
                            },
                        ],
                    }),
                    BackgroundLayer::LinearGradient(LinearGradient {
                        direction: GradientDirection::Horizontal(GradientHorizontal::Right),
                        interpolation: GradientInterpolation::Oklab,
                        repeating: false,
                        stops: vec![
                            GradientStop {
                                color: Color::rgb(15, 23, 42),
                                position: LengthPercentageValue::from_fraction(0.0),
                            },
                            GradientStop {
                                color: Color::rgb(37, 99, 235),
                                position: LengthPercentageValue::from_fraction(1.0),
                            },
                        ],
                    }),
                ],
                ..VisualStyle::default()
            });
        let previous = vec![
            background.clone(),
            RenderNode::container(LayoutBox::new(64.0, 72.0, 72.0, 72.0)).with_style(VisualStyle {
                background: Some(Color::rgba(15, 23, 42, 220)),
                ..VisualStyle::default()
            }),
        ];
        let next = vec![
            background,
            RenderNode::container(LayoutBox::new(452.0, 72.0, 72.0, 72.0)).with_style(
                VisualStyle {
                    background: Some(Color::rgba(15, 23, 42, 220)),
                    ..VisualStyle::default()
                },
            ),
        ];
        let mut incremental = vec![0_u32; 600 * 420];
        let mut full = vec![0_u32; 600 * 420];

        render_to_buffer(&previous, &mut incremental, 600, 420, Color::WHITE);
        render_scene_update(&previous, &next, &mut incremental, 600, 420, Color::WHITE);

        crate::gradient::clear_gradient_layer_cache_for_tests();
        render_to_buffer(&next, &mut full, 600, 420, Color::WHITE);

        assert_eq!(incremental, full);
    }

    #[test]
    fn incremental_render_redraws_pixels_cleared_by_fractional_dirty_regions() {
        let background =
            RenderNode::container(LayoutBox::new(0.0, 0.0, 32.0, 16.0)).with_style(VisualStyle {
                background: Some(Color::rgb(226, 232, 240)),
                ..VisualStyle::default()
            });
        let static_bar =
            RenderNode::container(LayoutBox::new(6.0, 4.0, 20.0, 8.0)).with_style(VisualStyle {
                background: Some(Color::rgb(15, 23, 42)),
                ..VisualStyle::default()
            });
        let previous = vec![
            background.clone(),
            static_bar.clone(),
            RenderNode::container(LayoutBox::new(10.0, 0.0, 7.2, 16.0)),
        ];
        let next = vec![
            background,
            static_bar,
            RenderNode::container(LayoutBox::new(10.0, 0.0, 11.6, 16.0)),
        ];
        let mut incremental = vec![0_u32; 32 * 16];
        let mut full = vec![0_u32; 32 * 16];

        render_to_buffer(&previous, &mut incremental, 32, 16, Color::WHITE);
        render_scene_update(&previous, &next, &mut incremental, 32, 16, Color::WHITE);
        render_to_buffer(&next, &mut full, 32, 16, Color::WHITE);

        assert_eq!(incremental, full);
    }

    #[test]
    fn incremental_render_matches_full_render_for_fractional_dirty_regions_over_text() {
        let _text_cache_guard = super::fonts::lock_text_mask_cache_for_tests();
        let background =
            RenderNode::container(LayoutBox::new(0.0, 0.0, 160.0, 80.0)).with_style(VisualStyle {
                background: Some(Color::rgb(245, 247, 250)),
                ..VisualStyle::default()
            });
        let text = RenderNode::text(LayoutBox::new(24.0, 20.0, 112.0, 36.0), "UIVERSE").with_style(
            VisualStyle {
                foreground: Color::rgba(255, 255, 255, 0),
                text: TextStyle {
                    size_px: 32.0,
                    ..TextStyle::default()
                },
                text_stroke: TextStrokeStyle {
                    width: 1.0,
                    color: Some(Color::rgb(15, 23, 42)),
                },
                ..VisualStyle::default()
            },
        );
        let previous = vec![
            background.clone(),
            text.clone(),
            RenderNode::container(LayoutBox::new(0.0, 0.0, 53.2, 80.0)),
        ];
        let next = vec![
            background,
            text,
            RenderNode::container(LayoutBox::new(0.0, 0.0, 57.7, 80.0)),
        ];
        let mut incremental = vec![0_u32; 160 * 80];
        let mut full = vec![0_u32; 160 * 80];

        render_to_buffer(&previous, &mut incremental, 160, 80, Color::WHITE);
        render_scene_update(&previous, &next, &mut incremental, 160, 80, Color::WHITE);
        render_to_buffer(&next, &mut full, 160, 80, Color::WHITE);

        assert_eq!(incremental, full);
    }

    #[test]
    fn incremental_render_does_not_leave_stripes_after_a_fractional_reveal_sweeps_over_text() {
        let _text_cache_guard = super::fonts::lock_text_mask_cache_for_tests();
        fn scene(reveal_width: f32) -> Vec<RenderNode> {
            vec![
                RenderNode::container(LayoutBox::new(0.0, 0.0, 200.0, 96.0)).with_style(
                    VisualStyle {
                        background: Some(Color::rgb(38, 38, 38)),
                        ..VisualStyle::default()
                    },
                ),
                RenderNode::text(LayoutBox::new(28.0, 28.0, 144.0, 40.0), "UIVERSE").with_style(
                    VisualStyle {
                        foreground: Color::rgba(255, 255, 255, 0),
                        text: TextStyle {
                            size_px: 36.0,
                            ..TextStyle::default()
                        },
                        text_stroke: TextStrokeStyle {
                            width: 1.0,
                            color: Some(Color::rgb(240, 240, 240)),
                        },
                        ..VisualStyle::default()
                    },
                ),
                RenderNode::container(LayoutBox::new(28.0, 20.0, reveal_width, 56.0)).with_style(
                    VisualStyle {
                        overflow: Overflow::CLIP,
                        ..VisualStyle::default()
                    },
                ),
            ]
        }

        let widths = [4.2, 21.4, 39.8, 58.1, 76.6, 95.2, 63.7, 27.3, 4.2];
        let mut incremental = vec![0_u32; 200 * 96];
        let mut previous = scene(widths[0]);
        render_to_buffer(&previous, &mut incremental, 200, 96, Color::WHITE);

        for width in widths.iter().copied().skip(1) {
            let next = scene(width);
            render_scene_update(&previous, &next, &mut incremental, 200, 96, Color::WHITE);
            previous = next;
        }

        let final_scene = scene(*widths.last().expect("sequence should not be empty"));
        let mut full = vec![0_u32; 200 * 96];
        render_to_buffer(&final_scene, &mut full, 200, 96, Color::WHITE);

        assert_eq!(incremental, full);
    }
}
