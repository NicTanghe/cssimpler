use std::sync::OnceLock;

use anyhow::Result;
use cssimpler_core::{Node, RenderNode};
use cssimpler_renderer::{FrameInfo, WindowConfig};
use cssimpler_style::{Stylesheet, build_render_tree, parse_stylesheet};

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

fn build_ui(state: &DemoState) -> Node {
    Node::element("div")
        .with_id("app")
        .with_child(
            Node::element("section")
                .with_class("card")
                .with_child(
                    Node::element("h1")
                        .with_class("title")
                        .with_child(Node::text("Rust-native UI"))
                        .into(),
                )
                .with_child(
                    Node::element("p")
                        .with_class("subtitle")
                        .with_child(Node::text(format!(
                            "frame {}  dt={}ms",
                            state.frame_index, state.last_frame_ms
                        )))
                        .into(),
                )
                .into(),
        )
        .into()
}

fn stylesheet() -> &'static Stylesheet {
    static STYLESHEET: OnceLock<Stylesheet> = OnceLock::new();

    STYLESHEET.get_or_init(|| {
        parse_stylesheet(include_str!("demo.css")).expect("demo stylesheet should stay valid")
    })
}
