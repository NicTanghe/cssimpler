use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::Result;
use cssimpler::app::{App, Invalidation, RuntimeStats, latest_runtime_stats};
use cssimpler::core::Node;
use cssimpler::renderer::{
    FrameInfo, FramePaintMode, FrameTimingStats, WindowConfig, latest_frame_timing_stats,
};
use cssimpler::style::{Stylesheet, parse_stylesheet};
use cssimpler::ui;

const ACTION_ADD_TILES: u64 = 1 << 0;
const ACTION_ADD_PASSES: u64 = 1 << 1;
const ACTION_TOGGLE_ANIMATION: u64 = 1 << 2;
const ACTION_TOGGLE_PULSE: u64 = 1 << 3;
const ACTION_SPIKE: u64 = 1 << 4;
const ACTION_RESET: u64 = 1 << 5;

const PHASE_COUNT: usize = 3;
const PHASE_STEP_INTERVAL: Duration = Duration::from_millis(120);
const PERF_LOG_INTERVAL: Duration = Duration::from_secs(1);
const MAX_TILE_COUNT: usize = 48;
const ANIMATED_TILE_WINDOW: usize = 2;
const ACTIVE_PODS_PER_ANIMATED_TILE: usize = 2;
const DEFAULT_TILE_COUNT: usize = 12;
const DEFAULT_PASSES_PER_TILE: usize = 4;
const TILE_STEP: usize = 8;
const PASS_STEP: usize = 2;
const SPIKE_TILE_STEP: usize = 16;
const SPIKE_PASS_STEP: usize = 4;

static ACTIONS: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, PartialEq, Eq)]
struct EffectStressState {
    frame_index: u64,
    last_frame_ms: u128,
    tile_count: usize,
    passes_per_tile: usize,
    animate: bool,
    pulse_layout: bool,
    phase: usize,
    phase_elapsed: Duration,
    animation_band_start: usize,
    log_elapsed: Duration,
    renderer_stats: FrameTimingStats,
    app_stats: RuntimeStats,
}

impl Default for EffectStressState {
    fn default() -> Self {
        Self {
            frame_index: 0,
            last_frame_ms: 0,
            tile_count: DEFAULT_TILE_COUNT,
            passes_per_tile: DEFAULT_PASSES_PER_TILE,
            animate: true,
            pulse_layout: false,
            phase: 0,
            phase_elapsed: Duration::ZERO,
            animation_band_start: 0,
            log_elapsed: Duration::ZERO,
            renderer_stats: FrameTimingStats::default(),
            app_stats: RuntimeStats::default(),
        }
    }
}

fn main() -> Result<()> {
    let config = WindowConfig::new("cssimpler / gui effect pressure", 1440, 960);

    App::new(EffectStressState::default(), stylesheet(), update, build_ui)
        .run(config)
        .map_err(Into::into)
}

fn update(state: &mut EffectStressState, frame: FrameInfo) -> Invalidation {
    let actions = ACTIONS.swap(0, Ordering::Relaxed);
    let invalidation = apply_frame(state, frame, actions);
    maybe_log_perf(state, actions);
    invalidation
}

fn apply_frame(state: &mut EffectStressState, frame: FrameInfo, actions: u64) -> Invalidation {
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

    if actions & ACTION_ADD_TILES != 0 {
        state.tile_count = (state.tile_count.saturating_add(TILE_STEP)).min(MAX_TILE_COUNT);
        invalidation = Invalidation::Layout;
    }

    if actions & ACTION_ADD_PASSES != 0 {
        state.passes_per_tile = state.passes_per_tile.saturating_add(PASS_STEP);
        invalidation = Invalidation::Layout;
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

fn maybe_log_perf(state: &mut EffectStressState, actions: u64) {
    let should_log = actions != 0 || state.log_elapsed >= PERF_LOG_INTERVAL;
    if !should_log {
        return;
    }

    while state.log_elapsed >= PERF_LOG_INTERVAL {
        state.log_elapsed = state.log_elapsed.saturating_sub(PERF_LOG_INTERVAL);
    }

    eprintln!(
        "[gui_effect_pressure] anim={} phase={} tiles={} passes={} dt={}ms tree={} paint={} present={} total={} mode={} dirty={} workers={}",
        animation_label(state),
        phase_label(state.phase),
        state.tile_count,
        state.passes_per_tile,
        state.last_frame_ms,
        format_us(state.app_stats.render_tree_us),
        format_us(state.renderer_stats.paint_us),
        format_us(state.renderer_stats.present_us),
        format_us(state.renderer_stats.total_us),
        paint_mode_label(state.renderer_stats),
        state.renderer_stats.dirty_regions,
        state.renderer_stats.render_workers,
    );
}

fn build_ui(state: &EffectStressState) -> Node {
    ui! {
        <div id="app">
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
            <section class="wall-shell">
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
            {stat_chip("dirty regions", state.renderer_stats.dirty_regions.to_string())}
            {stat_chip("workers", state.renderer_stats.render_workers.to_string())}
        </div>
    }
}

fn build_control_row(state: &EffectStressState) -> Node {
    ui! {
        <div class="control-row">
            {control_button("+8 tiles", add_tiles, false)}
            {control_button("+2 passes / tile", add_passes, false)}
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
    let phase = if animated_tile {
        phase_class((state.phase + tile_index) % PHASE_COUNT)
    } else {
        phase_class(static_phase(tile_index, 0))
    };

    add_classes(
        ui! {
            <article class="effect-tile">
                <div class="tile-header">
                    <p class="tile-label">
                        {format!("bank {:02}", tile_index % 100)}
                    </p>
                    <p class="tile-meta">
                        {format!("{} fx", state.passes_per_tile)}
                    </p>
                </div>
                {build_pod_grid(tile_index, state)}
            </article>
        },
        [variant, band_state, phase],
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
        state.tile_count.min(ANIMATED_TILE_WINDOW)
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
        FramePaintMode::Incremental => format!("incremental {}", stats.dirty_regions),
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
        && animated_tile_indices(state.tile_count, state.animation_band_start).contains(&index)
}

fn animated_tile_indices(tile_count: usize, band_start: usize) -> Vec<usize> {
    if tile_count == 0 {
        return Vec::new();
    }

    (0..tile_count.min(ANIMATED_TILE_WINDOW))
        .map(|offset| (band_start + offset) % tile_count)
        .collect()
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

fn add_tiles() {
    ACTIONS.fetch_or(ACTION_ADD_TILES, Ordering::Relaxed);
}

fn add_passes() {
    ACTIONS.fetch_or(ACTION_ADD_PASSES, Ordering::Relaxed);
}

fn toggle_animation() {
    ACTIONS.fetch_or(ACTION_TOGGLE_ANIMATION, Ordering::Relaxed);
}

fn toggle_pulse() {
    ACTIONS.fetch_or(ACTION_TOGGLE_PULSE, Ordering::Relaxed);
}

fn spike() {
    ACTIONS.fetch_or(ACTION_SPIKE, Ordering::Relaxed);
}

fn reset() {
    ACTIONS.fetch_or(ACTION_RESET, Ordering::Relaxed);
}

fn stylesheet() -> &'static Stylesheet {
    static STYLESHEET: OnceLock<Stylesheet> = OnceLock::new();

    STYLESHEET.get_or_init(|| {
        parse_stylesheet(include_str!("gui_effect_pressure.css"))
            .expect("gui effect pressure stylesheet should stay valid")
    })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{
        ACTION_ADD_PASSES, ACTION_ADD_TILES, ACTION_RESET, ACTION_SPIKE, ACTION_TOGGLE_ANIMATION,
        ACTION_TOGGLE_PULSE, DEFAULT_PASSES_PER_TILE, DEFAULT_TILE_COUNT, EffectStressState,
        animated_pod_count, estimated_effect_nodes, phase_label,
    };
    use cssimpler::app::Invalidation;
    use cssimpler::renderer::FrameInfo;

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
        assert_eq!(super::animated_tile_indices(5, 4), vec![4, 0]);
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
}
