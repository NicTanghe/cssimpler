use std::sync::OnceLock;

use anyhow::Result;
use cssimpler::core::RenderNode;
use cssimpler::renderer::{FrameInfo, WindowConfig};
use cssimpler::style::{Stylesheet, build_render_tree, parse_stylesheet};
use cssimpler::ui;

#[derive(Debug, Default)]
struct DemoState {
    frame_index: u64,
    last_frame_ms: u128,
}

fn main() -> Result<()> {
    let config = WindowConfig::new("cssimpler", 960, 540);
    let mut state = DemoState::default();

    cssimpler_renderer::run(config, move |frame| {
        update(&mut state, frame);
        render(&state)
    })
    .map_err(Into::into)
}

fn update(state: &mut DemoState, frame: FrameInfo) {
    state.frame_index = frame.frame_index;
    state.last_frame_ms = frame.delta.as_millis();
}

fn render(state: &DemoState) -> Vec<RenderNode> {
    let ui = build_ui(state);
    vec![build_render_tree(&ui, stylesheet())]
}

fn build_ui(state: &DemoState) -> cssimpler::core::Node {
    ui! {
        <div id="app">
            <section class="card">
                <h1 class="title">
                    {"Rust-native UI"}
                </h1>
                <p class="subtitle">
                    {format!("frame {}  dt={}ms", state.frame_index, state.last_frame_ms)}
                </p>
            </section>
        </div>
    }
}

fn stylesheet() -> &'static Stylesheet {
    static STYLESHEET: OnceLock<Stylesheet> = OnceLock::new();

    STYLESHEET.get_or_init(|| {
        parse_stylesheet(include_str!("demo.css")).expect("demo stylesheet should stay valid")
    })
}
