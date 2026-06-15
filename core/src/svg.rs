use crate::{Color, LengthPercentageValue};

#[derive(Clone, Debug, PartialEq)]
pub enum SvgPaint {
    Color(Color),
    CurrentColor,
    PaintServer(String),
    None,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SvgStyle {
    pub fill: Option<SvgPaint>,
    pub stroke: Option<SvgPaint>,
    pub stroke_width: Option<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SvgViewBox {
    pub min_x: f32,
    pub min_y: f32,
    pub width: f32,
    pub height: f32,
}

impl SvgViewBox {
    pub const fn new(min_x: f32, min_y: f32, width: f32, height: f32) -> Self {
        Self {
            min_x,
            min_y,
            width,
            height,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SvgPoint {
    pub x: f32,
    pub y: f32,
}

impl SvgPoint {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SvgBounds {
    pub min_x: f32,
    pub min_y: f32,
    pub max_x: f32,
    pub max_y: f32,
}

impl SvgBounds {
    pub const fn new(min_x: f32, min_y: f32, max_x: f32, max_y: f32) -> Self {
        Self {
            min_x,
            min_y,
            max_x,
            max_y,
        }
    }

    pub fn union(self, other: Self) -> Self {
        Self {
            min_x: self.min_x.min(other.min_x),
            min_y: self.min_y.min(other.min_y),
            max_x: self.max_x.max(other.max_x),
            max_y: self.max_y.max(other.max_y),
        }
    }

    pub fn expand(self, amount: f32) -> Self {
        Self {
            min_x: self.min_x - amount,
            min_y: self.min_y - amount,
            max_x: self.max_x + amount,
            max_y: self.max_y + amount,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SvgContour {
    pub points: Vec<SvgPoint>,
    pub closed: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SvgPathGeometry {
    pub contours: Vec<SvgContour>,
    pub bounds: Option<SvgBounds>,
}

impl SvgPathGeometry {
    pub fn new(contours: Vec<SvgContour>) -> Self {
        Self {
            bounds: svg_geometry_bounds(&contours),
            contours,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum SvgPathPaintSource {
    Color(Color),
    PaintServer(SvgPaintServerReference),
}

#[derive(Clone, Debug, PartialEq)]
pub struct SvgPaintServerReference {
    pub id: String,
    pub kind: SvgPaintServerKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SvgPaintServerKind {
    LinearGradient,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SvgPaintServer {
    pub id: String,
    pub kind: SvgPaintServerKind,
    pub data: SvgPaintServerData,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SvgPaintServerData {
    LinearGradient(SvgLinearGradient),
}

#[derive(Clone, Debug, PartialEq)]
pub struct SvgLinearGradient {
    pub x1: LengthPercentageValue,
    pub y1: LengthPercentageValue,
    pub x2: LengthPercentageValue,
    pub y2: LengthPercentageValue,
    pub stops: Vec<SvgGradientStop>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SvgGradientStop {
    pub color: Color,
    pub offset: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SvgPathPaint {
    pub fill: Option<SvgPathPaintSource>,
    pub stroke: Option<SvgPathPaintSource>,
    pub stroke_width: f32,
}

impl Default for SvgPathPaint {
    fn default() -> Self {
        Self {
            fill: Some(SvgPathPaintSource::Color(Color::BLACK)),
            stroke: None,
            stroke_width: 1.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SvgPathInstance {
    pub geometry: SvgPathGeometry,
    pub paint: SvgPathPaint,
}

impl SvgPathInstance {
    pub fn bounds(&self) -> Option<SvgBounds> {
        let stroke_padding =
            if self.paint.stroke.is_some() && self.paint.stroke_width > f32::EPSILON {
                self.paint.stroke_width * 0.5
            } else {
                0.0
            };
        self.geometry
            .bounds
            .map(|bounds| bounds.expand(stroke_padding))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SvgScene {
    pub view_box: SvgViewBox,
    pub paint_servers: Vec<SvgPaintServer>,
    pub paths: Vec<SvgPathInstance>,
    pub bounds: Option<SvgBounds>,
}

impl SvgScene {
    pub fn new(view_box: SvgViewBox, paths: Vec<SvgPathInstance>) -> Self {
        Self::with_paint_servers(view_box, Vec::new(), paths)
    }

    pub fn with_paint_servers(
        view_box: SvgViewBox,
        paint_servers: Vec<SvgPaintServer>,
        paths: Vec<SvgPathInstance>,
    ) -> Self {
        let bounds = paths
            .iter()
            .filter_map(SvgPathInstance::bounds)
            .reduce(SvgBounds::union);
        Self {
            view_box,
            paint_servers,
            paths,
            bounds,
        }
    }
}

fn svg_geometry_bounds(contours: &[SvgContour]) -> Option<SvgBounds> {
    let mut bounds: Option<SvgBounds> = None;
    for point in contours.iter().flat_map(|contour| contour.points.iter()) {
        bounds = Some(match bounds {
            Some(existing) => SvgBounds {
                min_x: existing.min_x.min(point.x),
                min_y: existing.min_y.min(point.y),
                max_x: existing.max_x.max(point.x),
                max_y: existing.max_y.max(point.y),
            },
            None => SvgBounds::new(point.x, point.y, point.x, point.y),
        });
    }
    bounds
}
