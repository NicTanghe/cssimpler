use std::sync::OnceLock;

use anyhow::Result;
use cssimpler::app::{App, Invalidation};
use cssimpler::core::Node;
use cssimpler::renderer::{FrameInfo, WindowConfig};
use cssimpler::style::{Stylesheet, parse_stylesheet};
use cssimpler::ui;

fn main() -> Result<()> {
    let config = WindowConfig::new("cssimpler / backdrop blur", 1280, 780);

    App::new((), stylesheet(), update, build_ui)
        .run(config)
        .map_err(Into::into)
}

fn update(_state: &mut (), _frame: FrameInfo) -> Invalidation {
    Invalidation::Clean
}

fn build_ui(_state: &()) -> Node {
    ui! {
        <div id="app">
            <section class="scene">
                <div class="wall">
                    <div class="column column-a"></div>
                    <div class="column column-b"></div>
                    <div class="column column-c"></div>
                    <div class="column column-d"></div>
                    <div class="column column-e"></div>

                    <span class="echo echo-one">SHARP</span>
                    <span class="echo echo-two">TYPE</span>
                    <span class="echo echo-three">COLOR</span>

                    <div class="chip-row chip-row-top">
                        <span class="chip chip-cyan">NEON</span>
                        <span class="chip chip-lime">GLASS</span>
                        <span class="chip chip-pink">FROST</span>
                        <span class="chip chip-gold">FOCUS</span>
                    </div>

                    <div class="chip-row chip-row-bottom">
                        <span class="chip chip-ink">PIXELS</span>
                        <span class="chip chip-coral">EDGES</span>
                        <span class="chip chip-blue">LAYERS</span>
                        <span class="chip chip-white">DEPTH</span>
                    </div>

                    <div class="orb orb-a"></div>
                    <div class="orb orb-b"></div>
                    <div class="orb orb-c"></div>
                </div>

                <section class="comparison">
                    <article class="panel tint-panel">
                        <p class="badge">Tint only</p>
                        <h1 class="title">No backdrop filter</h1>
                        <p class="body">
                            This card uses the same translucent fill, but the sharp bars and labels behind it stay crisp.
                        </p>
                        <div class="code-chip">
                            background: rgba(255, 255, 255, 0.18)
                        </div>
                        <div class="mini-row">
                            <span class="mini mini-cyan"></span>
                            <span class="mini mini-pink"></span>
                            <span class="mini mini-gold"></span>
                        </div>
                    </article>

                    <article class="panel blur-panel">
                        <p class="badge">Backdrop blur</p>
                        <h1 class="title">Actual frosted glass</h1>
                        <p class="body">
                            This card keeps the same tint, then adds backdrop-filter so the content behind it softens inside the rounded bounds.
                        </p>
                        <div class="code-chip emphasis">
                            backdrop-filter: blur(14px)
                        </div>
                        <div class="mini-row">
                            <span class="mini mini-cyan"></span>
                            <span class="mini mini-pink"></span>
                            <span class="mini mini-gold"></span>
                        </div>
                    </article>
                </section>

                <p class="footer-note">
                    Both cards use the same translucent background. Only the right card adds backdrop-filter blur.
                </p>
            </section>
        </div>
    }
}

fn stylesheet() -> &'static Stylesheet {
    static STYLESHEET: OnceLock<Stylesheet> = OnceLock::new();

    STYLESHEET.get_or_init(|| {
        parse_stylesheet(include_str!("backdrop_blur.css"))
            .expect("backdrop blur example stylesheet should stay valid")
    })
}

#[cfg(test)]
mod tests {
    use super::{build_ui, stylesheet};

    #[test]
    fn backdrop_blur_example_stylesheet_parses_and_builds_ui() {
        let _ = stylesheet();
        let tree = build_ui(&());

        let cssimpler::core::Node::Element(root) = tree else {
            panic!("root should be an element");
        };

        assert_eq!(root.children.len(), 1);
    }
}
