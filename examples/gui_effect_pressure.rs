use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::Result;
use cssimpler::app::{App, Invalidation, Refresh, RuntimeStats, latest_runtime_stats};
use cssimpler::core::Node;
use cssimpler::renderer::{
    FrameInfo, FramePaintMode, FramePaintReason, FrameTimingStats, WindowConfig,
    latest_frame_timing_stats,
};
use cssimpler::style::{Stylesheet, parse_stylesheet};
use cssimpler::ui;

const ACTION_ADD_TILES: u64 = 1 << 0;
const ACTION_REMOVE_TILES: u64 = 1 << 1;
const ACTION_ADD_PASSES: u64 = 1 << 2;
const ACTION_ADD_ANIMATED_TILES: u64 = 1 << 3;
const ACTION_REMOVE_ANIMATED_TILES: u64 = 1 << 4;
const ACTION_TOGGLE_ANIMATION: u64 = 1 << 5;
const ACTION_TOGGLE_PULSE: u64 = 1 << 6;
const ACTION_SPIKE: u64 = 1 << 7;
const ACTION_RESET: u64 = 1 << 8;
const ACTION_TOGGLE_BASELINE: u64 = 1 << 9;
const MANUAL_ACTION_MASK: u64 = ACTION_ADD_TILES
    | ACTION_REMOVE_TILES
    | ACTION_ADD_PASSES
    | ACTION_ADD_ANIMATED_TILES
    | ACTION_REMOVE_ANIMATED_TILES
    | ACTION_TOGGLE_ANIMATION
    | ACTION_TOGGLE_PULSE
    | ACTION_SPIKE
    | ACTION_RESET;

const PHASE_COUNT: usize = 3;
const PHASE_STEP_INTERVAL: Duration = Duration::from_millis(120);
const PERF_LOG_INTERVAL: Duration = Duration::from_secs(1);
pub const MAX_TILE_COUNT: usize = 48;
const ACTIVE_PODS_PER_ANIMATED_TILE: usize = 2;
const DEFAULT_ANIMATED_TILE_WINDOW: usize = 2;
const DEFAULT_TILE_COUNT: usize = 12;
const DEFAULT_PASSES_PER_TILE: usize = 4;
const ANIMATED_TILE_STEP: usize = 1;
const TILE_STEP: usize = 8;
const PASS_STEP: usize = 2;
const SPIKE_TILE_STEP: usize = 16;
const SPIKE_PASS_STEP: usize = 4;
const BASELINE_TILE_COUNT: usize = DEFAULT_TILE_COUNT + SPIKE_TILE_STEP;
const BASELINE_PASSES_PER_TILE: usize = DEFAULT_PASSES_PER_TILE + SPIKE_PASS_STEP;
const BASELINE_ANIMATED_TILE_WINDOW: usize = DEFAULT_ANIMATED_TILE_WINDOW + 1;
const BASELINE_WARMUP_FRAMES: u32 = 30;
const BASELINE_SAMPLE_FRAMES: u32 = 120;

static ACTIONS: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectStressState {
    frame_index: u64,
    last_frame_ms: u128,
    tile_count: usize,
    passes_per_tile: usize,
    animated_tile_window: usize,
    animate: bool,
    pulse_layout: bool,
    phase: usize,
    phase_elapsed: Duration,
    animation_band_start: usize,
    log_elapsed: Duration,
    renderer_stats: FrameTimingStats,
    app_stats: RuntimeStats,
    baseline_harness: Option<BaselineHarness>,
    last_baseline_summary: Option<BaselineSummary>,
}

impl Default for EffectStressState {
    fn default() -> Self {
        Self {
            frame_index: 0,
            last_frame_ms: 0,
            tile_count: DEFAULT_TILE_COUNT,
            passes_per_tile: DEFAULT_PASSES_PER_TILE,
            animated_tile_window: DEFAULT_ANIMATED_TILE_WINDOW,
            animate: true,
            pulse_layout: false,
            phase: 0,
            phase_elapsed: Duration::ZERO,
            animation_band_start: 0,
            log_elapsed: Duration::ZERO,
            renderer_stats: FrameTimingStats::default(),
            app_stats: RuntimeStats::default(),
            baseline_harness: None,
            last_baseline_summary: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct BaselineHarness {
    scenario: BaselineScenario,
    warmup_frames: u32,
    sample_frames: u32,
    warmup_frames_remaining: u32,
    sample_frames_collected: u32,
    accumulator: BaselineAccumulator,
    completed: Vec<BaselineScenarioSummary>,
}

impl BaselineHarness {
    fn new() -> Self {
        Self::new_with_limits(BASELINE_WARMUP_FRAMES, BASELINE_SAMPLE_FRAMES)
    }

    fn new_with_limits(warmup_frames: u32, sample_frames: u32) -> Self {
        Self {
            scenario: BaselineScenario::Idle,
            warmup_frames,
            sample_frames: sample_frames.max(1),
            warmup_frames_remaining: warmup_frames,
            sample_frames_collected: 0,
            accumulator: BaselineAccumulator::default(),
            completed: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum BaselineScenario {
    #[default]
    Idle,
    AnimatedPaint,
    PulseLayout,
}

impl BaselineScenario {
    const ALL: [Self; 3] = [Self::Idle, Self::AnimatedPaint, Self::PulseLayout];

    fn label(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::AnimatedPaint => "animated-paint",
            Self::PulseLayout => "pulse-layout",
        }
    }

    fn animates(self) -> bool {
        !matches!(self, Self::Idle)
    }

    fn pulses_layout(self) -> bool {
        matches!(self, Self::PulseLayout)
    }

    fn next(self) -> Option<Self> {
        match self {
            Self::Idle => Some(Self::AnimatedPaint),
            Self::AnimatedPaint => Some(Self::PulseLayout),
            Self::PulseLayout => None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct BaselineAccumulator {
    frame_count: u32,
    render_tree_us_total: u64,
    scene_prep_us_total: u64,
    paint_us_total: u64,
    present_us_total: u64,
    total_us_total: u64,
    max_paint_us: u64,
    max_total_us: u64,
    idle_frames: u32,
    full_frames: u32,
    incremental_frames: u32,
}

impl BaselineAccumulator {
    fn observe(&mut self, state: &EffectStressState) {
        self.frame_count = self.frame_count.saturating_add(1);
        self.render_tree_us_total = self
            .render_tree_us_total
            .saturating_add(state.app_stats.render_tree_us);
        self.scene_prep_us_total = self
            .scene_prep_us_total
            .saturating_add(state.renderer_stats.scene_prep_us);
        self.paint_us_total = self
            .paint_us_total
            .saturating_add(state.renderer_stats.paint_us);
        self.present_us_total = self
            .present_us_total
            .saturating_add(state.renderer_stats.present_us);
        self.total_us_total = self
            .total_us_total
            .saturating_add(state.renderer_stats.total_us);
        self.max_paint_us = self.max_paint_us.max(state.renderer_stats.paint_us);
        self.max_total_us = self.max_total_us.max(state.renderer_stats.total_us);
        match state.renderer_stats.paint_mode {
            FramePaintMode::Idle => self.idle_frames = self.idle_frames.saturating_add(1),
            FramePaintMode::Full => self.full_frames = self.full_frames.saturating_add(1),
            FramePaintMode::Incremental => {
                self.incremental_frames = self.incremental_frames.saturating_add(1);
            }
        }
    }

    fn finish(&self, scenario: BaselineScenario) -> BaselineScenarioSummary {
        BaselineScenarioSummary {
            scenario,
            frames: self.frame_count.max(1),
            avg_render_tree_us: self.render_tree_us_total / u64::from(self.frame_count.max(1)),
            avg_scene_prep_us: self.scene_prep_us_total / u64::from(self.frame_count.max(1)),
            avg_paint_us: self.paint_us_total / u64::from(self.frame_count.max(1)),
            avg_present_us: self.present_us_total / u64::from(self.frame_count.max(1)),
            avg_total_us: self.total_us_total / u64::from(self.frame_count.max(1)),
            max_paint_us: self.max_paint_us,
            max_total_us: self.max_total_us,
            idle_frames: self.idle_frames,
            full_frames: self.full_frames,
            incremental_frames: self.incremental_frames,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct BaselineScenarioSummary {
    scenario: BaselineScenario,
    frames: u32,
    avg_render_tree_us: u64,
    avg_scene_prep_us: u64,
    avg_paint_us: u64,
    avg_present_us: u64,
    avg_total_us: u64,
    max_paint_us: u64,
    max_total_us: u64,
    idle_frames: u32,
    full_frames: u32,
    incremental_frames: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct BaselineSummary {
    tile_count: usize,
    passes_per_tile: usize,
    animated_tile_window: usize,
    scenarios: Vec<BaselineScenarioSummary>,
}

fn main() -> Result<()> {
    let config = WindowConfig::new("cssimpler / gui effect pressure", 1440, 960);
    if baseline_autostart_requested() {
        ACTIONS.fetch_or(ACTION_TOGGLE_BASELINE, Ordering::Relaxed);
    }

    App::new(EffectStressState::default(), stylesheet(), update, build_ui)
        .run(config)
        .map_err(Into::into)
}

fn update(state: &mut EffectStressState, frame: FrameInfo) -> Refresh {
    let actions = ACTIONS.swap(0, Ordering::Relaxed);
    let previous = state.clone();
    let invalidation = apply_frame(state, frame, actions);
    maybe_log_perf(state, actions);
    normal_app_refresh(&previous, state, invalidation)
}

pub fn apply_frame(state: &mut EffectStressState, frame: FrameInfo, actions: u64) -> Invalidation {
    state.frame_index = frame.frame_index;
    state.last_frame_ms = frame.delta.as_millis();
    state.renderer_stats = latest_frame_timing_stats();
    state.app_stats = latest_runtime_stats();
    state.log_elapsed += frame.delta;

    if actions & ACTION_RESET != 0 {
        *state = EffectStressState {
            frame_index: frame.frame_index,
            last_frame_ms: frame.delta.as_millis(),
            renderer_stats: latest_frame_timing_stats(),
            app_stats: latest_runtime_stats(),
            ..EffectStressState::default()
        };
        return Invalidation::Layout;
    }

    let mut invalidation = Invalidation::Clean;
    let baseline_toggle_requested = actions & ACTION_TOGGLE_BASELINE != 0;
    let manual_actions = actions & MANUAL_ACTION_MASK;

    if state.baseline_harness.is_some() && manual_actions != 0 && !baseline_toggle_requested {
        cancel_baseline_harness(state, "manual override");
    }

    if baseline_toggle_requested {
        if state.baseline_harness.is_some() {
            cancel_baseline_harness(state, "stopped");
        } else {
            invalidation = invalidation.max(start_baseline_harness(state));
        }
    }

    if actions & ACTION_ADD_TILES != 0 {
        state.tile_count = (state.tile_count.saturating_add(TILE_STEP)).min(MAX_TILE_COUNT);
        invalidation = Invalidation::Layout;
    }

    if actions & ACTION_REMOVE_TILES != 0 {
        state.tile_count = state.tile_count.saturating_sub(TILE_STEP);
        invalidation = Invalidation::Layout;
    }

    if actions & ACTION_ADD_PASSES != 0 {
        state.passes_per_tile = state.passes_per_tile.saturating_add(PASS_STEP);
        invalidation = Invalidation::Layout;
    }

    if actions & ACTION_ADD_ANIMATED_TILES != 0 {
        state.animated_tile_window = state
            .animated_tile_window
            .saturating_add(ANIMATED_TILE_STEP)
            .min(MAX_TILE_COUNT);
        invalidation = invalidation.max(Invalidation::Paint);
    }

    if actions & ACTION_REMOVE_ANIMATED_TILES != 0 {
        state.animated_tile_window = state
            .animated_tile_window
            .saturating_sub(ANIMATED_TILE_STEP);
        invalidation = invalidation.max(Invalidation::Paint);
    }

    if actions & ACTION_TOGGLE_PULSE != 0 {
        state.pulse_layout = !state.pulse_layout;
        invalidation = Invalidation::Layout;
    }

    if actions & ACTION_SPIKE != 0 {
        state.tile_count = (state.tile_count.saturating_add(SPIKE_TILE_STEP)).min(MAX_TILE_COUNT);
        state.passes_per_tile = state.passes_per_tile.saturating_add(SPIKE_PASS_STEP);
        state.animate = true;
        state.pulse_layout = true;
        invalidation = Invalidation::Layout;
    }

    if actions & ACTION_TOGGLE_ANIMATION != 0 {
        state.animate = !state.animate;
        state.phase_elapsed = Duration::ZERO;
        invalidation = invalidation.max(Invalidation::Paint);
    }

    invalidation = invalidation.max(advance_baseline_harness(state));

    if !state.animate || state.tile_count == 0 {
        state.phase_elapsed = Duration::ZERO;
        return invalidation;
    }

    state.phase_elapsed += frame.delta;
    let tick_count = elapsed_tick_count(state.phase_elapsed);
    if tick_count == 0 {
        return invalidation;
    }

    state.phase_elapsed = remaining_phase_elapsed(state.phase_elapsed);
    state.phase = (state.phase + tick_count) % PHASE_COUNT;
    state.animation_band_start = (state.animation_band_start + tick_count) % state.tile_count;

    invalidation.max(animation_invalidation(state))
}

fn baseline_autostart_requested() -> bool {
    matches!(
        std::env::var("CSSIMPLER_PRESSURE_BASELINE").ok().as_deref(),
        Some("1" | "true" | "TRUE" | "yes" | "YES")
    )
}

fn start_baseline_harness(state: &mut EffectStressState) -> Invalidation {
    let harness = BaselineHarness::new();
    let invalidation = apply_baseline_scenario(state, harness.scenario);
    eprintln!(
        "[gui_effect_pressure_baseline] status=start preset={}t/{}p/{}live warmup={} sample={} scenario={}",
        BASELINE_TILE_COUNT,
        BASELINE_PASSES_PER_TILE,
        BASELINE_ANIMATED_TILE_WINDOW,
        harness.warmup_frames,
        harness.sample_frames,
        harness.scenario.label(),
    );
    state.baseline_harness = Some(harness);
    state.last_baseline_summary = None;
    invalidation
}

fn cancel_baseline_harness(state: &mut EffectStressState, reason: &str) {
    let Some(harness) = state.baseline_harness.take() else {
        return;
    };
    eprintln!(
        "[gui_effect_pressure_baseline] status=cancel reason={} scenario={} warmup_left={} samples={}/{}",
        reason,
        harness.scenario.label(),
        harness.warmup_frames_remaining,
        harness.sample_frames_collected,
        harness.sample_frames,
    );
}

fn advance_baseline_harness(state: &mut EffectStressState) -> Invalidation {
    let Some(mut harness) = state.baseline_harness.take() else {
        return Invalidation::Clean;
    };

    if harness.warmup_frames_remaining > 0 {
        harness.warmup_frames_remaining = harness.warmup_frames_remaining.saturating_sub(1);
        state.baseline_harness = Some(harness);
        return Invalidation::Clean;
    }

    harness.accumulator.observe(state);
    harness.sample_frames_collected = harness.sample_frames_collected.saturating_add(1);

    if harness.sample_frames_collected < harness.sample_frames {
        state.baseline_harness = Some(harness);
        return Invalidation::Clean;
    }

    let summary = harness.accumulator.finish(harness.scenario);
    log_baseline_scenario_summary(&summary);
    harness.completed.push(summary);

    if let Some(next_scenario) = harness.scenario.next() {
        harness.scenario = next_scenario;
        harness.warmup_frames_remaining = harness.warmup_frames;
        harness.sample_frames_collected = 0;
        harness.accumulator = BaselineAccumulator::default();
        let invalidation = apply_baseline_scenario(state, next_scenario);
        eprintln!(
            "[gui_effect_pressure_baseline] status=transition scenario={}",
            next_scenario.label(),
        );
        state.baseline_harness = Some(harness);
        return invalidation;
    }

    let report = BaselineSummary {
        tile_count: BASELINE_TILE_COUNT,
        passes_per_tile: BASELINE_PASSES_PER_TILE,
        animated_tile_window: BASELINE_ANIMATED_TILE_WINDOW,
        scenarios: harness.completed,
    };
    log_baseline_completion(&report);
    state.last_baseline_summary = Some(report);
    Invalidation::Clean
}

fn apply_baseline_scenario(
    state: &mut EffectStressState,
    scenario: BaselineScenario,
) -> Invalidation {
    let previous_tile_count = state.tile_count;
    let previous_passes_per_tile = state.passes_per_tile;
    let previous_animated_tile_window = state.animated_tile_window;
    let previous_animate = state.animate;
    let previous_pulse_layout = state.pulse_layout;
    let previous_phase = state.phase;
    let previous_band_start = state.animation_band_start;

    state.tile_count = BASELINE_TILE_COUNT;
    state.passes_per_tile = BASELINE_PASSES_PER_TILE;
    state.animated_tile_window = BASELINE_ANIMATED_TILE_WINDOW;
    state.animate = scenario.animates();
    state.pulse_layout = scenario.pulses_layout();
    state.phase = 0;
    state.phase_elapsed = Duration::ZERO;
    state.animation_band_start = 0;

    if previous_tile_count != state.tile_count
        || previous_passes_per_tile != state.passes_per_tile
        || previous_animated_tile_window != state.animated_tile_window
        || previous_pulse_layout != state.pulse_layout
    {
        Invalidation::Layout
    } else if previous_animate != state.animate
        || previous_phase != state.phase
        || previous_band_start != state.animation_band_start
    {
        Invalidation::Paint
    } else {
        Invalidation::Clean
    }
}

fn log_baseline_scenario_summary(summary: &BaselineScenarioSummary) {
    eprintln!(
        "[gui_effect_pressure_baseline] scenario={} frames={} avg_tree={} avg_scene_prep={} avg_paint={} avg_present={} avg_total={} max_paint={} max_total={} modes=idle:{}/full:{}/incremental:{}",
        summary.scenario.label(),
        summary.frames,
        format_us(summary.avg_render_tree_us),
        format_us(summary.avg_scene_prep_us),
        format_us(summary.avg_paint_us),
        format_us(summary.avg_present_us),
        format_us(summary.avg_total_us),
        format_us(summary.max_paint_us),
        format_us(summary.max_total_us),
        summary.idle_frames,
        summary.full_frames,
        summary.incremental_frames,
    );
}

fn log_baseline_completion(summary: &BaselineSummary) {
    let scenarios = summary
        .scenarios
        .iter()
        .map(|scenario| scenario.scenario.label())
        .collect::<Vec<_>>()
        .join(",");
    eprintln!(
        "[gui_effect_pressure_baseline] status=complete preset={}t/{}p/{}live scenarios={}",
        summary.tile_count, summary.passes_per_tile, summary.animated_tile_window, scenarios,
    );
}

pub fn maybe_log_perf(state: &mut EffectStressState, actions: u64) {
    let should_log = actions != 0 || state.log_elapsed >= PERF_LOG_INTERVAL;
    if !should_log {
        return;
    }

    while state.log_elapsed >= PERF_LOG_INTERVAL {
        state.log_elapsed = state.log_elapsed.saturating_sub(PERF_LOG_INTERVAL);
    }

    eprintln!(
        "[gui_effect_pressure] anim={} phase={} tiles={} live_window={} passes={} dt={}ms tree={} paint={} present={} total={} mode={} reason={} dirty={} jobs={} damage={} painted={} scene_passes={} workers={}",
        animation_label(state),
        phase_label(state.phase),
        state.tile_count,
        state.animated_tile_window,
        state.passes_per_tile,
        state.last_frame_ms,
        format_us(state.app_stats.render_tree_us),
        format_us(state.renderer_stats.paint_us),
        format_us(state.renderer_stats.present_us),
        format_us(state.renderer_stats.total_us),
        paint_mode_label(state.renderer_stats),
        paint_reason_label(state.renderer_stats.paint_reason),
        state.renderer_stats.dirty_regions,
        state.renderer_stats.dirty_jobs,
        format_pixels(state.renderer_stats.damage_pixels),
        format_pixels(state.renderer_stats.painted_pixels),
        state.renderer_stats.scene_passes,
        state.renderer_stats.render_workers,
    );
}

fn build_ui(state: &EffectStressState) -> Node {
    ui! {
        <div id="app">
            {with_id(build_ui_hero(state), "hero")}
            <section id="wall-shell" class="wall-shell">
                {build_tile_wall(state)}
            </section>
        </div>
    }
}

fn build_metric_row(state: &EffectStressState) -> Node {
    ui! {
        <div class="metric-row">
            {stat_chip("tiles", state.tile_count.to_string())}
            {stat_chip("passes / tile", state.passes_per_tile.to_string())}
            {stat_chip("live window", state.animated_tile_window.to_string())}
            {stat_chip("effect pods", total_pods(state).to_string())}
            {stat_chip("animated tiles", animated_tile_count(state).to_string())}
            {stat_chip("animated pods", animated_pod_count(state).to_string())}
            {stat_chip("effect nodes", estimated_effect_nodes(state).to_string())}
            {stat_chip("animation", animation_label(state).to_string())}
            {stat_chip("phase", phase_label(state.phase).to_string())}
            {stat_chip("step", format!("{} ms", PHASE_STEP_INTERVAL.as_millis()))}
            {stat_chip("dt", format!("{} ms", state.last_frame_ms))}
            {stat_chip("app view", format_us(state.app_stats.view_us))}
            {stat_chip("tree build", format_us(state.app_stats.render_tree_us))}
            {stat_chip("scene swap", format_us(state.app_stats.scene_swap_us))}
            {stat_chip("transition", format_us(state.app_stats.transition_us))}
            {stat_chip("scene prep", format_us(state.renderer_stats.scene_prep_us))}
            {stat_chip("paint", format_us(state.renderer_stats.paint_us))}
            {stat_chip("present", format_us(state.renderer_stats.present_us))}
            {stat_chip("frame total", format_us(state.renderer_stats.total_us))}
            {stat_chip("paint mode", paint_mode_label(state.renderer_stats))}
            {stat_chip("paint reason", paint_reason_label(state.renderer_stats.paint_reason).to_string())}
            {stat_chip("dirty regions", state.renderer_stats.dirty_regions.to_string())}
            {stat_chip("dirty jobs", state.renderer_stats.dirty_jobs.to_string())}
            {stat_chip("damage", format_pixels(state.renderer_stats.damage_pixels))}
            {stat_chip("painted", format_pixels(state.renderer_stats.painted_pixels))}
            {stat_chip("scene passes", state.renderer_stats.scene_passes.to_string())}
            {stat_chip("workers", state.renderer_stats.render_workers.to_string())}
            {stat_chip("baseline", baseline_status_label(state))}
        </div>
    }
}

fn build_control_row(state: &EffectStressState) -> Node {
    ui! {
        <div class="control-row">
            {control_button("-8 tiles", remove_tiles, false)}
            {control_button("+8 tiles", add_tiles, false)}
            {control_button("+2 passes / tile", add_passes, false)}
            {control_button("-1 live tile", remove_animated_tiles, false)}
            {control_button("+1 live tile", add_animated_tiles, false)}
            {control_button(
                if state.animate {
                    "stop animation"
                } else {
                    "start animation"
                },
                toggle_animation,
                state.animate,
            )}
            {control_button(
                if state.pulse_layout {
                    "stop pulse"
                } else {
                    "start pulse"
                },
                toggle_pulse,
                state.pulse_layout,
            )}
            {control_button(
                if state.baseline_harness.is_some() {
                    "stop baseline"
                } else {
                    "run baseline"
                },
                toggle_baseline,
                state.baseline_harness.is_some(),
            )}
            {control_button("spike", spike, false)}
            {control_button("reset", reset, false)}
        </div>
    }
}

fn build_tile_wall(state: &EffectStressState) -> Node {
    append_children(
        ui! {
            <div class="tile-wall"></div>
        },
        (0..state.tile_count).map(|tile_index| build_tile(tile_index, state)),
    )
}

fn build_tile(tile_index: usize, state: &EffectStressState) -> Node {
    let variant = variant_class(tile_index);
    let animated_tile = tile_is_animated(tile_index, state);
    let band_state = if animated_tile {
        "band-active"
    } else {
        "band-rest"
    };
    let phase = phase_class(static_phase(tile_index, 0));
    let band_indicator = add_class(
        ui! {
            <div class="tile-band-indicator"></div>
        },
        band_state,
    );

    add_classes(
        with_id(
            ui! {
            <article class="effect-tile">
                <div class="tile-header">
                    <div class="tile-title-row">
                        {band_indicator}
                        <p class="tile-label">
                            {format!("bank {:02}", tile_index % 100)}
                        </p>
                    </div>
                    <p class="tile-meta">
                        {format!("{} fx", state.passes_per_tile)}
                    </p>
                </div>
                {build_pod_grid(tile_index, state)}
            </article>
            },
            tile_fragment_id(tile_index),
        ),
        [variant, phase],
    )
}

fn build_pod_grid(tile_index: usize, state: &EffectStressState) -> Node {
    let layout_mode = layout_class(state.pulse_layout);
    let animated_tile = tile_is_animated(tile_index, state);

    append_children(
        ui! {
            <div class="pod-grid"></div>
        },
        (0..state.passes_per_tile).map(|pass_index| {
            let animated =
                state.animate && animated_tile && pass_index < ACTIVE_PODS_PER_ANIMATED_TILE;
            build_effect_pod(tile_index, pass_index, layout_mode, animated, state.phase)
        }),
    )
}

fn build_effect_pod(
    tile_index: usize,
    pass_index: usize,
    layout_mode: &'static str,
    animated: bool,
    phase_seed: usize,
) -> Node {
    let phase = if animated {
        phase_class((phase_seed + tile_index + pass_index) % PHASE_COUNT)
    } else {
        phase_class(static_phase(tile_index, pass_index))
    };
    let motion = if animated { "pod-live" } else { "pod-rest" };
    let variant = variant_class(tile_index * 5 + pass_index);

    add_classes(
        ui! {
            <div class="effect-pod">
                <div class="effect-ring"></div>
                <div class="effect-core"></div>
                <div class="effect-beam"></div>
                {build_spark_row()}
            </div>
        },
        [phase, motion, variant, layout_mode],
    )
}

fn build_spark_row() -> Node {
    append_children(
        ui! {
            <div class="spark-row"></div>
        },
        (0..3).map(build_spark),
    )
}

fn build_spark(index: usize) -> Node {
    add_class(
        ui! {
            <div class="spark"></div>
        },
        spark_variant_class(index),
    )
}

fn stat_chip(label: impl Into<String>, value: impl Into<String>) -> Node {
    let label = label.into();
    let value = value.into();

    ui! {
        <div class="stat-chip">
            <p class="stat-label">
                {label}
            </p>
            <p class="stat-value">
                {value}
            </p>
        </div>
    }
}

fn control_button(label: &'static str, handler: fn(), active: bool) -> Node {
    let button = ui! {
        <button class="control-button" type="button" onclick={handler}>
            {label}
        </button>
    };

    if active {
        add_class(button, "active")
    } else {
        button
    }
}

fn add_class(node: Node, class_name: &'static str) -> Node {
    match node {
        Node::Element(element) => element.with_class(class_name).into(),
        Node::Text(_) => node,
    }
}

fn with_id(node: Node, id: impl Into<String>) -> Node {
    let id = id.into();
    match node {
        Node::Element(element) => element.with_id(id).into(),
        Node::Text(_) => node,
    }
}

fn add_classes<const N: usize>(node: Node, classes: [&'static str; N]) -> Node {
    classes.into_iter().fold(node, add_class)
}

fn append_children(node: Node, children: impl IntoIterator<Item = Node>) -> Node {
    match node {
        Node::Element(element) => element.with_children(children).into(),
        Node::Text(_) => node,
    }
}

fn total_pods(state: &EffectStressState) -> usize {
    state.tile_count.saturating_mul(state.passes_per_tile)
}

fn animated_tile_count(state: &EffectStressState) -> usize {
    if state.animate {
        state.tile_count.min(state.animated_tile_window)
    } else {
        0
    }
}

fn animated_pod_count(state: &EffectStressState) -> usize {
    animated_tile_count(state)
        .saturating_mul(state.passes_per_tile.min(ACTIVE_PODS_PER_ANIMATED_TILE))
}

fn estimated_effect_nodes(state: &EffectStressState) -> usize {
    let chrome_nodes = 26;
    let tile_shell_nodes = state.tile_count.saturating_mul(4);
    let pod_nodes = total_pods(state).saturating_mul(7);
    chrome_nodes + tile_shell_nodes + pod_nodes
}

fn format_us(duration_us: u64) -> String {
    format!("{:.2} ms", duration_us as f64 / 1000.0)
}

fn paint_mode_label(stats: FrameTimingStats) -> String {
    match stats.paint_mode {
        FramePaintMode::Idle => "idle".to_string(),
        FramePaintMode::Full => {
            if stats.render_workers > 1 {
                format!("full x{}", stats.render_workers)
            } else {
                "full".to_string()
            }
        }
        FramePaintMode::Incremental => {
            format!("incremental {}r/{}j", stats.dirty_regions, stats.dirty_jobs)
        }
    }
}

fn paint_reason_label(reason: FramePaintReason) -> &'static str {
    match reason {
        FramePaintReason::Idle => "idle",
        FramePaintReason::FullRedraw => "full redraw",
        FramePaintReason::DirtyRegionLimit => "dirty-region limit",
        FramePaintReason::DirtyAreaLimit => "dirty-area limit",
        FramePaintReason::FragmentedDamage => "fragmented damage",
        FramePaintReason::IncrementalDamage => "small damage",
    }
}

fn format_pixels(pixels: usize) -> String {
    if pixels >= 1_000_000 {
        format!("{:.2}M px", pixels as f64 / 1_000_000.0)
    } else if pixels >= 1_000 {
        format!("{:.1}K px", pixels as f64 / 1_000.0)
    } else {
        format!("{pixels} px")
    }
}

fn baseline_status_label(state: &EffectStressState) -> String {
    if let Some(harness) = &state.baseline_harness {
        if harness.warmup_frames_remaining > 0 {
            let warmed = harness
                .warmup_frames
                .saturating_sub(harness.warmup_frames_remaining);
            format!(
                "{} warm {}/{}",
                harness.scenario.label(),
                warmed,
                harness.warmup_frames
            )
        } else {
            format!(
                "{} sample {}/{}",
                harness.scenario.label(),
                harness.sample_frames_collected,
                harness.sample_frames
            )
        }
    } else if let Some(summary) = &state.last_baseline_summary {
        let avg_paint_us = summary
            .scenarios
            .iter()
            .map(|scenario| scenario.avg_paint_us)
            .sum::<u64>()
            / summary.scenarios.len().max(1) as u64;
        format!("done avg {}", format_us(avg_paint_us))
    } else {
        format!(
            "ready {}t/{}p/{}live",
            BASELINE_TILE_COUNT, BASELINE_PASSES_PER_TILE, BASELINE_ANIMATED_TILE_WINDOW
        )
    }
}

fn animation_label(state: &EffectStressState) -> &'static str {
    if state.animate { "running" } else { "paused" }
}

fn phase_label(phase: usize) -> &'static str {
    match phase % PHASE_COUNT {
        0 => "A",
        1 => "B",
        _ => "C",
    }
}

fn phase_class(phase: usize) -> &'static str {
    match phase % PHASE_COUNT {
        0 => "phase-a",
        1 => "phase-b",
        _ => "phase-c",
    }
}

fn static_phase(tile_index: usize, pass_index: usize) -> usize {
    (tile_index + pass_index) % PHASE_COUNT
}

fn tile_is_animated(index: usize, state: &EffectStressState) -> bool {
    state.animate
        && animation_band_contains(
            index,
            state.tile_count,
            state.animation_band_start,
            state.animated_tile_window,
        )
}

pub fn animated_tile_indices(
    tile_count: usize,
    band_start: usize,
    animated_tile_window: usize,
) -> Vec<usize> {
    if tile_count == 0 || animated_tile_window == 0 {
        return Vec::new();
    }

    (0..tile_count.min(animated_tile_window))
        .map(|offset| (band_start + offset) % tile_count)
        .collect()
}

fn animation_band_contains(
    index: usize,
    tile_count: usize,
    band_start: usize,
    animated_tile_window: usize,
) -> bool {
    if tile_count == 0 || animated_tile_window == 0 {
        return false;
    }

    (0..tile_count.min(animated_tile_window))
        .any(|offset| (band_start + offset) % tile_count == index)
}

fn elapsed_tick_count(elapsed: Duration) -> usize {
    let interval_ms = PHASE_STEP_INTERVAL.as_millis().max(1);
    (elapsed.as_millis() / interval_ms) as usize
}

fn remaining_phase_elapsed(elapsed: Duration) -> Duration {
    let ticks = elapsed_tick_count(elapsed) as u32;
    elapsed.saturating_sub(PHASE_STEP_INTERVAL.saturating_mul(ticks))
}

fn animation_invalidation(state: &EffectStressState) -> Invalidation {
    if state.pulse_layout {
        Invalidation::Layout
    } else {
        Invalidation::Paint
    }
}

fn variant_class(index: usize) -> &'static str {
    match index % 4 {
        0 => "variant-prism",
        1 => "variant-ember",
        2 => "variant-aurora",
        _ => "variant-ion",
    }
}

fn spark_variant_class(index: usize) -> &'static str {
    match index % 3 {
        0 => "spark-a",
        1 => "spark-b",
        _ => "spark-c",
    }
}

fn layout_class(pulse_layout: bool) -> &'static str {
    if pulse_layout {
        "pulse-layout"
    } else {
        "fixed-layout"
    }
}

fn build_ui_hero(state: &EffectStressState) -> Node {
    ui! {
        <section class="hero">
            <div class="hero-copy">
                <p class="eyebrow">
                    {"Example / GUI effect pressure"}
                </p>
                <h1 class="hero-title">
                    {"Effect-heavy animated wall"}
                </h1>
                <p class="hero-note">
                    {"This scene keeps text tiny on purpose and puts the pressure into gradients, glows, box shadows, and a narrow moving band of live effect pods."}
                </p>
            </div>
            {build_metric_row(state)}
            {build_control_row(state)}
        </section>
    }
}

fn fragment_tile_position_class(tile_index: usize) -> &'static str {
    match tile_index {
        0 => "fragment-tile-pos-0",
        1 => "fragment-tile-pos-1",
        2 => "fragment-tile-pos-2",
        3 => "fragment-tile-pos-3",
        4 => "fragment-tile-pos-4",
        5 => "fragment-tile-pos-5",
        6 => "fragment-tile-pos-6",
        7 => "fragment-tile-pos-7",
        8 => "fragment-tile-pos-8",
        9 => "fragment-tile-pos-9",
        10 => "fragment-tile-pos-10",
        11 => "fragment-tile-pos-11",
        12 => "fragment-tile-pos-12",
        13 => "fragment-tile-pos-13",
        14 => "fragment-tile-pos-14",
        15 => "fragment-tile-pos-15",
        16 => "fragment-tile-pos-16",
        17 => "fragment-tile-pos-17",
        18 => "fragment-tile-pos-18",
        19 => "fragment-tile-pos-19",
        20 => "fragment-tile-pos-20",
        21 => "fragment-tile-pos-21",
        22 => "fragment-tile-pos-22",
        23 => "fragment-tile-pos-23",
        24 => "fragment-tile-pos-24",
        25 => "fragment-tile-pos-25",
        26 => "fragment-tile-pos-26",
        27 => "fragment-tile-pos-27",
        28 => "fragment-tile-pos-28",
        29 => "fragment-tile-pos-29",
        30 => "fragment-tile-pos-30",
        31 => "fragment-tile-pos-31",
        32 => "fragment-tile-pos-32",
        33 => "fragment-tile-pos-33",
        34 => "fragment-tile-pos-34",
        35 => "fragment-tile-pos-35",
        36 => "fragment-tile-pos-36",
        37 => "fragment-tile-pos-37",
        38 => "fragment-tile-pos-38",
        39 => "fragment-tile-pos-39",
        40 => "fragment-tile-pos-40",
        41 => "fragment-tile-pos-41",
        42 => "fragment-tile-pos-42",
        43 => "fragment-tile-pos-43",
        44 => "fragment-tile-pos-44",
        45 => "fragment-tile-pos-45",
        46 => "fragment-tile-pos-46",
        _ => "fragment-tile-pos-47",
    }
}

fn add_tiles() {
    ACTIONS.fetch_or(ACTION_ADD_TILES, Ordering::Relaxed);
}

fn remove_tiles() {
    ACTIONS.fetch_or(ACTION_REMOVE_TILES, Ordering::Relaxed);
}

fn add_passes() {
    ACTIONS.fetch_or(ACTION_ADD_PASSES, Ordering::Relaxed);
}

fn add_animated_tiles() {
    ACTIONS.fetch_or(ACTION_ADD_ANIMATED_TILES, Ordering::Relaxed);
}

fn remove_animated_tiles() {
    ACTIONS.fetch_or(ACTION_REMOVE_ANIMATED_TILES, Ordering::Relaxed);
}

fn toggle_animation() {
    ACTIONS.fetch_or(ACTION_TOGGLE_ANIMATION, Ordering::Relaxed);
}

fn toggle_pulse() {
    ACTIONS.fetch_or(ACTION_TOGGLE_PULSE, Ordering::Relaxed);
}

fn toggle_baseline() {
    ACTIONS.fetch_or(ACTION_TOGGLE_BASELINE, Ordering::Relaxed);
}

fn spike() {
    ACTIONS.fetch_or(ACTION_SPIKE, Ordering::Relaxed);
}

fn reset() {
    ACTIONS.fetch_or(ACTION_RESET, Ordering::Relaxed);
}

pub fn stylesheet() -> &'static Stylesheet {
    static STYLESHEET: OnceLock<Stylesheet> = OnceLock::new();

    STYLESHEET.get_or_init(|| {
        parse_stylesheet(include_str!("gui_effect_pressure.css"))
            .expect("gui effect pressure stylesheet should stay valid")
    })
}

pub fn take_actions() -> u64 {
    ACTIONS.swap(0, Ordering::Relaxed)
}

fn active_tile_indices(state: &EffectStressState) -> Vec<usize> {
    if !state.animate {
        return Vec::new();
    }

    animated_tile_indices(
        state.tile_count,
        state.animation_band_start,
        state.animated_tile_window,
    )
}

pub fn tile_fragment_id(tile_index: usize) -> String {
    format!("tile-{tile_index:02}")
}

pub fn normal_app_refresh(
    previous: &EffectStressState,
    next: &EffectStressState,
    invalidation: Invalidation,
) -> Refresh {
    match invalidation {
        Invalidation::Clean => Refresh::clean(),
        Invalidation::Paint => {
            let mut ids = vec!["hero".to_string()];
            for tile_index in active_tile_indices(previous)
                .into_iter()
                .chain(active_tile_indices(next))
            {
                let id = tile_fragment_id(tile_index);
                if !ids.iter().any(|existing| existing == &id) {
                    ids.push(id);
                }
            }
            Refresh::fragments(ids, Invalidation::Paint)
        }
        Invalidation::Layout | Invalidation::Structure => Refresh::full(invalidation),
    }
}

pub fn fragment_refresh(
    previous: &EffectStressState,
    next: &EffectStressState,
    invalidation: Invalidation,
) -> Refresh {
    match invalidation {
        Invalidation::Clean => Refresh::clean(),
        Invalidation::Paint => {
            let mut ids = vec!["hero".to_string()];
            for tile_index in active_tile_indices(previous)
                .into_iter()
                .chain(active_tile_indices(next))
            {
                let id = tile_fragment_id(tile_index);
                if !ids.iter().any(|existing| existing == &id) {
                    ids.push(id);
                }
            }
            Refresh::fragments(ids, Invalidation::Paint)
        }
        Invalidation::Layout | Invalidation::Structure => {
            let mut ids = vec!["hero".to_string(), "wall-shell".to_string()];
            ids.extend((0..MAX_TILE_COUNT).map(tile_fragment_id));
            Refresh::fragments(ids, invalidation)
        }
    }
}

pub fn build_fragment_backdrop() -> Node {
    with_id(
        ui! {
            <div class="fragment-backdrop"></div>
        },
        "backdrop",
    )
}

pub fn build_fragment_hero(state: &EffectStressState) -> Node {
    with_id(add_class(build_ui_hero(state), "fragment-hero"), "hero")
}

pub fn build_fragment_wall_shell() -> Node {
    with_id(
        add_class(
            ui! {
                <section class="wall-shell"></section>
            },
            "fragment-wall-shell",
        ),
        "wall-shell",
    )
}

pub fn build_fragment_tile(tile_index: usize, state: &EffectStressState) -> Node {
    let mut node = add_classes(
        with_id(build_tile(tile_index, state), tile_fragment_id(tile_index)),
        ["fragment-tile", fragment_tile_position_class(tile_index)],
    );
    if tile_index >= state.tile_count {
        node = add_class(node, "fragment-hidden");
    }
    node
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{
        ACTION_ADD_ANIMATED_TILES, ACTION_ADD_PASSES, ACTION_ADD_TILES,
        ACTION_REMOVE_ANIMATED_TILES, ACTION_REMOVE_TILES, ACTION_RESET, ACTION_SPIKE,
        ACTION_TOGGLE_ANIMATION, ACTION_TOGGLE_BASELINE, ACTION_TOGGLE_PULSE,
        BASELINE_ANIMATED_TILE_WINDOW, BASELINE_PASSES_PER_TILE, BASELINE_TILE_COUNT,
        BaselineHarness, BaselineScenario, DEFAULT_ANIMATED_TILE_WINDOW, DEFAULT_PASSES_PER_TILE,
        DEFAULT_TILE_COUNT, EffectStressState, animated_pod_count, estimated_effect_nodes,
        normal_app_refresh, phase_label,
    };
    use cssimpler::app::{Invalidation, Refresh, RuntimeStats};
    use cssimpler::renderer::{FrameInfo, FramePaintMode, FrameTimingStats};

    #[test]
    fn actions_expand_the_effect_wall() {
        let mut state = EffectStressState::default();

        let refresh = super::apply_frame(
            &mut state,
            frame(1),
            ACTION_ADD_TILES | ACTION_ADD_PASSES | ACTION_SPIKE,
        );

        assert_eq!(refresh, Invalidation::Layout);
        assert!(state.tile_count > DEFAULT_TILE_COUNT);
        assert!(state.passes_per_tile > DEFAULT_PASSES_PER_TILE);
        assert!(state.animate);
        assert!(state.pulse_layout);
    }

    #[test]
    fn tile_controls_can_reduce_the_rendered_wall() {
        let mut state = EffectStressState {
            tile_count: 20,
            ..EffectStressState::default()
        };

        let refresh = super::apply_frame(&mut state, frame(1), ACTION_REMOVE_TILES);

        assert_eq!(refresh, Invalidation::Layout);
        assert_eq!(state.tile_count, 12);
    }

    #[test]
    fn tile_controls_saturate_at_zero() {
        let mut state = EffectStressState {
            tile_count: 4,
            ..EffectStressState::default()
        };

        let refresh = super::apply_frame(&mut state, frame(1), ACTION_REMOVE_TILES);

        assert_eq!(refresh, Invalidation::Layout);
        assert_eq!(state.tile_count, 0);
    }

    #[test]
    fn live_window_controls_adjust_the_number_of_animated_tiles() {
        let mut state = EffectStressState::default();

        let refresh = super::apply_frame(&mut state, frame(1), ACTION_ADD_ANIMATED_TILES);

        assert_eq!(refresh, Invalidation::Paint);
        assert_eq!(state.animated_tile_window, DEFAULT_ANIMATED_TILE_WINDOW + 1);
        assert_eq!(
            super::animated_tile_count(&state),
            DEFAULT_ANIMATED_TILE_WINDOW + 1
        );
    }

    #[test]
    fn live_window_controls_saturate_at_zero() {
        let mut state = EffectStressState {
            animated_tile_window: 1,
            ..EffectStressState::default()
        };

        let refresh = super::apply_frame(&mut state, frame(1), ACTION_REMOVE_ANIMATED_TILES);

        assert_eq!(refresh, Invalidation::Paint);
        assert_eq!(state.animated_tile_window, 0);
        assert_eq!(super::animated_tile_count(&state), 0);
    }

    #[test]
    fn animation_tick_uses_elapsed_time() {
        let mut state = EffectStressState {
            tile_count: 8,
            passes_per_tile: 6,
            animate: true,
            pulse_layout: false,
            ..EffectStressState::default()
        };

        let refresh = super::apply_frame(&mut state, frame_with_delta(1, 120), 0);

        assert_eq!(refresh, Invalidation::Paint);
        assert_eq!(phase_label(state.phase), "B");
        assert_eq!(state.animation_band_start, 1);
    }

    #[test]
    fn animation_waits_for_enough_elapsed_time() {
        let mut state = EffectStressState {
            animate: true,
            pulse_layout: false,
            ..EffectStressState::default()
        };

        let refresh = super::apply_frame(&mut state, frame_with_delta(10, 16), 0);

        assert_eq!(refresh, Invalidation::Clean);
        assert_eq!(phase_label(state.phase), "A");
    }

    #[test]
    fn toggles_and_reset_restore_defaults() {
        let mut state = EffectStressState::default();

        let toggle = super::apply_frame(
            &mut state,
            frame(2),
            ACTION_TOGGLE_ANIMATION | ACTION_TOGGLE_PULSE,
        );
        assert_eq!(toggle, Invalidation::Layout);
        assert!(!state.animate);
        assert!(state.pulse_layout);

        let refresh = super::apply_frame(&mut state, frame(3), ACTION_RESET);

        assert_eq!(refresh, Invalidation::Layout);
        assert_eq!(state.tile_count, DEFAULT_TILE_COUNT);
        assert_eq!(state.passes_per_tile, DEFAULT_PASSES_PER_TILE);
        assert!(state.animate);
        assert!(!state.pulse_layout);
    }

    #[test]
    fn estimates_scale_with_load() {
        let small = EffectStressState {
            tile_count: 12,
            passes_per_tile: 4,
            animate: false,
            ..EffectStressState::default()
        };
        let large = EffectStressState {
            tile_count: 36,
            passes_per_tile: 10,
            animate: true,
            ..EffectStressState::default()
        };

        assert!(estimated_effect_nodes(&large) > estimated_effect_nodes(&small));
        assert!(animated_pod_count(&large) >= animated_pod_count(&small));
    }

    #[test]
    fn animated_band_wraps_through_the_visible_tiles() {
        assert_eq!(super::animated_tile_indices(5, 4, 2), vec![4, 0]);
    }

    #[test]
    fn animated_band_uses_the_requested_window_size() {
        assert_eq!(super::animated_tile_indices(5, 4, 3), vec![4, 0, 1]);
    }

    #[test]
    fn normal_app_refresh_targets_only_the_live_band_for_paint() {
        let previous = EffectStressState {
            tile_count: 8,
            animation_band_start: 0,
            ..EffectStressState::default()
        };
        let next = EffectStressState {
            tile_count: 8,
            animation_band_start: 1,
            ..previous.clone()
        };

        let refresh = normal_app_refresh(&previous, &next, Invalidation::Paint);

        assert_eq!(
            refresh,
            Refresh::fragments(
                ["hero", "tile-00", "tile-01", "tile-02"],
                Invalidation::Paint
            )
        );
    }

    #[test]
    fn normal_app_refresh_includes_new_tiles_when_the_live_window_grows() {
        let previous = EffectStressState {
            tile_count: 8,
            animated_tile_window: 2,
            animation_band_start: 0,
            ..EffectStressState::default()
        };
        let next = EffectStressState {
            animated_tile_window: 4,
            ..previous.clone()
        };

        let refresh = normal_app_refresh(&previous, &next, Invalidation::Paint);

        assert_eq!(
            refresh,
            Refresh::fragments(
                ["hero", "tile-00", "tile-01", "tile-02", "tile-03"],
                Invalidation::Paint
            )
        );
    }

    #[test]
    fn baseline_action_starts_the_fixed_pressure_preset() {
        let mut state = EffectStressState::default();

        let refresh = super::apply_frame(&mut state, frame(1), ACTION_TOGGLE_BASELINE);

        assert_eq!(refresh, Invalidation::Layout);
        assert_eq!(state.tile_count, BASELINE_TILE_COUNT);
        assert_eq!(state.passes_per_tile, BASELINE_PASSES_PER_TILE);
        assert_eq!(state.animated_tile_window, BASELINE_ANIMATED_TILE_WINDOW);
        assert!(!state.animate);
        assert!(!state.pulse_layout);
        assert!(state.baseline_harness.is_some());
        assert_eq!(
            state
                .baseline_harness
                .as_ref()
                .map(|harness| harness.scenario),
            Some(BaselineScenario::Idle)
        );
    }

    #[test]
    fn baseline_harness_cycles_idle_paint_and_pulse_modes() {
        let mut state = EffectStressState::default();
        state.baseline_harness = Some(BaselineHarness::new_with_limits(1, 2));

        let initial = super::apply_baseline_scenario(&mut state, BaselineScenario::Idle);
        assert_eq!(initial, Invalidation::Layout);

        for _ in 0..12 {
            let Some(scenario) = state
                .baseline_harness
                .as_ref()
                .map(|harness| harness.scenario)
            else {
                break;
            };
            apply_baseline_sample(&mut state, scenario);
            let _ = super::advance_baseline_harness(&mut state);
        }

        assert!(state.baseline_harness.is_none());
        let summary = state
            .last_baseline_summary
            .as_ref()
            .expect("baseline run should complete and keep the last summary");
        assert_eq!(summary.tile_count, BASELINE_TILE_COUNT);
        assert_eq!(summary.passes_per_tile, BASELINE_PASSES_PER_TILE);
        assert_eq!(summary.animated_tile_window, BASELINE_ANIMATED_TILE_WINDOW);
        assert_eq!(summary.scenarios.len(), BaselineScenario::ALL.len());
        assert_eq!(summary.scenarios[0].scenario, BaselineScenario::Idle);
        assert_eq!(summary.scenarios[0].avg_paint_us, 0);
        assert_eq!(summary.scenarios[0].idle_frames, 2);
        assert_eq!(
            summary.scenarios[1].scenario,
            BaselineScenario::AnimatedPaint
        );
        assert_eq!(summary.scenarios[1].avg_paint_us, 320);
        assert_eq!(summary.scenarios[1].incremental_frames, 2);
        assert_eq!(summary.scenarios[2].scenario, BaselineScenario::PulseLayout);
        assert_eq!(summary.scenarios[2].avg_render_tree_us, 140);
        assert_eq!(summary.scenarios[2].full_frames, 2);
    }

    fn frame(frame_index: u64) -> FrameInfo {
        frame_with_delta(frame_index, 16)
    }

    fn frame_with_delta(frame_index: u64, delta_ms: u64) -> FrameInfo {
        FrameInfo {
            frame_index,
            delta: Duration::from_millis(delta_ms),
        }
    }

    fn apply_baseline_sample(state: &mut EffectStressState, scenario: BaselineScenario) {
        state.app_stats = match scenario {
            BaselineScenario::Idle => RuntimeStats {
                render_tree_us: 100,
                ..RuntimeStats::default()
            },
            BaselineScenario::AnimatedPaint => RuntimeStats {
                render_tree_us: 120,
                ..RuntimeStats::default()
            },
            BaselineScenario::PulseLayout => RuntimeStats {
                render_tree_us: 140,
                ..RuntimeStats::default()
            },
        };
        state.renderer_stats = match scenario {
            BaselineScenario::Idle => FrameTimingStats {
                scene_prep_us: 200,
                paint_us: 0,
                present_us: 300,
                total_us: 500,
                paint_mode: FramePaintMode::Idle,
                ..FrameTimingStats::default()
            },
            BaselineScenario::AnimatedPaint => FrameTimingStats {
                scene_prep_us: 220,
                paint_us: 320,
                present_us: 420,
                total_us: 620,
                paint_mode: FramePaintMode::Incremental,
                ..FrameTimingStats::default()
            },
            BaselineScenario::PulseLayout => FrameTimingStats {
                scene_prep_us: 240,
                paint_us: 340,
                present_us: 440,
                total_us: 740,
                paint_mode: FramePaintMode::Full,
                ..FrameTimingStats::default()
            },
        };
    }
}
