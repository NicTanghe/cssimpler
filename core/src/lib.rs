pub mod fonts;
pub mod scrollbar;

use crate::fonts::TextStyle;
use taffy::Style as TaffyStyle;

pub use scrollbar::{
    OverflowMode, ScrollbarAxisState, ScrollbarData, ScrollbarInteractionState, ScrollbarMetrics,
    ScrollbarStyle, ScrollbarWidth,
};

pub type EventHandler = fn();

pub trait IntoNode {
    fn into_node(self) -> Node;
}

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

pub fn into_node(value: impl IntoNode) -> Node {
    value.into_node()
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

impl IntoNode for Node {
    fn into_node(self) -> Node {
        self
    }
}

impl IntoNode for ElementNode {
    fn into_node(self) -> Node {
        self.into()
    }
}

impl IntoNode for String {
    fn into_node(self) -> Node {
        Node::Text(self)
    }
}

impl IntoNode for &str {
    fn into_node(self) -> Node {
        Node::Text(self.to_string())
    }
}

impl IntoNode for &String {
    fn into_node(self) -> Node {
        Node::Text(self.clone())
    }
}

impl IntoNode for char {
    fn into_node(self) -> Node {
        Node::Text(self.to_string())
    }
}

impl IntoNode for bool {
    fn into_node(self) -> Node {
        Node::Text(self.to_string())
    }
}

macro_rules! impl_into_node_via_to_string {
    ($($ty:ty),* $(,)?) => {
        $(
            impl IntoNode for $ty {
                fn into_node(self) -> Node {
                    Node::Text(self.to_string())
                }
            }
        )*
    };
}

impl_into_node_via_to_string!(i8, i16, i32, i64, i128, isize);
impl_into_node_via_to_string!(u8, u16, u32, u64, u128, usize);
impl_into_node_via_to_string!(f32, f64);

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

    pub const fn with_alpha(self, a: u8) -> Self {
        Self { a, ..self }
    }

    pub fn to_linear_rgba(self) -> LinearRgba {
        LinearRgba {
            r: srgb_channel_to_linear(self.r),
            g: srgb_channel_to_linear(self.g),
            b: srgb_channel_to_linear(self.b),
            a: self.a as f32 / 255.0,
        }
    }

    pub fn from_linear_rgba(color: LinearRgba) -> Self {
        Self {
            r: linear_channel_to_srgb(color.r),
            g: linear_channel_to_srgb(color.g),
            b: linear_channel_to_srgb(color.b),
            a: (color.a.clamp(0.0, 1.0) * 255.0).round() as u8,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LinearRgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl LinearRgba {
    pub const TRANSPARENT: Self = Self {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 0.0,
    };

    pub fn lerp(self, other: Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        Self {
            r: self.r + (other.r - self.r) * t,
            g: self.g + (other.g - self.g) * t,
            b: self.b + (other.b - self.b) * t,
            a: self.a + (other.a - self.a) * t,
        }
    }
}

fn srgb_channel_to_linear(channel: u8) -> f32 {
    let value = channel as f32 / 255.0;
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_channel_to_srgb(value: f32) -> u8 {
    let value = value.clamp(0.0, 1.0);
    let srgb = if value <= 0.003_130_8 {
        value * 12.92
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    };
    (srgb * 255.0).round() as u8
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LengthPercentageValue {
    pub px: f32,
    pub fraction: f32,
}

impl LengthPercentageValue {
    pub const ZERO: Self = Self {
        px: 0.0,
        fraction: 0.0,
    };

    pub const fn from_px(px: f32) -> Self {
        Self { px, fraction: 0.0 }
    }

    pub const fn from_fraction(fraction: f32) -> Self {
        Self { px: 0.0, fraction }
    }

    pub fn resolve(self, total: f32) -> f32 {
        self.px + self.fraction * total
    }

    pub fn lerp(self, other: Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        Self {
            px: self.px + (other.px - self.px) * t,
            fraction: self.fraction + (other.fraction - self.fraction) * t,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AnglePercentageValue {
    pub degrees: f32,
    pub turns: f32,
}

impl AnglePercentageValue {
    pub const ZERO: Self = Self {
        degrees: 0.0,
        turns: 0.0,
    };

    pub const fn from_degrees(degrees: f32) -> Self {
        Self {
            degrees,
            turns: 0.0,
        }
    }

    pub const fn from_turns(turns: f32) -> Self {
        Self {
            degrees: 0.0,
            turns,
        }
    }

    pub fn resolve_degrees(self) -> f32 {
        self.degrees + self.turns * 360.0
    }

    pub fn lerp(self, other: Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        Self {
            degrees: self.degrees + (other.degrees - self.degrees) * t,
            turns: self.turns + (other.turns - self.turns) * t,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GradientPoint {
    pub x: LengthPercentageValue,
    pub y: LengthPercentageValue,
}

impl GradientPoint {
    pub const CENTER: Self = Self {
        x: LengthPercentageValue::from_fraction(0.5),
        y: LengthPercentageValue::from_fraction(0.5),
    };
}

impl Default for GradientPoint {
    fn default() -> Self {
        Self::CENTER
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GradientHorizontal {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GradientVertical {
    Top,
    Bottom,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GradientDirection {
    Angle(f32),
    Horizontal(GradientHorizontal),
    Vertical(GradientVertical),
    Corner {
        horizontal: GradientHorizontal,
        vertical: GradientVertical,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GradientStop<P> {
    pub color: Color,
    pub position: P,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShapeExtent {
    ClosestSide,
    FarthestSide,
    ClosestCorner,
    FarthestCorner,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CircleRadius {
    Explicit(f32),
    Extent(ShapeExtent),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EllipseRadius {
    Explicit {
        x: LengthPercentageValue,
        y: LengthPercentageValue,
    },
    Extent(ShapeExtent),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RadialShape {
    Circle(CircleRadius),
    Ellipse(EllipseRadius),
}

#[derive(Clone, Debug, PartialEq)]
pub struct LinearGradient {
    pub direction: GradientDirection,
    pub repeating: bool,
    pub stops: Vec<GradientStop<LengthPercentageValue>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RadialGradient {
    pub shape: RadialShape,
    pub center: GradientPoint,
    pub repeating: bool,
    pub stops: Vec<GradientStop<LengthPercentageValue>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ConicGradient {
    pub angle: f32,
    pub center: GradientPoint,
    pub repeating: bool,
    pub stops: Vec<GradientStop<AnglePercentageValue>>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum BackgroundLayer {
    LinearGradient(LinearGradient),
    RadialGradient(RadialGradient),
    ConicGradient(ConicGradient),
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

    pub fn is_zero(self) -> bool {
        self.top == 0.0 && self.right == 0.0 && self.bottom == 0.0 && self.left == 0.0
    }
}

#[derive(Clone, Debug)]
pub struct LayoutStyle {
    pub taffy: TaffyStyle,
}

impl Default for LayoutStyle {
    fn default() -> Self {
        let mut taffy = TaffyStyle::default();
        taffy.display = taffy::Display::Block;
        Self { taffy }
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
pub struct BorderStyle {
    pub color: Color,
    pub widths: Insets,
}

impl Default for BorderStyle {
    fn default() -> Self {
        Self {
            color: Color::BLACK,
            widths: Insets::ZERO,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoxShadow {
    pub color: Color,
    pub offset_x: f32,
    pub offset_y: f32,
    pub blur_radius: f32,
    pub spread: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Overflow {
    pub x: OverflowMode,
    pub y: OverflowMode,
}

impl Overflow {
    pub const VISIBLE: Self = Self {
        x: OverflowMode::Visible,
        y: OverflowMode::Visible,
    };

    pub const CLIP: Self = Self {
        x: OverflowMode::Clip,
        y: OverflowMode::Clip,
    };

    pub fn clips_any_axis(self) -> bool {
        self.x.clips() || self.y.clips()
    }

    pub fn allows_scrolling(self) -> bool {
        self.x.allows_scrolling() || self.y.allows_scrolling()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct VisualStyle {
    pub background: Option<Color>,
    pub background_layers: Vec<BackgroundLayer>,
    pub foreground: Color,
    pub text: TextStyle,
    pub corner_radius: CornerRadius,
    pub border: BorderStyle,
    pub shadows: Vec<BoxShadow>,
    pub overflow: Overflow,
    pub scrollbar: ScrollbarStyle,
}

impl Default for VisualStyle {
    fn default() -> Self {
        Self {
            background: None,
            background_layers: Vec::new(),
            foreground: Color::BLACK,
            text: TextStyle::default(),
            corner_radius: CornerRadius::ZERO,
            border: BorderStyle::default(),
            shadows: Vec::new(),
            overflow: Overflow::VISIBLE,
            scrollbar: ScrollbarStyle::default(),
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

#[derive(Clone, Debug)]
pub struct RenderNode {
    pub kind: RenderKind,
    pub layout: LayoutBox,
    pub style: VisualStyle,
    pub content_inset: Insets,
    pub scrollbars: Option<ScrollbarData>,
    pub on_click: Option<EventHandler>,
    pub children: Vec<RenderNode>,
}

impl RenderNode {
    pub fn container(layout: LayoutBox) -> Self {
        Self {
            kind: RenderKind::Container,
            layout,
            style: VisualStyle::default(),
            content_inset: Insets::ZERO,
            scrollbars: None,
            on_click: None,
            children: Vec::new(),
        }
    }

    pub fn text(layout: LayoutBox, content: impl Into<String>) -> Self {
        Self {
            kind: RenderKind::Text(content.into()),
            layout,
            style: VisualStyle::default(),
            content_inset: Insets::ZERO,
            scrollbars: None,
            on_click: None,
            children: Vec::new(),
        }
    }

    pub fn with_style(mut self, style: VisualStyle) -> Self {
        self.style = style;
        self
    }

    pub fn with_content_inset(mut self, content_inset: Insets) -> Self {
        self.content_inset = content_inset;
        self
    }

    pub fn with_scrollbars(mut self, scrollbars: ScrollbarData) -> Self {
        self.scrollbars = Some(scrollbars);
        self
    }

    pub fn on_click(mut self, handler: EventHandler) -> Self {
        self.on_click = Some(handler);
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
