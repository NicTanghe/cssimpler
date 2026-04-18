use std::path::PathBuf;
use std::sync::OnceLock;

use anyhow::{Context, Result, anyhow};
use cssimpler::app::{App, Invalidation};
use cssimpler::core::Node;
use cssimpler::fonts::register_font_file;
use cssimpler::renderer::{FrameInfo, WindowConfig};
use cssimpler::style::{Stylesheet, parse_stylesheet};
use cssimpler::ui;

const DEMO_FONT_ASSET: &str = "examples/assets/powerline-demo.ttf";

#[derive(Debug)]
struct DemoState {
    bundled_family: String,
}

fn main() -> Result<()> {
    let bundled_family = register_demo_font()?;
    let config = WindowConfig::new("cssimpler / powerline typography", 1280, 760);

    App::new(
        DemoState {
            bundled_family: bundled_family.clone(),
        },
        stylesheet(&bundled_family),
        update,
        build_ui,
    )
    .run(config)
    .map_err(Into::into)
}

fn update(_state: &mut DemoState, _frame: FrameInfo) -> Invalidation {
    Invalidation::Clean
}

fn build_ui(state: &DemoState) -> Node {
    ui! {
        <div id="app">
            <section class="hero">
                <p class="eyebrow">
                    H5 / arbitrary font demo
                </p>
                <h1 class="hero-title">
                    Project-local Powerline font registration
                </h1>
                <p class="hero-copy">
                    The left panel keeps the Windows-friendly system stack. The right panel registers a bundled TTF from examples/assets and uses it for both layout and paint.
                </p>
            </section>
            <section class="comparison-grid">
                <article class="panel panel-system">
                    <p class="panel-label">
                        System font baseline
                    </p>
                    <h2 class="panel-title">
                        Segoe UI / Arial / sans-serif
                    </h2>
                    <p class="panel-copy">
                        This sample keeps the normal desktop stack so you can compare the metrics and texture against the bundled font card.
                    </p>
                    <p class="system-line">
                        Library review queue 0123456789
                    </p>
                    <p class="system-line system-line-soft">
                        Wrapped layout should stay proportional here.
                    </p>
                </article>
                <article class="panel panel-bundled">
                    <p class="panel-label panel-label-bundled">
                        Bundled asset
                    </p>
                    <h2 class="panel-title panel-title-bundled">
                        {state.bundled_family.clone()}
                    </h2>
                    <p class="panel-copy panel-copy-bundled">
                        {format!("Loaded from {DEMO_FONT_ASSET} at startup.")}
                    </p>
                    <p class="powerline-line">
                        "repo   main    cargo test   pass"
                    </p>
                    <p class="powerline-line powerline-line-soft">
                        Glyph coverage now comes from the registered TTF instead of the bitmap fallback.
                    </p>
                </article>
            </section>
            <section class="notes">
                <article class="note-card">
                    <p class="note-label">
                        Why this matters
                    </p>
                    <p class="note-copy">
                        Changing font family changes text measurement, wrapping, and the final pixels. H5 is about proving the engine owns that end-to-end.
                    </p>
                </article>
                <article class="note-card">
                    <p class="note-label">
                        Bundled font
                    </p>
                    <p class="note-copy">
                        Anonymice Powerline ships with the repo so the demo does not depend on system installation state.
                    </p>
                </article>
            </section>
        </div>
    }
}

fn register_demo_font() -> Result<String> {
    let asset_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(DEMO_FONT_ASSET);
    let families = register_font_file(&asset_path)
        .with_context(|| format!("failed to register demo font at {}", asset_path.display()))?;

    families.into_iter().next().ok_or_else(|| {
        anyhow!(
            "font registration succeeded but no family names were discovered for {}",
            asset_path.display()
        )
    })
}

fn stylesheet(bundled_family: &str) -> &'static Stylesheet {
    static STYLESHEET: OnceLock<Stylesheet> = OnceLock::new();

    STYLESHEET.get_or_init(|| {
        let source = include_str!("powerline_typography.css")
            .replace("__POWERLINE_FONT__", &escape_css_string(bundled_family));
        parse_stylesheet(&source).expect("powerline typography stylesheet should stay valid")
    })
}

fn escape_css_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}
