use std::sync::OnceLock;

use anyhow::Result;
use cssimpler::core::RenderNode;
use cssimpler::renderer::WindowConfig;
use cssimpler::style::{Stylesheet, build_render_tree_in_viewport, parse_stylesheet};
use cssimpler::ui;

fn main() -> Result<()> {
    let config = WindowConfig::new("cssimpler / taffy layout", 1200, 720);

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
                <div class="hero-top">
                    <div class="hero-copy">
                        <p class="eyebrow">
                            Example / Taffy layout
                        </p>
                        <h1 class="hero-title">
                            Nested flex, grid, gap, and wrapping
                        </h1>
                        <p class="hero-note">
                            This scene is laid out by Taffy through the cssimpler CSS bridge.
                        </p>
                    </div>
                    <div class="hero-badge">
                        <p class="badge-label">
                            Layout mode
                        </p>
                        <p class="badge-value">
                            flex + grid
                        </p>
                    </div>
                </div>
                <div class="chip-row">
                    <p class="chip">display:flex</p>
                    <p class="chip">display:grid</p>
                    <p class="chip">gap</p>
                    <p class="chip">flex-wrap</p>
                    <p class="chip">grid-column</p>
                    <p class="chip">padding</p>
                </div>
            </section>
            <main class="workspace">
                <aside class="sidebar">
                    <h2 class="section-title">
                        Sidebar stack
                    </h2>
                    <div class="nav-stack">
                        <div class="nav-card nav-card-a">
                            <p class="card-title">Inbox</p>
                            <p class="card-note">Fixed-width column with flex children</p>
                        </div>
                        <div class="nav-card nav-card-b">
                            <p class="card-title">Review</p>
                            <p class="card-note">Spacing comes from gap + padding</p>
                        </div>
                        <div class="nav-card nav-card-c">
                            <p class="card-title">Archive</p>
                            <p class="card-note">Text leaves are measured during layout</p>
                        </div>
                    </div>
                </aside>
                <section class="content">
                    <div class="stats">
                        <article class="stat-card stat-card-a">
                            <p class="section-label">Pinned tasks</p>
                            <p class="stat-value">12</p>
                            <p class="section-note">Grid item 1 / 3</p>
                        </article>
                        <article class="stat-card stat-card-b">
                            <p class="section-label">Open reviews</p>
                            <p class="stat-value">4</p>
                            <p class="section-note">Same template track sizing</p>
                        </article>
                        <article class="stat-card stat-card-c">
                            <p class="section-label">Queued deploys</p>
                            <p class="stat-value">2</p>
                            <p class="section-note">Fractional columns share the row</p>
                        </article>
                    </div>
                    <div class="board">
                        <article class="panel panel-wide">
                            <p class="section-label">Wide panel</p>
                            <p class="section-note">
                                This panel spans both grid columns and contains a wrapping flex row.
                            </p>
                            <div class="task-row">
                                <p class="task-pill">header</p>
                                <p class="task-pill">content</p>
                                <p class="task-pill">sidebar</p>
                                <p class="task-pill">cards</p>
                                <p class="task-pill">chips</p>
                                <p class="task-pill">board</p>
                            </div>
                        </article>
                        <article class="panel panel-left">
                            <p class="section-label">Left panel</p>
                            <p class="section-note">
                                Placed at grid column 1 / row 2 with its own vertical stack.
                            </p>
                        </article>
                        <article class="panel panel-right">
                            <p class="section-label">Right panel</p>
                            <p class="section-note">
                                Placed at grid column 2 / row 2 while the board stretches to fill the remainder.
                            </p>
                        </article>
                    </div>
                </section>
            </main>
        </div>
    }
}

fn stylesheet() -> &'static Stylesheet {
    static STYLESHEET: OnceLock<Stylesheet> = OnceLock::new();

    STYLESHEET.get_or_init(|| {
        parse_stylesheet(include_str!("taffy_layout.css"))
            .expect("taffy layout stylesheet should stay valid")
    })
}
