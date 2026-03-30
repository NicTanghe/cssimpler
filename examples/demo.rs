use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Result;
use cssimpler::app::{App, Invalidation, RenderMode};
use cssimpler::renderer::{FrameInfo, WindowConfig};
use cssimpler::style::{Stylesheet, parse_stylesheet};
use cssimpler::ui;

#[derive(Debug, Default)]
struct DemoState {
    click_count: u64,
    frame_index: u64,
    last_frame_ms: u128,
}

static CLICK_COUNT: AtomicU64 = AtomicU64::new(0);

fn main() -> Result<()> {
    let config = WindowConfig::new("cssimpler", 960, 540);
    App::new(DemoState::default(), stylesheet(), update, build_ui)
        .with_render_mode(RenderMode::EveryFrame)
        .run(config)
        .map_err(Into::into)
}

fn update(state: &mut DemoState, frame: FrameInfo) -> Invalidation {
    state.click_count = CLICK_COUNT.load(Ordering::Relaxed);
    state.frame_index = frame.frame_index;
    state.last_frame_ms = frame.delta.as_millis();
    Invalidation::Paint
}

fn build_ui(state: &DemoState) -> cssimpler::core::Node {
    ui! {
        <div id="app">
            <section class="card">
                <h1 class="title">
                    {"Increment button demo"}
                </h1>
                <p class="subtitle">
                    {format!("count {}", state.click_count)}
                </p>
                <button class="button" onclick={increment}>
                    {"Increment"}
                </button>
                <p class="meta">
                    {format!("frame {}  dt={}ms", state.frame_index, state.last_frame_ms)}
                </p>
            </section>
        </div>
    }
}

fn increment() {
    CLICK_COUNT.fetch_add(1, Ordering::Relaxed);
}

fn stylesheet() -> &'static Stylesheet {
    static STYLESHEET: OnceLock<Stylesheet> = OnceLock::new();

    STYLESHEET.get_or_init(|| {
        parse_stylesheet(include_str!("demo.css")).expect("demo stylesheet should stay valid")
    })
}
