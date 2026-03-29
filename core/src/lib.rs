use taffy::Style as TaffyStyle;

pub type EventHandler = fn();

#[derive(Clone, Debug)]
pub enum Node {
    Element(ElementNode),
    Text(String),
}

impl Node {
    pub fn element(tag: impl Into<String>) -> ElementNode {
        ElementNode::new(tag)
    }

    pub fn text(text: impl Into<String>) -> Self {
        Self::Text(text.into())
    }
}

#[derive(Clone, Debug)]
pub struct ElementNode {
    pub tag: String,
    pub id: Option<String>,
    pub classes: Vec<String>,
    pub style: Style,
    pub children: Vec<Node>,
    pub on_click: Option<EventHandler>,
}

impl ElementNode {
    pub fn new(tag: impl Into<String>) -> Self {
        Self {
            tag: tag.into(),
            id: None,
            classes: Vec::new(),
            style: Style::default(),
            children: Vec::new(),
            on_click: None,
        }
    }

    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    pub fn with_class(mut self, class_name: impl Into<String>) -> Self {
        self.classes.push(class_name.into());
        self
    }

    pub fn with_style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }

    pub fn with_child(mut self, child: Node) -> Self {
        self.children.push(child);
        self
    }

    pub fn with_children(mut self, children: impl IntoIterator<Item = Node>) -> Self {
        self.children.extend(children);
        self
    }

    pub fn on_click(mut self, handler: EventHandler) -> Self {
        self.on_click = Some(handler);
        self
    }
}

impl From<ElementNode> for Node {
    fn from(value: ElementNode) -> Self {
        Self::Element(value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const WHITE: Self = Self::rgb(255, 255, 255);
    pub const BLACK: Self = Self::rgb(0, 0, 0);

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Insets {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

impl Insets {
    pub const ZERO: Self = Self::all(0.0);

    pub const fn all(value: f32) -> Self {
        Self {
            top: value,
            right: value,
            bottom: value,
            left: value,
        }
    }
}

#[derive(Clone, Debug)]
pub struct LayoutStyle {
    pub taffy: TaffyStyle,
    pub margin: Insets,
    pub padding: Insets,
}

impl Default for LayoutStyle {
    fn default() -> Self {
        Self {
            taffy: TaffyStyle::default(),
            margin: Insets::ZERO,
            padding: Insets::ZERO,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CornerRadius {
    pub top_left: f32,
    pub top_right: f32,
    pub bottom_right: f32,
    pub bottom_left: f32,
}

impl CornerRadius {
    pub const ZERO: Self = Self::all(0.0);

    pub const fn all(value: f32) -> Self {
        Self {
            top_left: value,
            top_right: value,
            bottom_right: value,
            bottom_left: value,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VisualStyle {
    pub background: Option<Color>,
    pub foreground: Color,
    pub corner_radius: CornerRadius,
}

impl Default for VisualStyle {
    fn default() -> Self {
        Self {
            background: None,
            foreground: Color::BLACK,
            corner_radius: CornerRadius::ZERO,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Style {
    pub layout: LayoutStyle,
    pub visual: VisualStyle,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LayoutBox {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl LayoutBox {
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum RenderKind {
    Container,
    Text(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct RenderNode {
    pub kind: RenderKind,
    pub layout: LayoutBox,
    pub style: VisualStyle,
    pub children: Vec<RenderNode>,
}

impl RenderNode {
    pub fn container(layout: LayoutBox) -> Self {
        Self {
            kind: RenderKind::Container,
            layout,
            style: VisualStyle::default(),
            children: Vec::new(),
        }
    }

    pub fn text(layout: LayoutBox, content: impl Into<String>) -> Self {
        Self {
            kind: RenderKind::Text(content.into()),
            layout,
            style: VisualStyle::default(),
            children: Vec::new(),
        }
    }

    pub fn with_style(mut self, style: VisualStyle) -> Self {
        self.style = style;
        self
    }

    pub fn with_child(mut self, child: RenderNode) -> Self {
        self.children.push(child);
        self
    }

    pub fn with_children(mut self, children: impl IntoIterator<Item = RenderNode>) -> Self {
        self.children.extend(children);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::{Color, LayoutBox, Node, RenderKind, RenderNode, VisualStyle};

    #[test]
    fn nested_dom_trees_are_supported() {
        let tree = Node::element("div")
            .with_class("card")
            .with_child(Node::element("p").with_child(Node::text("hello")).into())
            .into();

        match tree {
            Node::Element(element) => {
                assert_eq!(element.tag, "div");
                assert_eq!(element.classes, vec!["card".to_string()]);
                assert_eq!(element.children.len(), 1);
            }
            Node::Text(_) => panic!("expected an element node"),
        }
    }

    #[test]
    fn render_nodes_stay_renderer_facing() {
        let scene = RenderNode::container(LayoutBox::new(0.0, 0.0, 100.0, 80.0))
            .with_style(VisualStyle {
                background: Some(Color::rgb(240, 240, 240)),
                ..VisualStyle::default()
            })
            .with_child(RenderNode::text(
                LayoutBox::new(8.0, 8.0, 84.0, 20.0),
                "cssimpler",
            ));

        assert!(matches!(scene.kind, RenderKind::Container));
        assert_eq!(scene.children.len(), 1);
    }
}
