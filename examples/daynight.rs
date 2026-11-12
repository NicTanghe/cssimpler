//! Isolated reproduction of FoldCraft's day/night theme switch.
//! Run with `cargo run --example daynight`.
//! Inspired by Benjamin Réthoré and mobinkakei's MIT-licensed switch:
//! https://uiverse.io/mobinkakei/strong-lizard-79

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

use anyhow::Result;
use cssimpler::app::{App, Invalidation};
use cssimpler::core::Node;
use cssimpler::renderer::{FrameInfo, WindowConfig};
use cssimpler::style::{Stylesheet, parse_stylesheet};
use cssimpler::ui;

#[derive(Default)]
struct State {
    actions: Arc<AtomicU64>,
    light: bool,
    simulate_scene_change: bool,
}

fn main() -> Result<()> {
    App::new(State::default(), stylesheet(), update, build_ui)
        .run(WindowConfig::new("daynight", 400, 240))
        .map_err(Into::into)
}

fn update(state: &mut State, _frame: FrameInfo) -> Invalidation {
    let actions = state.actions.swap(0, Ordering::Relaxed);
    if actions == 0 {
        return Invalidation::Clean;
    }
    if actions & 1 != 0 {
        state.light = !state.light;
    }
    if actions & 2 != 0 {
        state.simulate_scene_change = !state.simulate_scene_change;
    }
    Invalidation::Layout
}

fn build_ui(state: &State) -> Node {
    let class = if state.light {
        "day-night-switch light"
    } else {
        "day-night-switch dark"
    };
    let actions = Arc::clone(&state.actions);
    let diagnostic_actions = Arc::clone(&state.actions);
    let diagnostic_label = if state.simulate_scene_change {
        "TEST: unrelated scene change ON"
    } else {
        "TEST: unrelated scene change OFF"
    };
    // Deliberately add/remove a separate node when the switch changes. This
    // tests the whole-scene structure check; it is not part of the switch.
    let diagnostic_node = if state.simulate_scene_change && state.light {
        ui! { <div class="diagnostic-placeholder"><span class="diagnostic-marker"></span></div> }
    } else {
        ui! { <div class="diagnostic-placeholder"></div> }
    };
    ui! {
        <div id="app">
            <button id="theme-toggle" title="Toggle day and night" onclick_shared={move || { actions.fetch_or(1, Ordering::Relaxed); }}>
                <div class={class}>
                    <span class="sky-shape shape1"></span>
                    <span class="sky-shape shape2"></span>
                    <span class="sky-shape shape3"></span>
                    <span class="sky-shape shape4"></span>
                    <span class="sky-shape shape5"></span>
                    <div class="switch-thumb">
                        <span class="crater crater1"></span>
                        <span class="crater crater2"></span>
                        <span class="crater crater3"></span>
                    </div>
                </div>
            </button>
            <button class="diagnostic-toggle" onclick_shared={move || { diagnostic_actions.fetch_or(2, Ordering::Relaxed); }}>
                {diagnostic_label}
            </button>
            <div class="diagnostic-area">{diagnostic_node}</div>
        </div>
    }
}

fn stylesheet() -> &'static Stylesheet {
    static STYLESHEET: OnceLock<Stylesheet> = OnceLock::new();
    STYLESHEET.get_or_init(|| {
        parse_stylesheet(include_str!("daynight.css")).expect("valid daynight stylesheet")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn frame(index: u64) -> FrameInfo {
        FrameInfo {
            frame_index: index,
            delta: Duration::from_millis(16),
        }
    }

    fn thumb_offset(scene: &[cssimpler::core::RenderNode]) -> f32 {
        fn find<'a>(
            nodes: &'a [cssimpler::core::RenderNode],
            path: &[usize],
        ) -> Option<&'a cssimpler::core::RenderNode> {
            for node in nodes {
                if node
                    .element_path
                    .as_ref()
                    .is_some_and(|p| p.root == 0 && p.children == path)
                {
                    return Some(node);
                }
                if let Some(found) = find(&node.children, path) {
                    return Some(found);
                }
            }
            None
        }
        let switch = find(scene, &[0, 0]).expect("switch should be rendered");
        let thumb = find(scene, &[0, 0, 5]).expect("thumb should be rendered");
        thumb.layout.x - switch.layout.x
    }

    #[test]
    fn unrelated_structure_change_does_not_cancel_toggle_transition() {
        let state = State {
            simulate_scene_change: true,
            ..State::default()
        };
        let mut app = App::new(state, stylesheet(), update, build_ui);
        app.frame(frame(0));
        app.state().actions.store(1, Ordering::Relaxed);
        app.frame(frame(1));
        assert!(app.latest_stats().transition_active);
        assert_eq!(app.latest_stats().transition_elapsed_us, 16_000);
        assert_eq!(app.latest_stats().transition_duration_us, 400_000);
    }

    #[test]
    fn unrelated_update_during_transition_does_not_restart_clock() {
        let mut app = App::new(State::default(), stylesheet(), update, build_ui);
        let initial_offset = thumb_offset(&app.frame(frame(0)));
        app.state().actions.store(1, Ordering::Relaxed);
        let first_offset = thumb_offset(&app.frame(frame(1)));
        assert!(first_offset < initial_offset);
        assert_eq!(app.latest_stats().transition_elapsed_us, 16_000);
        app.state().actions.store(2, Ordering::Relaxed);
        let second_offset = thumb_offset(&app.frame(frame(2)));
        assert!(
            second_offset < first_offset,
            "thumb should keep moving after the scene change"
        );
        assert!(app.latest_stats().transition_active);
        assert_eq!(app.latest_stats().transition_elapsed_us, 32_000);
        app.state().actions.store(2, Ordering::Relaxed);
        let third_offset = thumb_offset(&app.frame(frame(3)));
        assert!(
            third_offset < second_offset,
            "thumb should keep moving after removal"
        );
        assert!(app.latest_stats().transition_active);
        assert_eq!(app.latest_stats().transition_elapsed_us, 48_000);
    }

    #[test]
    fn repeated_toggles_with_unrelated_changes_keep_a_transition_active() {
        let state = State {
            simulate_scene_change: true,
            ..State::default()
        };
        let mut app = App::new(state, stylesheet(), update, build_ui);
        let mut previous = thumb_offset(&app.frame(frame(0)));
        for index in 1..=6 {
            app.state().actions.store(1, Ordering::Relaxed);
            let current = thumb_offset(&app.frame(frame(index)));
            assert!(
                app.latest_stats().transition_active,
                "toggle {index} should animate"
            );
            if index % 2 == 1 {
                assert!(current < previous, "thumb should move toward day");
            } else {
                assert!(current > previous, "thumb should move toward night");
            }
            previous = current;
        }
    }
}
