use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Result;
use cssimpler::app::{App, Invalidation, RenderMode};
use cssimpler::renderer::{FrameInfo, WindowConfig};
use cssimpler::style::parse_stylesheet;
use cssimpler::ui;

#[derive(Debug, Default)]
struct DemoState {
    click_count: Arc<AtomicU64>,
    frame_index: u64,
    last_frame_ms: u128,
}

fn main() -> Result<()> {
    let config = WindowConfig::new("cssimpler", 960, 540);
    let stylesheet = parse_stylesheet(include_str!("demo.css"))?;
    App::new(DemoState::default(), &stylesheet, update, build_ui)
        //weirdly whith OnInvalifation it is still rendereing out at 60fps
        .with_render_mode(RenderMode::OnInvalidation)
        .run(config)
        .map_err(Into::into)
}

fn update(state: &mut DemoState, frame: FrameInfo) -> Invalidation {
    state.frame_index = frame.frame_index;
    state.last_frame_ms = frame.delta.as_millis();
    Invalidation::Paint
}

fn build_ui(state: &DemoState) -> cssimpler::core::Node {
    let count = state.click_count.load(Ordering::Relaxed);
    let click_count = Arc::clone(&state.click_count);
    ui! {
        <div id="app">
            <section class="card">
                <h1 class="title">
                    Increment button demo
                </h1>
                <p class="subtitle">
                    {format!("count {count}")}
                </p>
                <button class="button" onclick_shared={move || { click_count.fetch_add(1, Ordering::Relaxed); }}>
                    Increment
                </button>
                <p class="meta">
                    {format!("frame {}  dt={}ms", state.frame_index, state.last_frame_ms)}
                </p>
            </section>
        </div>
    }
}
