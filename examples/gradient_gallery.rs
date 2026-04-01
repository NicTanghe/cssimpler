use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Result;
use cssimpler::app::{App, Invalidation};
use cssimpler::core::{
    BackgroundLayer, ElementInteractionState, GradientInterpolation, Node, RenderNode,
};
use cssimpler::renderer::{FrameInfo, SceneProvider, ViewportSize, WindowConfig};
use cssimpler::style::{Stylesheet, parse_stylesheet};
use cssimpler::ui;

const ACTION_TOGGLE_INTERPOLATION: u64 = 1 << 0;

static ACTIONS: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct GalleryState {
    interpolation: GradientInterpolation,
}

type GalleryApp = App<
    'static,
    GalleryState,
    fn(&mut GalleryState, FrameInfo) -> Invalidation,
    fn(&GalleryState) -> Node,
>;

struct GradientGalleryProvider {
    app: GalleryApp,
    scene: Vec<RenderNode>,
}

impl GradientGalleryProvider {
    fn new() -> Self {
        Self {
            app: App::new(
                GalleryState::default(),
                stylesheet(),
                update as fn(&mut GalleryState, FrameInfo) -> Invalidation,
                build_ui as fn(&GalleryState) -> Node,
            ),
            scene: Vec::new(),
        }
    }
}

impl SceneProvider for GradientGalleryProvider {
    fn update(&mut self, frame: FrameInfo) {
        <GalleryApp as SceneProvider>::update(&mut self.app, frame);
    }

    fn scene(&self) -> &[RenderNode] {
        &self.scene
    }

    fn capture_scene(&mut self) -> Vec<RenderNode> {
        let mut scene = <GalleryApp as SceneProvider>::capture_scene(&mut self.app);
        apply_gradient_interpolation(&mut scene, self.app.state().interpolation);
        self.scene = scene.clone();
        scene
    }

    fn set_viewport(&mut self, viewport: ViewportSize) {
        <GalleryApp as SceneProvider>::set_viewport(&mut self.app, viewport);
    }

    fn set_element_interaction(&mut self, interaction: ElementInteractionState) -> bool {
        <GalleryApp as SceneProvider>::set_element_interaction(&mut self.app, interaction)
    }
}

fn main() -> Result<()> {
    let config = WindowConfig::new("cssimpler / gradient gallery", 1280, 840);
    cssimpler::renderer::run_with_scene_provider(config, GradientGalleryProvider::new())
        .map_err(Into::into)
}

fn update(state: &mut GalleryState, _frame: FrameInfo) -> Invalidation {
    let actions = ACTIONS.swap(0, Ordering::Relaxed);
    if actions == 0 {
        return Invalidation::Clean;
    }

    if actions & ACTION_TOGGLE_INTERPOLATION != 0 {
        state.interpolation = next_interpolation(state.interpolation);
    }

    Invalidation::Paint
}

fn build_ui(state: &GalleryState) -> Node {
    ui! {
        <div id="app">
            <section class="hero">
                <div class="hero-copy">
                    <p class="eyebrow">
                        {"Example / Gradient gallery"}
                    </p>
                    <h1 class="hero-title">
                        {"All supported gradient types in one scene"}
                    </h1>
                    <p class="hero-note">
                        {format!(
                            "{} is active. Compare the same stops in {}.",
                            current_mode_label(state.interpolation),
                            next_mode_label(state.interpolation),
                        )}
                    </p>
                </div>
                {build_mode_switch(state)}
            </section>
            <section class="gallery">
                <article class="swatch linear-card">
                    <p class="swatch-type">{"linear-gradient"}</p>
                    <h2 class="swatch-title">{"Linear dusk blend"}</h2>
                    <p class="swatch-note">{"A diagonal ramp that makes interpolation differences easy to spot."}</p>
                </article>
                <article class="swatch repeating-linear-card">
                    <p class="swatch-type">{"repeating-linear-gradient"}</p>
                    <h2 class="swatch-title">{"Looped ribbon"}</h2>
                    <p class="swatch-note">{"Repeating stripes keep the stop layout fixed while the blend mode changes."}</p>
                </article>
                <article class="swatch radial-card">
                    <p class="swatch-type">{"radial-gradient"}</p>
                    <h2 class="swatch-title">{"Bloom core"}</h2>
                    <p class="swatch-note">{"The center glow shows how each mode handles warm transitions."}</p>
                </article>
                <article class="swatch repeating-radial-card">
                    <p class="swatch-type">{"repeating-radial-gradient"}</p>
                    <h2 class="swatch-title">{"Pulse bloom"}</h2>
                    <p class="swatch-note">{"Repeated rings make subtle interpolation shifts much easier to compare."}</p>
                </article>
                <article class="swatch conic-card">
                    <p class="swatch-type">{"conic-gradient"}</p>
                    <h2 class="swatch-title">{"Angular sweep"}</h2>
                    <p class="swatch-note">{"A full rotation around the center shows how hues bridge across sectors."}</p>
                </article>
                <article class="swatch repeating-conic-card">
                    <p class="swatch-type">{"repeating-conic-gradient"}</p>
                    <h2 class="swatch-title">{"Pinwheel loop"}</h2>
                    <p class="swatch-note">{"Short repeated sectors highlight the difference between perceptual and linear blends."}</p>
                </article>
            </section>
        </div>
    }
}

fn build_mode_switch(state: &GalleryState) -> Node {
    Node::element("button")
        .with_class("mode-switch")
        .on_click(toggle_interpolation)
        .with_child(
            Node::element("p")
                .with_class("mode-kicker")
                .with_child(Node::text("Interpolation"))
                .into(),
        )
        .with_child(
            Node::element("div")
                .with_class("mode-toggle-row")
                .with_child(
                    Node::element("h2")
                        .with_class("mode-value")
                        .with_child(Node::text(current_mode_label(state.interpolation)))
                        .into(),
                )
                .with_child(
                    Node::element("p")
                        .with_class("mode-toggle-badge")
                        .with_child(Node::text(format!(
                            "vs {}",
                            next_mode_label(state.interpolation)
                        )))
                        .into(),
                )
                .into(),
        )
        .with_child(
            Node::element("p")
                .with_class("mode-caption")
                .with_child(Node::text("Same stops, different blend math."))
                .into(),
        )
        .into()
}

fn toggle_interpolation() {
    ACTIONS.fetch_or(ACTION_TOGGLE_INTERPOLATION, Ordering::Relaxed);
}

fn next_interpolation(interpolation: GradientInterpolation) -> GradientInterpolation {
    match interpolation {
        GradientInterpolation::Oklab => GradientInterpolation::LinearSrgb,
        GradientInterpolation::LinearSrgb => GradientInterpolation::Oklab,
    }
}

fn current_mode_label(interpolation: GradientInterpolation) -> &'static str {
    match interpolation {
        GradientInterpolation::Oklab => "Oklab",
        GradientInterpolation::LinearSrgb => "Linear sRGB",
    }
}

fn next_mode_label(interpolation: GradientInterpolation) -> &'static str {
    current_mode_label(next_interpolation(interpolation))
}

fn apply_gradient_interpolation(nodes: &mut [RenderNode], interpolation: GradientInterpolation) {
    for node in nodes {
        apply_node_gradient_interpolation(node, interpolation);
    }
}

fn apply_node_gradient_interpolation(node: &mut RenderNode, interpolation: GradientInterpolation) {
    for layer in &mut node.style.background_layers {
        match layer {
            BackgroundLayer::LinearGradient(gradient) => gradient.interpolation = interpolation,
            BackgroundLayer::RadialGradient(gradient) => gradient.interpolation = interpolation,
            BackgroundLayer::ConicGradient(gradient) => gradient.interpolation = interpolation,
        }
    }

    apply_gradient_interpolation(&mut node.children, interpolation);
}

fn stylesheet() -> &'static Stylesheet {
    static STYLESHEET: OnceLock<Stylesheet> = OnceLock::new();

    STYLESHEET.get_or_init(|| {
        parse_stylesheet(include_str!("gradient_gallery.css"))
            .expect("gradient gallery stylesheet should stay valid")
    })
}

#[cfg(test)]
mod tests {
    use cssimpler::core::RenderKind;
    use cssimpler::style::build_render_tree_in_viewport;

    use super::{GalleryState, RenderNode, build_mode_switch, stylesheet};

    #[test]
    fn mode_toggle_stays_a_container_so_the_full_button_remains_clickable() {
        let tree = build_mode_switch(&GalleryState::default());
        let scene = build_render_tree_in_viewport(&tree, stylesheet(), 1280, 840);
        let toggle = find_clickable_node(&scene).expect("mode toggle should be rendered");

        assert!(matches!(toggle.kind, RenderKind::Container));
        assert!(toggle.layout.width > 200.0);
        assert!(toggle.layout.height >= 80.0);
        assert!(count_text_nodes(toggle) >= 3);
    }

    fn find_clickable_node(node: &RenderNode) -> Option<&RenderNode> {
        if node.on_click.is_some() {
            return Some(node);
        }

        for child in &node.children {
            if let Some(node) = find_clickable_node(child) {
                return Some(node);
            }
        }

        None
    }

    fn count_text_nodes(node: &RenderNode) -> usize {
        let own = usize::from(matches!(node.kind, RenderKind::Text(_)));
        own + node.children.iter().map(count_text_nodes).sum::<usize>()
    }
}
