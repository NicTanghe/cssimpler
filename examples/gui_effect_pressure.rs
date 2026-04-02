use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Result;
use cssimpler::app::{App, Invalidation};
use cssimpler::core::Node;
use cssimpler::renderer::{FrameInfo, WindowConfig};
use cssimpler::style::{Stylesheet, parse_stylesheet};

const ACTION_ADD_TILES: u64 = 1 << 0;
const ACTION_ADD_PASSES: u64 = 1 << 1;
const ACTION_TOGGLE_ANIMATION: u64 = 1 << 2;
const ACTION_TOGGLE_PULSE: u64 = 1 << 3;
const ACTION_SPIKE: u64 = 1 << 4;
const ACTION_RESET: u64 = 1 << 5;

const PHASE_COUNT: usize = 3;
const PHASE_STEP_FRAMES: u64 = 4;
const DEFAULT_TILE_COUNT: usize = 18;
const DEFAULT_PASSES_PER_TILE: usize = 6;
const TILE_STEP: usize = 12;
const PASS_STEP: usize = 2;
const SPIKE_TILE_STEP: usize = 24;
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
}

impl Default for EffectStressState {
    fn default() -> Self {
        Self {
            frame_index: 0,
            last_frame_ms: 0,
            tile_count: DEFAULT_TILE_COUNT,
            passes_per_tile: DEFAULT_PASSES_PER_TILE,
            animate: true,
            pulse_layout: true,
            phase: 0,
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
    apply_frame(state, frame, actions)
}

fn apply_frame(state: &mut EffectStressState, frame: FrameInfo, actions: u64) -> Invalidation {
    state.frame_index = frame.frame_index;
    state.last_frame_ms = frame.delta.as_millis();

    if actions & ACTION_RESET != 0 {
        *state = EffectStressState {
            frame_index: frame.frame_index,
            last_frame_ms: frame.delta.as_millis(),
            ..EffectStressState::default()
        };
        return Invalidation::Layout;
    }

    let mut invalidation = Invalidation::Clean;

    if actions & ACTION_ADD_TILES != 0 {
        state.tile_count = state.tile_count.saturating_add(TILE_STEP);
        invalidation = invalidation.max(Invalidation::Layout);
    }

    if actions & ACTION_ADD_PASSES != 0 {
        state.passes_per_tile = state.passes_per_tile.saturating_add(PASS_STEP);
        invalidation = invalidation.max(Invalidation::Layout);
    }

    if actions & ACTION_TOGGLE_ANIMATION != 0 {
        state.animate = !state.animate;
        invalidation = invalidation.max(Invalidation::Layout);
    }

    if actions & ACTION_TOGGLE_PULSE != 0 {
        state.pulse_layout = !state.pulse_layout;
        invalidation = invalidation.max(Invalidation::Layout);
    }

    if actions & ACTION_SPIKE != 0 {
        state.tile_count = state.tile_count.saturating_add(SPIKE_TILE_STEP);
        state.passes_per_tile = state.passes_per_tile.saturating_add(SPIKE_PASS_STEP);
        state.animate = true;
        state.pulse_layout = true;
        invalidation = invalidation.max(Invalidation::Layout);
    }

    if state.animate && frame.frame_index % PHASE_STEP_FRAMES == 0 {
        state.phase = (state.phase + 1) % PHASE_COUNT;
        let tick_invalidation = if state.pulse_layout {
            Invalidation::Layout
        } else {
            Invalidation::Paint
        };
        invalidation = invalidation.max(tick_invalidation);
    }

    invalidation
}

fn build_ui(state: &EffectStressState) -> Node {
    Node::element("div")
        .with_id("app")
        .with_child(build_hero(state))
        .with_child(build_wall_shell(state))
        .into()
}

fn build_hero(state: &EffectStressState) -> Node {
    Node::element("section")
        .with_class("hero")
        .with_child(
            Node::element("div")
                .with_class("hero-copy")
                .with_child(text_block("p", "eyebrow", "Example / GUI effect pressure"))
                .with_child(text_block("h1", "hero-title", "Effect-heavy animated wall"))
                .with_child(text_block(
                    "p",
                    "hero-note",
                    "This scene keeps text tiny on purpose and instead multiplies gradients, glows, drop shadows, and animated size changes across a dense wall of tiles.",
                ))
                .into(),
        )
        .with_child(
            Node::element("div")
                .with_class("metric-row")
                .with_children([
                    stat_chip("tiles", state.tile_count.to_string()),
                    stat_chip("passes / tile", state.passes_per_tile.to_string()),
                    stat_chip("effect pods", total_pods(state).to_string()),
                    stat_chip("effect nodes", estimated_effect_nodes(state).to_string()),
                    stat_chip("scene copies", estimated_scene_copies(state).to_string()),
                    stat_chip("phase", phase_label(state.phase).to_string()),
                    stat_chip("dt", format!("{} ms", state.last_frame_ms)),
                ])
                .into(),
        )
        .with_child(
            Node::element("div")
                .with_class("control-row")
                .with_children([
                    control_button("+12 tiles", add_tiles, false),
                    control_button("+2 passes / tile", add_passes, false),
                    control_button(
                        if state.animate {
                            "stop animation"
                        } else {
                            "start animation"
                        },
                        toggle_animation,
                        state.animate,
                    ),
                    control_button(
                        if state.pulse_layout {
                            "stop pulse"
                        } else {
                            "start pulse"
                        },
                        toggle_pulse,
                        state.pulse_layout,
                    ),
                    control_button("spike", spike, false),
                    control_button("reset", reset, false),
                ])
                .into(),
        )
        .into()
}

fn build_wall_shell(state: &EffectStressState) -> Node {
    Node::element("section")
        .with_class("wall-shell")
        .with_child(
            Node::element("div")
                .with_class("tile-wall")
                .with_children((0..state.tile_count).map(|index| build_effect_tile(index, state)))
                .into(),
        )
        .into()
}

fn build_effect_tile(index: usize, state: &EffectStressState) -> Node {
    let phase = phase_class((state.phase + index) % PHASE_COUNT);
    let variant = variant_class(index);
    let layout_mode = layout_class(state.pulse_layout);

    Node::element("article")
        .with_class("effect-tile")
        .with_class(phase)
        .with_class(variant)
        .with_child(
            Node::element("div")
                .with_class("tile-header")
                .with_child(text_block(
                    "p",
                    "tile-label",
                    format!("bank {:02}", index % 100),
                ))
                .with_child(text_block(
                    "p",
                    "tile-meta",
                    format!("{} fx", state.passes_per_tile),
                ))
                .into(),
        )
        .with_child(
            Node::element("div")
                .with_class("pod-grid")
                .with_children((0..state.passes_per_tile).map(|pass_index| {
                    build_effect_pod(index, pass_index, state.phase, layout_mode)
                }))
                .into(),
        )
        .into()
}

fn build_effect_pod(
    tile_index: usize,
    pass_index: usize,
    phase_seed: usize,
    layout_mode: &'static str,
) -> Node {
    let phase = phase_class((phase_seed + tile_index + pass_index) % PHASE_COUNT);
    let variant = variant_class(tile_index * 5 + pass_index);

    Node::element("div")
        .with_class("effect-pod")
        .with_class(phase)
        .with_class(variant)
        .with_class(layout_mode)
        .with_child(Node::element("div").with_class("effect-ring").into())
        .with_child(Node::element("div").with_class("effect-core").into())
        .with_child(
            Node::element("div")
                .with_class("spark-row")
                .with_children((0..3).map(build_spark))
                .into(),
        )
        .into()
}

fn build_spark(index: usize) -> Node {
    Node::element("div")
        .with_class("spark")
        .with_class(spark_variant_class(index))
        .into()
}

fn text_block(tag: &str, class_name: &str, content: impl Into<String>) -> Node {
    Node::element(tag)
        .with_class(class_name)
        .with_child(Node::text(content))
        .into()
}

fn stat_chip(label: impl Into<String>, value: impl Into<String>) -> Node {
    Node::element("div")
        .with_class("stat-chip")
        .with_child(text_block("p", "stat-label", label))
        .with_child(text_block("p", "stat-value", value))
        .into()
}

fn control_button(label: &'static str, handler: fn(), active: bool) -> Node {
    let mut button = Node::element("button")
        .with_class("control-button")
        .with_attribute("type", "button")
        .on_click(handler);

    if active {
        button = button.with_class("active");
    }

    button.with_child(Node::text(label)).into()
}

fn total_pods(state: &EffectStressState) -> usize {
    state.tile_count.saturating_mul(state.passes_per_tile)
}

fn estimated_effect_nodes(state: &EffectStressState) -> usize {
    let tile_shell_nodes = 6;
    let pod_nodes = 7;
    6 + state
        .tile_count
        .saturating_mul(tile_shell_nodes + state.passes_per_tile * pod_nodes)
}

fn estimated_scene_copies(state: &EffectStressState) -> usize {
    if state.animate { 3 } else { 1 }
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
        estimated_effect_nodes, estimated_scene_copies, phase_label,
    };
    use cssimpler::app::Invalidation;
    use cssimpler::renderer::FrameInfo;

    #[test]
    fn actions_expand_the_effect_wall() {
        let mut state = EffectStressState::default();

        let invalidation = super::apply_frame(
            &mut state,
            frame(1),
            ACTION_ADD_TILES | ACTION_ADD_PASSES | ACTION_SPIKE,
        );

        assert_eq!(invalidation, Invalidation::Layout);
        assert!(state.tile_count > DEFAULT_TILE_COUNT);
        assert!(state.passes_per_tile > DEFAULT_PASSES_PER_TILE);
        assert!(state.animate);
        assert!(state.pulse_layout);
    }

    #[test]
    fn animation_tick_is_paint_only_without_layout_pulses() {
        let mut state = EffectStressState {
            animate: true,
            pulse_layout: false,
            ..EffectStressState::default()
        };

        let invalidation = super::apply_frame(&mut state, frame(4), 0);

        assert_eq!(invalidation, Invalidation::Paint);
        assert_eq!(phase_label(state.phase), "B");
    }

    #[test]
    fn toggles_and_reset_restore_defaults() {
        let mut state = EffectStressState::default();

        super::apply_frame(
            &mut state,
            frame(2),
            ACTION_TOGGLE_ANIMATION | ACTION_TOGGLE_PULSE,
        );
        assert!(!state.animate);
        assert!(!state.pulse_layout);

        let invalidation = super::apply_frame(&mut state, frame(3), ACTION_RESET);

        assert_eq!(invalidation, Invalidation::Layout);
        assert_eq!(state.tile_count, DEFAULT_TILE_COUNT);
        assert_eq!(state.passes_per_tile, DEFAULT_PASSES_PER_TILE);
        assert!(state.animate);
        assert!(state.pulse_layout);
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
        assert_eq!(estimated_scene_copies(&small), 1);
        assert_eq!(estimated_scene_copies(&large), 3);
    }

    fn frame(frame_index: u64) -> FrameInfo {
        FrameInfo {
            frame_index,
            delta: Duration::from_millis(16),
        }
    }
}
