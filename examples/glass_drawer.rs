use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Result;
use cssimpler::app::{App, Invalidation};
use cssimpler::core::{Color, Node};
use cssimpler::renderer::{FrameInfo, WindowConfig};
use cssimpler::style::{Stylesheet, parse_stylesheet};
use cssimpler::ui;

const ACTION_SELECT_MICA: u64 = 1 << 0;
const ACTION_SELECT_ACRYLIC: u64 = 1 << 1;
const ACTION_CYCLE_HUE: u64 = 1 << 2;
const ACTION_CYCLE_VALUE: u64 = 1 << 3;
const HUE_COUNT: usize = 6;
const VALUE_COUNT: usize = 3;

static ACTIONS: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GlassMode {
    Mica,
    Acrylic,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct GlassState {
    mode: GlassMode,
    hue: usize,
    value: usize,
}

impl Default for GlassState {
    fn default() -> Self {
        Self {
            mode: GlassMode::Acrylic,
            hue: 3,
            value: 0,
        }
    }
}

fn main() -> Result<()> {
    let config = WindowConfig::new("cssimpler / native glass picker", 900, 560)
        .with_glass_capable(true)
        .with_decorations(false);

    App::new(GlassState::default(), stylesheet(), update, build_ui)
        .run(WindowConfig {
            clear_color: Color::rgb(14, 18, 24),
            ..config
        })
        .map_err(Into::into)
}

fn update(state: &mut GlassState, _frame: FrameInfo) -> Invalidation {
    let actions = ACTIONS.swap(0, Ordering::Relaxed);
    if actions == 0 {
        return Invalidation::Clean;
    }

    if actions & ACTION_SELECT_MICA != 0 {
        state.mode = GlassMode::Mica;
    }
    if actions & ACTION_SELECT_ACRYLIC != 0 {
        state.mode = GlassMode::Acrylic;
    }
    if actions & ACTION_CYCLE_HUE != 0 {
        state.mode = GlassMode::Acrylic;
        state.hue = (state.hue + 1) % HUE_COUNT;
    }
    if actions & ACTION_CYCLE_VALUE != 0 {
        state.mode = GlassMode::Acrylic;
        state.value = (state.value + 1) % VALUE_COUNT;
    }

    Invalidation::Layout
}

fn build_ui(state: &GlassState) -> Node {
    add_classes(
        ui! {
            <div id="app">
                <section class="control-panel">
                    <div class="mode-switch">
                        {mode_button("MICA", select_mica, state.mode == GlassMode::Mica)}
                        {mode_button("ACRYLIC", select_acrylic, state.mode == GlassMode::Acrylic)}
                    </div>

                    <div class="picker-row">
                        <button class="sv-picker" type="button" onclick={cycle_value}>
                            {add_class(ui! { <span class="sv-knob"></span> }, value_class(state.value))}
                        </button>
                        <button class="hue-strip" type="button" onclick={cycle_hue}>
                            {add_class(ui! { <span class="hue-knob"></span> }, hue_class(state.hue))}
                        </button>
                    </div>
                </section>
            </div>
        },
        [
            mode_class(state.mode),
            tint_class(state.hue),
            value_class(state.value),
        ],
    )
}

fn mode_button(label: &'static str, handler: fn(), selected: bool) -> Node {
    let button = ui! {
        <button class="mode-segment" type="button" onclick={handler}>
            {label}
        </button>
    };

    if selected {
        add_class(button, "selected")
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

fn add_classes<const N: usize>(node: Node, class_names: [&'static str; N]) -> Node {
    class_names.into_iter().fold(node, add_class)
}

fn mode_class(mode: GlassMode) -> &'static str {
    match mode {
        GlassMode::Mica => "mode-mica",
        GlassMode::Acrylic => "mode-acrylic",
    }
}

fn tint_class(hue: usize) -> &'static str {
    match hue % HUE_COUNT {
        0 => "tint-red",
        1 => "tint-yellow",
        2 => "tint-green",
        3 => "tint-cyan",
        4 => "tint-blue",
        _ => "tint-magenta",
    }
}

fn value_class(value: usize) -> &'static str {
    match value % VALUE_COUNT {
        0 => "value-bright",
        1 => "value-mid",
        _ => "value-deep",
    }
}

fn hue_class(hue: usize) -> &'static str {
    match hue % HUE_COUNT {
        0 => "hue-red",
        1 => "hue-yellow",
        2 => "hue-green",
        3 => "hue-cyan",
        4 => "hue-blue",
        _ => "hue-magenta",
    }
}

fn select_mica() {
    ACTIONS.fetch_or(ACTION_SELECT_MICA, Ordering::Relaxed);
}

fn select_acrylic() {
    ACTIONS.fetch_or(ACTION_SELECT_ACRYLIC, Ordering::Relaxed);
}

fn cycle_hue() {
    ACTIONS.fetch_or(ACTION_CYCLE_HUE, Ordering::Relaxed);
}

fn cycle_value() {
    ACTIONS.fetch_or(ACTION_CYCLE_VALUE, Ordering::Relaxed);
}

fn stylesheet() -> &'static Stylesheet {
    static STYLESHEET: OnceLock<Stylesheet> = OnceLock::new();

    STYLESHEET.get_or_init(|| {
        parse_stylesheet(include_str!("glass_drawer.css"))
            .expect("native glass picker example stylesheet should stay valid")
    })
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use super::{
        ACTION_CYCLE_HUE, ACTION_SELECT_MICA, GlassMode, GlassState, build_ui, stylesheet, update,
    };
    use cssimpler::app::Invalidation;
    use cssimpler::renderer::FrameInfo;

    #[test]
    fn native_glass_picker_example_stylesheet_parses_and_builds_ui() {
        let _ = stylesheet();
        let tree = build_ui(&GlassState::default());

        let cssimpler::core::Node::Element(root) = tree else {
            panic!("root should be an element");
        };

        assert_eq!(root.children.len(), 1);
        assert!(root.classes.contains(&"mode-acrylic".to_string()));
    }

    #[test]
    fn native_glass_picker_actions_update_state() {
        let mut state = GlassState::default();
        super::ACTIONS.store(ACTION_SELECT_MICA | ACTION_CYCLE_HUE, Ordering::Relaxed);

        let invalidation = update(
            &mut state,
            FrameInfo {
                frame_index: 1,
                delta: std::time::Duration::ZERO,
            },
        );

        assert_eq!(invalidation, Invalidation::Layout);
        assert_eq!(state.mode, GlassMode::Acrylic);
        assert_eq!(state.hue, 4);
    }
}
