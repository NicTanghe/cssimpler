//! Visible stress test for transitions during unrelated scene changes.
//! Run with `cargo run --release --example transition_stress_gui -- 1000`.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::Result;
use cssimpler::app::{App, Invalidation};
use cssimpler::core::Node;
use cssimpler::renderer::{FrameInfo, WindowConfig, latest_frame_timing_stats};
use cssimpler::style::parse_stylesheet;
use cssimpler::ui;

const TOGGLE: u64 = 1;
const TOGGLE_AUTO: u64 = 2;
const TOGGLE_CHANGES: u64 = 4;
const ADD_TILES: u64 = 8;
const REMOVE_TILES: u64 = 16;
const TILE_STEP: usize = 250;
const MAX_TILES: usize = 5000;
const ANIMATION_INTERVAL: Duration = Duration::from_millis(1400);
const CHANGE_INTERVAL: Duration = Duration::from_millis(300);
const REPORT_INTERVAL: Duration = Duration::from_secs(2);

struct State {
    actions: Arc<AtomicU64>,
    tiles: usize,
    alternate: bool,
    auto: bool,
    changes: bool,
    extra_node: bool,
    animation_elapsed: Duration,
    change_elapsed: Duration,
    report_elapsed: Duration,
    report_frames: u64,
    report_slow: u64,
    report_max_delta: Duration,
    report_max_work_us: u64,
    report_max_paint_us: u64,
}

impl State {
    fn new(tiles: usize) -> Self {
        Self {
            actions: Arc::default(),
            tiles,
            alternate: false,
            auto: true,
            changes: false,
            extra_node: false,
            animation_elapsed: Duration::ZERO,
            change_elapsed: Duration::ZERO,
            report_elapsed: Duration::ZERO,
            report_frames: 0,
            report_slow: 0,
            report_max_delta: Duration::ZERO,
            report_max_work_us: 0,
            report_max_paint_us: 0,
        }
    }
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let tiles: usize = args.next().map(|n| n.parse()).transpose()?.unwrap_or(1000);
    anyhow::ensure!(
        (1..=MAX_TILES).contains(&tiles),
        "tile count must be between 1 and {MAX_TILES}"
    );
    anyhow::ensure!(
        args.next().is_none(),
        "usage: transition_stress_gui [tile count]"
    );
    let stylesheet = parse_stylesheet(include_str!("transition_stress_gui.css"))?;
    println!("Frame timing is printed every two seconds. The window shows the actual animation.");
    App::new(State::new(tiles), &stylesheet, update, build_ui)
        .with_continuous_updates(true)
        .run(WindowConfig::new(
            "cssimpler / transition stress",
            1100,
            760,
        ))?;
    Ok(())
}

fn update(state: &mut State, frame: FrameInfo) -> Invalidation {
    report_frame_timing(state, frame);
    let actions = state.actions.swap(0, Ordering::Relaxed);
    let mut invalidation = Invalidation::Clean;

    if actions & TOGGLE_AUTO != 0 {
        state.auto = !state.auto;
        state.animation_elapsed = Duration::ZERO;
        invalidation = Invalidation::Layout;
    }
    if actions & TOGGLE_CHANGES != 0 {
        state.changes = !state.changes;
        state.change_elapsed = Duration::ZERO;
        if !state.changes && state.extra_node {
            state.extra_node = false;
            invalidation = Invalidation::Structure;
        } else {
            invalidation = Invalidation::Layout;
        }
    }
    if actions & ADD_TILES != 0 {
        state.tiles = state.tiles.saturating_add(TILE_STEP).min(MAX_TILES);
        invalidation = Invalidation::Structure;
    }
    if actions & REMOVE_TILES != 0 {
        state.tiles = state.tiles.saturating_sub(TILE_STEP).max(1);
        invalidation = Invalidation::Structure;
    }

    let mut toggle = actions & TOGGLE != 0;
    if state.auto {
        state.animation_elapsed += frame.delta;
        if state.animation_elapsed >= ANIMATION_INTERVAL {
            state.animation_elapsed = Duration::ZERO;
            toggle = true;
        }
    }
    if toggle {
        state.alternate = !state.alternate;
        state.animation_elapsed = Duration::ZERO;
        invalidation = invalidation.max(Invalidation::Layout);
    }
    if state.changes {
        state.change_elapsed += frame.delta;
        if state.change_elapsed >= CHANGE_INTERVAL {
            state.change_elapsed = Duration::ZERO;
            state.extra_node = !state.extra_node;
            invalidation = Invalidation::Structure;
        }
    }
    invalidation
}

fn report_frame_timing(state: &mut State, frame: FrameInfo) {
    if frame.delta.is_zero() {
        return;
    }
    let timing = latest_frame_timing_stats();
    state.report_elapsed += frame.delta;
    state.report_frames += 1;
    state.report_slow += u64::from(frame.delta > Duration::from_millis(25));
    state.report_max_delta = state.report_max_delta.max(frame.delta);
    state.report_max_work_us = state.report_max_work_us.max(timing.total_us);
    state.report_max_paint_us = state.report_max_paint_us.max(timing.paint_us);
    if state.report_elapsed >= REPORT_INTERVAL {
        let fps = state.report_frames as f64 / state.report_elapsed.as_secs_f64();
        println!(
            "tiles={} auto={} unrelated_changes={} observed_fps={fps:.1} intervals_over_25ms={}/{} worst_interval_ms={:.1} max_frame_work_ms={:.1} max_paint_ms={:.1}",
            state.tiles,
            state.auto,
            state.changes,
            state.report_slow,
            state.report_frames,
            state.report_max_delta.as_secs_f64() * 1000.0,
            state.report_max_work_us as f64 / 1000.0,
            state.report_max_paint_us as f64 / 1000.0,
        );
        state.report_elapsed = Duration::ZERO;
        state.report_frames = 0;
        state.report_slow = 0;
        state.report_max_delta = Duration::ZERO;
        state.report_max_work_us = 0;
        state.report_max_paint_us = 0;
    }
}

fn action(actions: &Arc<AtomicU64>, flag: u64) -> impl Fn() + Send + Sync + 'static {
    let actions = Arc::clone(actions);
    move || {
        actions.fetch_or(flag, Ordering::Relaxed);
    }
}

fn build_ui(state: &State) -> Node {
    let phase = if state.alternate {
        "tile warm"
    } else {
        "tile cool"
    };
    let mut wall = Node::element("div").with_class("wall");
    for _ in 0..state.tiles {
        wall = wall.with_child(Node::element("div").with_class(phase).into());
    }
    let mut unrelated = Node::element("div").with_class("unrelated-box");
    if state.extra_node {
        unrelated = unrelated.with_child(Node::element("div").with_class("unrelated-dot").into());
    }
    let reference_class = if state.alternate {
        "reference warm"
    } else {
        "reference cool"
    };
    let status = format!(
        "{} tiles  |  Auto: {}  |  Unrelated changes: {}",
        state.tiles,
        if state.auto { "ON" } else { "OFF" },
        if state.changes { "ON" } else { "OFF" }
    );
    ui! {
        <div id="app">
            <div class="header">
                <h1>Transition stress test</h1>
                <p>{status}</p>
                <p class="hint">Watch the switch and tiles. Enable unrelated changes to compare.</p>
            </div>
            <div class="controls">
                <button onclick_shared={action(&state.actions, TOGGLE)}>Toggle now</button>
                <button onclick_shared={action(&state.actions, TOGGLE_AUTO)}>Auto on / off</button>
                <button onclick_shared={action(&state.actions, TOGGLE_CHANGES)}>Unrelated changes on / off</button>
                <button onclick_shared={action(&state.actions, REMOVE_TILES)}>-250 tiles</button>
                <button onclick_shared={action(&state.actions, ADD_TILES)}>+250 tiles</button>
            </div>
            <div class="reference-row">
                <div class={reference_class}><div class="knob"></div></div>
                <p>Reference switch</p>
                <p class="hint">Yellow marker appears and disappears:</p>
                {Node::Element(unrelated)}
            </div>
            {Node::Element(wall)}
            <p class="hint">Some tiles may be clipped. Frame timing is printed in the terminal.</p>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(index: u64, millis: u64) -> FrameInfo {
        FrameInfo {
            frame_index: index,
            delta: Duration::from_millis(millis),
        }
    }

    #[test]
    fn controls_and_timers_change_only_the_requested_state() {
        let mut state = State::new(1000);
        state.actions.store(TOGGLE_CHANGES, Ordering::Relaxed);
        assert_eq!(update(&mut state, frame(0, 1)), Invalidation::Layout);
        assert!(state.changes);
        assert!(!state.extra_node);
        assert_eq!(update(&mut state, frame(1, 300)), Invalidation::Structure);
        assert!(state.extra_node);
        assert_eq!(update(&mut state, frame(2, 300)), Invalidation::Structure);
        assert!(!state.extra_node);
        state.actions.store(TOGGLE, Ordering::Relaxed);
        assert_eq!(update(&mut state, frame(3, 1)), Invalidation::Layout);
        assert!(state.alternate);
    }

    #[test]
    fn scene_changes_keep_the_reference_transition_running() {
        let stylesheet = parse_stylesheet(include_str!("transition_stress_gui.css")).unwrap();
        let mut state = State::new(20);
        state.auto = false;
        let mut app = App::new(state, &stylesheet, update, build_ui);
        app.frame(frame(0, 16));
        app.state()
            .actions
            .store(TOGGLE | TOGGLE_CHANGES, Ordering::Relaxed);
        app.frame(frame(1, 16));
        assert!(app.latest_stats().transition_active);
        assert_eq!(app.latest_stats().transition_elapsed_us, 16_000);
        app.frame(frame(2, 300));
        assert!(app.latest_stats().transition_active);
        assert_eq!(app.latest_stats().transition_elapsed_us, 316_000);
    }
}
