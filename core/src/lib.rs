pub mod color;
pub mod custom_properties;
pub mod dom;
pub mod fonts;
pub mod generated_content;
pub mod interaction;
pub mod scrollbar;
pub mod svg;
pub mod transitions;

use crate::fonts::{PreparedTextLayout, TextStyle};
use taffy::Style as TaffyStyle;

pub use color::{Color, GradientInterpolation, LinearRgba};
pub use custom_properties::CustomProperties;
pub use dom::{ElementNode, EventHandler, EventHandlers, IntoNode, Node, into_node};
pub use generated_content::GeneratedTextSource;
pub use interaction::{ElementInteractionState, ElementPath};
pub use scrollbar::{
    OverflowMode, ScrollbarAxisState, ScrollbarData, ScrollbarInteractionState, ScrollbarMetrics,
    ScrollbarStyle, ScrollbarWidth,
};
pub use svg::{
    SvgBounds, SvgContour, SvgPaint, SvgPathGeometry, SvgPathInstance, SvgPathPaint, SvgPoint,
    SvgScene, SvgStyle, SvgViewBox,
};
pub use transitions::{
    TransitionEntry, TransitionPropertyName, TransitionStyle, TransitionTimingFunction,
};

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
    pub interpolation: GradientInterpolation,
    pub repeating: bool,
    pub stops: Vec<GradientStop<LengthPercentageValue>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RadialGradient {
    pub shape: RadialShape,
    pub center: GradientPoint,
    pub interpolation: GradientInterpolation,
    pub repeating: bool,
    pub stops: Vec<GradientStop<LengthPercentageValue>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ConicGradient {
    pub angle: f32,
    pub center: GradientPoint,
    pub interpolation: GradientInterpolation,
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
    pub line_style: BorderLineStyle,
}

impl Default for BorderStyle {
    fn default() -> Self {
        Self {
            color: Color::BLACK,
            widths: Insets::ZERO,
            line_style: BorderLineStyle::Solid,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BorderLineStyle {
    #[default]
    Solid,
    Dashed,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoxShadow {
    pub color: Color,
    pub offset_x: f32,
    pub offset_y: f32,
    pub blur_radius: f32,
    pub spread: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ShadowEffect {
    pub color: Option<Color>,
    pub offset_x: f32,
    pub offset_y: f32,
    pub blur_radius: f32,
    pub spread: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TextStrokeStyle {
    pub width: f32,
    pub color: Option<Color>,
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TransformOrigin {
    pub x: LengthPercentageValue,
    pub y: LengthPercentageValue,
}

impl TransformOrigin {
    pub const CENTER: Self = Self {
        x: LengthPercentageValue::from_fraction(0.5),
        y: LengthPercentageValue::from_fraction(0.5),
    };
}

impl Default for TransformOrigin {
    fn default() -> Self {
        Self::CENTER
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TransformStyleMode {
    #[default]
    Flat,
    Preserve3d,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TransformMatrix3d {
    pub m11: f32,
    pub m12: f32,
    pub m13: f32,
    pub m14: f32,
    pub m21: f32,
    pub m22: f32,
    pub m23: f32,
    pub m24: f32,
    pub m31: f32,
    pub m32: f32,
    pub m33: f32,
    pub m34: f32,
    pub m41: f32,
    pub m42: f32,
    pub m43: f32,
    pub m44: f32,
}

impl TransformMatrix3d {
    pub const IDENTITY: Self = Self {
        m11: 1.0,
        m12: 0.0,
        m13: 0.0,
        m14: 0.0,
        m21: 0.0,
        m22: 1.0,
        m23: 0.0,
        m24: 0.0,
        m31: 0.0,
        m32: 0.0,
        m33: 1.0,
        m34: 0.0,
        m41: 0.0,
        m42: 0.0,
        m43: 0.0,
        m44: 1.0,
    };

    pub const fn translate(x: f32, y: f32, z: f32) -> Self {
        Self {
            m14: x,
            m24: y,
            m34: z,
            ..Self::IDENTITY
        }
    }

    pub const fn scale(x: f32, y: f32, z: f32) -> Self {
        Self {
            m11: x,
            m22: y,
            m33: z,
            ..Self::IDENTITY
        }
    }

    pub fn rotate(x: f32, y: f32, z: f32, degrees: f32) -> Self {
        let length = (x * x + y * y + z * z).sqrt();
        if length <= f32::EPSILON {
            return Self::IDENTITY;
        }

        let x = x / length;
        let y = y / length;
        let z = z / length;
        let radians = degrees.to_radians();
        let sin = radians.sin();
        let cos = radians.cos();
        let t = 1.0 - cos;

        Self {
            m11: t * x * x + cos,
            m12: t * x * y - sin * z,
            m13: t * x * z + sin * y,
            m14: 0.0,
            m21: t * x * y + sin * z,
            m22: t * y * y + cos,
            m23: t * y * z - sin * x,
            m24: 0.0,
            m31: t * x * z - sin * y,
            m32: t * y * z + sin * x,
            m33: t * z * z + cos,
            m34: 0.0,
            m41: 0.0,
            m42: 0.0,
            m43: 0.0,
            m44: 1.0,
        }
    }

    pub fn perspective(depth: f32) -> Option<Self> {
        (depth.abs() > f32::EPSILON).then_some(Self {
            m43: -1.0 / depth,
            ..Self::IDENTITY
        })
    }

    pub fn is_identity(self) -> bool {
        self == Self::IDENTITY
    }

    pub fn is_2d(self) -> bool {
        self.m13 == 0.0
            && self.m23 == 0.0
            && self.m31 == 0.0
            && self.m32 == 0.0
            && self.m33 == 1.0
            && self.m34 == 0.0
            && self.m41 == 0.0
            && self.m42 == 0.0
            && self.m43 == 0.0
            && self.m44 == 1.0
    }

    pub fn multiply(self, other: Self) -> Self {
        Self {
            m11: self.m11 * other.m11
                + self.m12 * other.m21
                + self.m13 * other.m31
                + self.m14 * other.m41,
            m12: self.m11 * other.m12
                + self.m12 * other.m22
                + self.m13 * other.m32
                + self.m14 * other.m42,
            m13: self.m11 * other.m13
                + self.m12 * other.m23
                + self.m13 * other.m33
                + self.m14 * other.m43,
            m14: self.m11 * other.m14
                + self.m12 * other.m24
                + self.m13 * other.m34
                + self.m14 * other.m44,
            m21: self.m21 * other.m11
                + self.m22 * other.m21
                + self.m23 * other.m31
                + self.m24 * other.m41,
            m22: self.m21 * other.m12
                + self.m22 * other.m22
                + self.m23 * other.m32
                + self.m24 * other.m42,
            m23: self.m21 * other.m13
                + self.m22 * other.m23
                + self.m23 * other.m33
                + self.m24 * other.m43,
            m24: self.m21 * other.m14
                + self.m22 * other.m24
                + self.m23 * other.m34
                + self.m24 * other.m44,
            m31: self.m31 * other.m11
                + self.m32 * other.m21
                + self.m33 * other.m31
                + self.m34 * other.m41,
            m32: self.m31 * other.m12
                + self.m32 * other.m22
                + self.m33 * other.m32
                + self.m34 * other.m42,
            m33: self.m31 * other.m13
                + self.m32 * other.m23
                + self.m33 * other.m33
                + self.m34 * other.m43,
            m34: self.m31 * other.m14
                + self.m32 * other.m24
                + self.m33 * other.m34
                + self.m34 * other.m44,
            m41: self.m41 * other.m11
                + self.m42 * other.m21
                + self.m43 * other.m31
                + self.m44 * other.m41,
            m42: self.m41 * other.m12
                + self.m42 * other.m22
                + self.m43 * other.m32
                + self.m44 * other.m42,
            m43: self.m41 * other.m13
                + self.m42 * other.m23
                + self.m43 * other.m33
                + self.m44 * other.m43,
            m44: self.m41 * other.m14
                + self.m42 * other.m24
                + self.m43 * other.m34
                + self.m44 * other.m44,
        }
    }

    pub fn transform_point(self, x: f32, y: f32, z: f32, w: f32) -> (f32, f32, f32, f32) {
        (
            self.m11 * x + self.m12 * y + self.m13 * z + self.m14 * w,
            self.m21 * x + self.m22 * y + self.m23 * z + self.m24 * w,
            self.m31 * x + self.m32 * y + self.m33 * z + self.m34 * w,
            self.m41 * x + self.m42 * y + self.m43 * z + self.m44 * w,
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TransformOperation {
    Translate {
        x: LengthPercentageValue,
        y: LengthPercentageValue,
    },
    TranslateZ {
        z: f32,
    },
    Scale {
        x: f32,
        y: f32,
    },
    Rotate {
        degrees: f32,
    },
    RotateX {
        degrees: f32,
    },
    RotateY {
        degrees: f32,
    },
    RotateZ {
        degrees: f32,
    },
    Matrix3d {
        matrix: TransformMatrix3d,
    },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Transform2D {
    pub origin: TransformOrigin,
    pub operations: Vec<TransformOperation>,
}

impl Transform2D {
    pub fn is_identity(&self) -> bool {
        self.operations.is_empty()
    }

    pub fn uses_depth(&self) -> bool {
        self.operations
            .iter()
            .any(|operation| operation.uses_depth())
    }
}

impl TransformOperation {
    pub fn uses_depth(self) -> bool {
        match self {
            Self::TranslateZ { .. } | Self::RotateX { .. } | Self::RotateY { .. } => true,
            Self::Matrix3d { matrix } => !matrix.is_2d(),
            _ => false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct VisualStyle {
    pub background: Option<Color>,
    pub background_layers: Vec<BackgroundLayer>,
    pub foreground: Color,
    pub svg: SvgStyle,
    pub text: TextStyle,
    pub text_stroke: TextStrokeStyle,
    pub text_shadows: Vec<ShadowEffect>,
    pub filter_drop_shadows: Vec<ShadowEffect>,
    pub backdrop_blur_radius: f32,
    pub corner_radius: CornerRadius,
    pub border: BorderStyle,
    pub shadows: Vec<BoxShadow>,
    pub overflow: Overflow,
    pub transform: Transform2D,
    pub perspective: Option<f32>,
    pub transform_style: TransformStyleMode,
    pub scrollbar: ScrollbarStyle,
}

impl Default for VisualStyle {
    fn default() -> Self {
        Self {
            background: None,
            background_layers: Vec::new(),
            foreground: Color::BLACK,
            svg: SvgStyle::default(),
            text: TextStyle::default(),
            text_stroke: TextStrokeStyle::default(),
            text_shadows: Vec::new(),
            filter_drop_shadows: Vec::new(),
            backdrop_blur_radius: 0.0,
            corner_radius: CornerRadius::ZERO,
            border: BorderStyle::default(),
            shadows: Vec::new(),
            overflow: Overflow::VISIBLE,
            transform: Transform2D::default(),
            perspective: None,
            transform_style: TransformStyleMode::Flat,
            scrollbar: ScrollbarStyle::default(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Style {
    pub custom_properties: CustomProperties,
    pub layout: LayoutStyle,
    pub visual: VisualStyle,
    pub generated_text: Option<GeneratedTextSource>,
    pub transitions: TransitionStyle,
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
    Svg(SvgScene),
}

#[derive(Clone, Debug)]
pub struct RenderNode {
    pub kind: RenderKind,
    pub layout: LayoutBox,
    pub style: VisualStyle,
    pub transitions: TransitionStyle,
    pub text_layout: Option<PreparedTextLayout>,
    pub element_id: Option<String>,
    pub element_path: Option<ElementPath>,
    pub content_inset: Insets,
    pub scrollbars: Option<ScrollbarData>,
    pub handlers: EventHandlers,
    pub children: Vec<RenderNode>,
}

impl RenderNode {
    pub fn container(layout: LayoutBox) -> Self {
        Self {
            kind: RenderKind::Container,
            layout,
            style: VisualStyle::default(),
            transitions: TransitionStyle::default(),
            text_layout: None,
            element_id: None,
            element_path: None,
            content_inset: Insets::ZERO,
            scrollbars: None,
            handlers: EventHandlers::default(),
            children: Vec::new(),
        }
    }

    pub fn text(layout: LayoutBox, content: impl Into<String>) -> Self {
        Self {
            kind: RenderKind::Text(content.into()),
            layout,
            style: VisualStyle::default(),
            transitions: TransitionStyle::default(),
            text_layout: None,
            element_id: None,
            element_path: None,
            content_inset: Insets::ZERO,
            scrollbars: None,
            handlers: EventHandlers::default(),
            children: Vec::new(),
        }
    }

    pub fn svg(layout: LayoutBox, scene: SvgScene) -> Self {
        Self {
            kind: RenderKind::Svg(scene),
            layout,
            style: VisualStyle::default(),
            transitions: TransitionStyle::default(),
            text_layout: None,
            element_id: None,
            element_path: None,
            content_inset: Insets::ZERO,
            scrollbars: None,
            handlers: EventHandlers::default(),
            children: Vec::new(),
        }
    }

    pub fn with_style(mut self, style: VisualStyle) -> Self {
        self.style = style;
        self
    }

    pub fn with_transitions(mut self, transitions: TransitionStyle) -> Self {
        self.transitions = transitions;
        self
    }

    pub fn with_text_layout(mut self, text_layout: impl Into<Option<PreparedTextLayout>>) -> Self {
        self.text_layout = text_layout.into();
        self
    }

    pub fn with_element_id(mut self, element_id: impl Into<String>) -> Self {
        self.element_id = Some(element_id.into());
        self
    }

    pub fn with_element_path(mut self, element_path: ElementPath) -> Self {
        self.element_path = Some(element_path);
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

    pub fn with_handlers(mut self, handlers: EventHandlers) -> Self {
        self.handlers = handlers;
        self
    }

    pub fn on_click(mut self, handler: EventHandler) -> Self {
        self.handlers.click = Some(handler);
        self
    }

    pub fn on_contextmenu(mut self, handler: EventHandler) -> Self {
        self.handlers.contextmenu = Some(handler);
        self
    }

    pub fn on_dblclick(mut self, handler: EventHandler) -> Self {
        self.handlers.dblclick = Some(handler);
        self
    }

    pub fn on_mousedown(mut self, handler: EventHandler) -> Self {
        self.handlers.mousedown = Some(handler);
        self
    }

    pub fn on_mouseenter(mut self, handler: EventHandler) -> Self {
        self.handlers.mouseenter = Some(handler);
        self
    }

    pub fn on_mouseleave(mut self, handler: EventHandler) -> Self {
        self.handlers.mouseleave = Some(handler);
        self
    }

    pub fn on_mousemove(mut self, handler: EventHandler) -> Self {
        self.handlers.mousemove = Some(handler);
        self
    }

    pub fn on_mouseout(mut self, handler: EventHandler) -> Self {
        self.handlers.mouseout = Some(handler);
        self
    }

    pub fn on_mouseover(mut self, handler: EventHandler) -> Self {
        self.handlers.mouseover = Some(handler);
        self
    }

    pub fn on_mouseup(mut self, handler: EventHandler) -> Self {
        self.handlers.mouseup = Some(handler);
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
    use super::{Color, LayoutBox, RenderKind, RenderNode, VisualStyle};

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
