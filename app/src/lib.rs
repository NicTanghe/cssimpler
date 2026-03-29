use std::sync::OnceLock;

use anyhow::Result;
use cssimpler_core::{Color, ElementNode, LayoutBox, Node, RenderNode, VisualStyle};
use cssimpler_renderer::{FrameInfo, WindowConfig};
use cssimpler_style::{Declaration, ElementRef, Stylesheet, parse_stylesheet};

#[derive(Debug, Default)]
pub struct AppState {
    pub frame_index: u64,
    pub last_frame_ms: u128,
}

pub fn run() -> Result<()> {
    let config = WindowConfig::new("cssimpler", 960, 540);
    let mut state = AppState::default();

    cssimpler_renderer::run(config, move |frame| {
        update(&mut state, frame);
        render(&state)
    })
    .map_err(Into::into)
}

pub fn update(state: &mut AppState, frame: FrameInfo) {
    state.frame_index = frame.frame_index;
    state.last_frame_ms = frame.delta.as_millis();
}

pub fn render(state: &AppState) -> Vec<RenderNode> {
    let ui = build_ui(state);
    vec![scene_from_ui(&ui, stylesheet())]
}

pub fn build_ui(state: &AppState) -> Node {
    Node::element("div")
        .with_id("app")
        .with_class("root")
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

fn scene_from_ui(node: &Node, stylesheet: &Stylesheet) -> RenderNode {
    match node {
        Node::Element(root) => element_to_render_node(root, stylesheet),
        Node::Text(_) => unreachable!("app root should always be an element"),
    }
}

fn element_to_render_node(element: &ElementNode, stylesheet: &Stylesheet) -> RenderNode {
    let computed = compute_style(element, stylesheet);
    let layout = LayoutBox::new(
        computed.x.unwrap_or(0.0),
        computed.y.unwrap_or(0.0),
        computed.width.unwrap_or(0.0),
        computed.height.unwrap_or(0.0),
    );
    let visual = VisualStyle {
        background: computed.background,
        foreground: computed.foreground.unwrap_or(Color::BLACK),
        ..VisualStyle::default()
    };
    let child_elements: Vec<_> = element
        .children
        .iter()
        .filter_map(|child| match child {
            Node::Element(child) => Some(element_to_render_node(child, stylesheet)),
            Node::Text(_) => None,
        })
        .collect();
    let text = element_text(element);

    if child_elements.is_empty() && !text.is_empty() {
        RenderNode::text(layout, text).with_style(visual)
    } else {
        RenderNode::container(layout)
            .with_style(visual)
            .with_children(child_elements)
    }
}

fn compute_style(element: &ElementNode, stylesheet: &Stylesheet) -> ComputedStyle {
    let mut computed = ComputedStyle::default();
    let element_ref = ElementRef {
        tag: &element.tag,
        id: element.id.as_deref(),
        classes: &element.classes,
    };

    for rule in stylesheet.matching_rules(element_ref) {
        for declaration in &rule.declarations {
            computed.apply(*declaration);
        }
    }

    computed
}

fn element_text(element: &ElementNode) -> String {
    let mut content = String::new();

    for child in &element.children {
        collect_text(child, &mut content);
    }

    content
}

fn collect_text(node: &Node, buffer: &mut String) {
    match node {
        Node::Text(content) => buffer.push_str(content),
        Node::Element(element) => {
            for child in &element.children {
                collect_text(child, buffer);
            }
        }
    }
}

#[derive(Debug, Default)]
struct ComputedStyle {
    background: Option<Color>,
    foreground: Option<Color>,
    x: Option<f32>,
    y: Option<f32>,
    width: Option<f32>,
    height: Option<f32>,
}

impl ComputedStyle {
    fn apply(&mut self, declaration: Declaration) {
        match declaration {
            Declaration::Background(color) => self.background = Some(color),
            Declaration::Foreground(color) => self.foreground = Some(color),
            Declaration::X(value) => self.x = Some(value),
            Declaration::Y(value) => self.y = Some(value),
            Declaration::Width(value) => self.width = Some(value),
            Declaration::Height(value) => self.height = Some(value),
        }
    }
}

#[cfg(test)]
mod tests {
    use cssimpler_core::{Node, RenderKind};

    use super::{AppState, build_ui, render};

    #[test]
    fn demo_ui_has_the_expected_nested_structure() {
        let tree = build_ui(&AppState {
            frame_index: 7,
            last_frame_ms: 0,
        });

        match tree {
            Node::Element(root) => {
                assert_eq!(root.id.as_deref(), Some("app"));
                assert_eq!(root.children.len(), 1);
            }
            Node::Text(_) => panic!("expected root element"),
        }
    }

    #[test]
    fn render_builds_a_background_scene_with_text() {
        let scene = render(&AppState {
            frame_index: 12,
            last_frame_ms: 16,
        });

        assert_eq!(scene.len(), 1);
        assert_eq!(scene[0].children.len(), 1);
        assert_eq!(scene[0].children[0].children.len(), 2);
        assert!(matches!(
            scene[0].children[0].children[0].kind,
            RenderKind::Text(_)
        ));
        assert_eq!(scene[0].layout.width, 960.0);
        assert_eq!(scene[0].children[0].layout.width, 640.0);
    }
}
