use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

use anyhow::Result;
use cssimpler::app::{App, Invalidation};
use cssimpler::core::{Color, Node};
use cssimpler::renderer::{FrameInfo, WindowConfig};
use cssimpler::style::{Stylesheet, parse_stylesheet};
use cssimpler::ui;

const ACTION_TOGGLE_DRAWER: u64 = 1 << 0;

#[derive(Debug, Default)]
struct DrawerState {
    actions: Arc<AtomicU64>,
    collapsed: bool,
}

fn main() -> Result<()> {
    let config = WindowConfig::new("cssimpler / glass drawer", 900, 560)
        .with_glass_capable(true)
        .with_decorations(false);

    App::new(DrawerState::default(), stylesheet(), update, build_ui)
        .run(WindowConfig {
            clear_color: Color::rgb(12, 15, 20),
            ..config
        })
        .map_err(Into::into)
}

fn update(state: &mut DrawerState, _frame: FrameInfo) -> Invalidation {
    let actions = state.actions.swap(0, Ordering::Relaxed);
    if actions & ACTION_TOGGLE_DRAWER == 0 {
        return Invalidation::Clean;
    }

    state.collapsed = !state.collapsed;
    Invalidation::Layout
}

fn build_ui(state: &DrawerState) -> Node {
    let actions = Arc::clone(&state.actions);
    let root = ui! {
        <div id="app">
            <aside class="drawer">
                <button class="drawer-toggle" type="button" onclick_shared={move || { actions.fetch_or(ACTION_TOGGLE_DRAWER, Ordering::Relaxed); }}>
                    <span class="hamburger-line"></span>
                    <span class="hamburger-line"></span>
                    <span class="hamburger-line"></span>
                </button>

                <div class="drawer-brand">
                    <span class="brand-mark"></span>
                    <div class="brand-copy">
                        <p class="brand-title">Librarian</p>
                        <p class="brand-subtitle">Archive desk</p>
                    </div>
                </div>

                <nav class="drawer-content">
                    <p class="section-label">Stacks</p>
                    <div class="nav-item selected">
                        <span class="nav-dot"></span>
                        <span class="nav-label">Dashboard</span>
                    </div>
                    <div class="nav-item">
                        <span class="nav-dot"></span>
                        <span class="nav-label">Returns</span>
                    </div>
                    <div class="nav-item">
                        <span class="nav-dot"></span>
                        <span class="nav-label">New holds</span>
                    </div>
                    <div class="nav-item">
                        <span class="nav-dot"></span>
                        <span class="nav-label">Community</span>
                    </div>
                </nav>

                <div class="drawer-footer">
                    <p class="footer-number">146</p>
                    <p class="footer-label">new holds</p>
                </div>
            </aside>

            <main class="workspace">
                <section class="summary-strip">
                    <div class="summary-block">
                        <p class="summary-label">Checked out</p>
                        <p class="summary-value">1,284</p>
                    </div>
                    <div class="summary-block">
                        <p class="summary-label">Available</p>
                        <p class="summary-value">8,631</p>
                    </div>
                    <div class="summary-block">
                        <p class="summary-label">Requests</p>
                        <p class="summary-value">73</p>
                    </div>
                </section>

                <section class="work-panel">
                    <p class="panel-label">Today</p>
                    <h1 class="panel-title">Quietly busy morning</h1>
                    <p class="panel-copy">Returns are moving steadily and the community desk is opening another hold shelf.</p>
                </section>
            </main>
        </div>
    };

    if state.collapsed {
        add_class(root, "drawer-collapsed")
    } else {
        add_class(root, "drawer-open")
    }
}

fn add_class(node: Node, class_name: &'static str) -> Node {
    match node {
        Node::Element(element) => element.with_class(class_name).into(),
        Node::Text(_) => node,
    }
}

fn stylesheet() -> &'static Stylesheet {
    static STYLESHEET: OnceLock<Stylesheet> = OnceLock::new();

    STYLESHEET.get_or_init(|| {
        parse_stylesheet(include_str!("glass_drawer.css"))
            .expect("glass drawer example stylesheet should stay valid")
    })
}

#[cfg(test)]
mod tests {
    use super::{ACTION_TOGGLE_DRAWER, DrawerState, build_ui, stylesheet, update};
    use cssimpler::app::Invalidation;
    use cssimpler::renderer::FrameInfo;
    use std::sync::atomic::Ordering;

    #[test]
    fn glass_drawer_example_stylesheet_parses_and_builds_ui() {
        let _ = stylesheet();
        let tree = build_ui(&DrawerState::default());

        let cssimpler::core::Node::Element(root) = tree else {
            panic!("root should be an element");
        };

        assert!(root.classes.contains(&"drawer-open".to_string()));
        assert_eq!(root.children.len(), 2);
    }

    #[test]
    fn glass_drawer_toggle_collapses_the_drawer() {
        let mut state = DrawerState::default();
        state.actions.store(ACTION_TOGGLE_DRAWER, Ordering::Relaxed);

        let invalidation = update(
            &mut state,
            FrameInfo {
                frame_index: 1,
                delta: std::time::Duration::ZERO,
            },
        );

        assert_eq!(invalidation, Invalidation::Layout);
        assert!(state.collapsed);
    }
}
