use std::sync::OnceLock;

use anyhow::Result;
use cssimpler::core::RenderNode;
use cssimpler::renderer::WindowConfig;
use cssimpler::style::{Stylesheet, build_render_tree_in_viewport, parse_stylesheet};
use cssimpler::ui;

fn main() -> Result<()> {
    let config = WindowConfig::new("cssimpler / drop shadow", 980, 640);
    cssimpler_renderer::run_with_viewport(config, |_, viewport| {
        render(viewport.width, viewport.height)
    })
    .map_err(Into::into)
}

fn render(viewport_width: usize, viewport_height: usize) -> Vec<RenderNode> {
    let ui = build_ui();
    vec![build_render_tree_in_viewport(
        &ui,
        stylesheet(),
        viewport_width,
        viewport_height,
    )]
}

fn build_ui() -> cssimpler::core::Node {
    ui! {
        <div id="app">
            <section class="hero-card">
                <p class="eyebrow">
                    Example / Epoch E
                </p>
                <h1 class="title">
                    Drop shadow, rounded corners, border, and clipping
                </h1>
                <p class="copy">
                    The main card uses box-shadow and border-radius, while the preview strip clips overflowing content.
                </p>
                <div class="preview-frame">
                    <div class="preview-strip">
                        <p class="pill pill-a">shadow</p>
                        <p class="pill pill-b">radius</p>
                        <p class="pill pill-c">border</p>
                        <p class="pill pill-d">overflow</p>
                        <p class="pill pill-a">clip</p>
                        <p class="pill pill-b">native</p>
                    </div>
                </div>
                <div class="action-row">
                    <div class="ghost-button">
                        Secondary surface
                    </div>
                    <div class="primary-button">
                        Primary shadow
                    </div>
                </div>
            </section>
        </div>
    }
}

fn stylesheet() -> &'static Stylesheet {
    static STYLESHEET: OnceLock<Stylesheet> = OnceLock::new();

    STYLESHEET.get_or_init(|| {
        parse_stylesheet(include_str!("drop_shadow.css"))
            .expect("drop shadow stylesheet should stay valid")
    })
}
