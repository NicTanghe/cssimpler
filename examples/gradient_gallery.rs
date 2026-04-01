use std::sync::OnceLock;

use anyhow::Result;
use cssimpler::core::RenderNode;
use cssimpler::renderer::WindowConfig;
use cssimpler::style::{Stylesheet, build_render_tree_in_viewport, parse_stylesheet};
use cssimpler::ui;

fn main() -> Result<()> {
    let config = WindowConfig::new("cssimpler / gradient gallery", 1280, 840);
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
            <section class="hero">
                <p class="eyebrow">
                    {"Example / Gradient gallery"}
                </p>
                <h1 class="hero-title">
                    {"All supported gradient types in one scene"}
                </h1>
                <p class="hero-note">
                    {"Layered gradients set the stage, and six cards show the modes."}
                </p>
            </section>
            <section class="gallery">
                <article class="swatch linear-card">
                    <p class="swatch-type">{"linear-gradient"}</p>
                    <h2 class="swatch-title">{"Linear dusk blend"}</h2>
                    <p class="swatch-note">{"A smooth diagonal ramp rendered in linear light."}</p>
                </article>
                <article class="swatch repeating-linear-card">
                    <p class="swatch-type">{"repeating-linear-gradient"}</p>
                    <h2 class="swatch-title">{"Looped ribbon"}</h2>
                    <p class="swatch-note">{"A smooth repeating ribbon still uses fixed-length stops."}</p>
                </article>
                <article class="swatch radial-card">
                    <p class="swatch-type">{"radial-gradient"}</p>
                    <h2 class="swatch-title">{"Bloom core"}</h2>
                    <p class="swatch-note">{"A bright center fades outward into a darker edge."}</p>
                </article>
                <article class="swatch repeating-radial-card">
                    <p class="swatch-type">{"repeating-radial-gradient"}</p>
                    <h2 class="swatch-title">{"Pulse bloom"}</h2>
                    <p class="swatch-note">{"Repeating radial stops fade in and out from the center."}</p>
                </article>
                <article class="swatch conic-card">
                    <p class="swatch-type">{"conic-gradient"}</p>
                    <h2 class="swatch-title">{"Angular sweep"}</h2>
                    <p class="swatch-note">{"Color rotates around the midpoint in a continuous sweep."}</p>
                </article>
                <article class="swatch repeating-conic-card">
                    <p class="swatch-type">{"repeating-conic-gradient"}</p>
                    <h2 class="swatch-title">{"Pinwheel loop"}</h2>
                    <p class="swatch-note">{"Repeating angles blend into a continuous spinning fan."}</p>
                </article>
            </section>
        </div>
    }
}

fn stylesheet() -> &'static Stylesheet {
    static STYLESHEET: OnceLock<Stylesheet> = OnceLock::new();

    STYLESHEET.get_or_init(|| {
        parse_stylesheet(include_str!("gradient_gallery.css"))
            .expect("gradient gallery stylesheet should stay valid")
    })
}
