//! Backend-independent vector commands, native elements, and SVG export.
use crate::{
    Color, SvgBounds, SvgPathGeometry, SvgPathInstance, SvgPathPaint, SvgPathPaintSource, SvgPoint,
    SvgScene, SvgViewBox,
};
use std::{
    error::Error,
    fmt::{self, Write},
};

pub type VectorPoint = SvgPoint;
pub type VectorViewBox = SvgViewBox;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PathCommand {
    MoveTo(VectorPoint),
    LineTo(VectorPoint),
    QuadTo {
        control: VectorPoint,
        end: VectorPoint,
    },
    CubicTo {
        control1: VectorPoint,
        control2: VectorPoint,
        end: VectorPoint,
    },
    Close,
}

impl PathCommand {
    pub(crate) fn points(&self) -> impl Iterator<Item = VectorPoint> {
        let points = match *self {
            Self::MoveTo(p) | Self::LineTo(p) => [Some(p), None, None],
            Self::QuadTo { control, end } => [Some(control), Some(end), None],
            Self::CubicTo {
                control1,
                control2,
                end,
            } => [Some(control1), Some(control2), Some(end)],
            Self::Close => [None; 3],
        };
        points.into_iter().flatten()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VectorPaint {
    pub fill: Option<Color>,
    pub stroke: Option<Color>,
    pub stroke_width: f32,
}
impl Default for VectorPaint {
    fn default() -> Self {
        Self {
            fill: Some(Color::BLACK),
            stroke: None,
            stroke_width: 1.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VectorError(pub(crate) &'static str);
impl fmt::Display for VectorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}
impl Error for VectorError {}

#[derive(Clone, Debug, PartialEq)]
pub struct VectorPath {
    commands: Vec<PathCommand>,
    paint: VectorPaint,
}
impl VectorPath {
    pub fn new(commands: Vec<PathCommand>, paint: VectorPaint) -> Result<Self, VectorError> {
        if !paint.stroke_width.is_finite() || paint.stroke_width < 0.0 {
            return Err(VectorError("stroke width must be finite and nonnegative"));
        }
        let mut started = false;
        for command in &commands {
            if command
                .points()
                .any(|p| !p.x.is_finite() || !p.y.is_finite())
            {
                return Err(VectorError("path coordinates must be finite"));
            }
            if matches!(command, PathCommand::MoveTo(_)) {
                started = true;
            } else if !started {
                return Err(VectorError("a contour must start with MoveTo"));
            }
        }
        Ok(Self { commands, paint })
    }
    pub fn commands(&self) -> &[PathCommand] {
        &self.commands
    }
    pub fn paint(&self) -> VectorPaint {
        self.paint
    }
    /// Serialize authored commands without flattening curves or rounding coordinates.
    pub fn to_svg_path_data(&self) -> String {
        let mut d = String::new();
        for c in &self.commands {
            if !d.is_empty() {
                d.push(' ');
            }
            match c {
                PathCommand::MoveTo(p) => write!(d, "M {} {}", p.x, p.y),
                PathCommand::LineTo(p) => write!(d, "L {} {}", p.x, p.y),
                PathCommand::QuadTo { control: c, end: p } => {
                    write!(d, "Q {} {} {} {}", c.x, c.y, p.x, p.y)
                }
                PathCommand::CubicTo {
                    control1: a,
                    control2: b,
                    end: p,
                } => write!(d, "C {} {} {} {} {} {}", a.x, a.y, b.x, b.y, p.x, p.y),
                PathCommand::Close => write!(d, "Z"),
            }
            .expect("writing to a String cannot fail");
        }
        d
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct VectorScene {
    view_box: VectorViewBox,
    paths: Vec<VectorPath>,
}
impl VectorScene {
    pub fn new(view_box: VectorViewBox, paths: Vec<VectorPath>) -> Result<Self, VectorError> {
        if [
            view_box.min_x,
            view_box.min_y,
            view_box.width,
            view_box.height,
        ]
        .iter()
        .any(|v| !v.is_finite())
            || view_box.width <= 0.0
            || view_box.height <= 0.0
        {
            return Err(VectorError(
                "viewBox must be finite with positive dimensions",
            ));
        }
        Ok(Self { view_box, paths })
    }
    pub fn view_box(&self) -> VectorViewBox {
        self.view_box
    }
    pub fn paths(&self) -> &[VectorPath] {
        &self.paths
    }
    /// Export a standalone SVG using numeric user coordinates (default SVG units are px).
    /// The current paint subset uses nonzero fills and round stroke caps/joins.
    pub fn to_svg(&self) -> String {
        fn paint(out: &mut String, name: &str, color: Option<Color>) {
            match color {
                None => write!(out, " {name}=\"none\""),
                Some(c) => write!(
                    out,
                    " {name}=\"#{:02x}{:02x}{:02x}\" {name}-opacity=\"{}\"",
                    c.r,
                    c.g,
                    c.b,
                    f32::from(c.a) / 255.0
                ),
            }
            .unwrap();
        }
        let v = self.view_box;
        let mut out = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"{} {} {} {}\">\n",
            v.width, v.height, v.min_x, v.min_y, v.width, v.height
        );
        for path in &self.paths {
            write!(out, "  <path d=\"{}\"", path.to_svg_path_data()).unwrap();
            paint(&mut out, "fill", path.paint.fill);
            paint(&mut out, "stroke", path.paint.stroke);
            writeln!(out," stroke-width=\"{}\" fill-rule=\"nonzero\" stroke-linecap=\"round\" stroke-linejoin=\"round\"/>",path.paint.stroke_width).unwrap();
        }
        out.push_str("</svg>\n");
        out
    }
    /// Bridge into the shared renderer pipeline; no SVG strings are produced or parsed.
    pub fn into_render_scene(self) -> SvgScene {
        SvgScene::new(
            self.view_box,
            self.paths
                .into_iter()
                .map(|p| {
                    let bounds = p
                        .commands
                        .iter()
                        .flat_map(PathCommand::points)
                        .map(|p| SvgBounds::new(p.x, p.y, p.x, p.y))
                        .reduce(SvgBounds::union);
                    SvgPathInstance {
                        geometry: SvgPathGeometry {
                            contours: vec![],
                            commands: p.commands,
                            bounds,
                        },
                        paint: SvgPathPaint {
                            fill: p.paint.fill.map(SvgPathPaintSource::Color),
                            stroke: p.paint.stroke.map(SvgPathPaintSource::Color),
                            stroke_width: p.paint.stroke_width,
                        },
                    }
                })
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn export_retains_controls_precision_and_paint() {
        let commands = vec![
            PathCommand::MoveTo(VectorPoint::new(0.12345679, 2.)),
            PathCommand::CubicTo {
                control1: VectorPoint::new(-10., 100.),
                control2: VectorPoint::new(80., -40.),
                end: VectorPoint::new(90., 10.),
            },
            PathCommand::QuadTo {
                control: VectorPoint::new(95., 20.),
                end: VectorPoint::new(99., 30.),
            },
            PathCommand::Close,
        ];
        let path = VectorPath::new(
            commands.clone(),
            VectorPaint {
                fill: None,
                stroke: Some(Color::rgba(1, 2, 3, 128)),
                stroke_width: 2.,
            },
        )
        .unwrap();
        assert_eq!(
            path.to_svg_path_data(),
            "M 0.12345679 2 C -10 100 80 -40 90 10 Q 95 20 99 30 Z"
        );
        let scene = VectorScene::new(VectorViewBox::new(0., 0., 100., 100.), vec![path]).unwrap();
        let svg = scene.to_svg();
        assert!(svg.contains("fill=\"none\" stroke=\"#010203\" stroke-opacity=\"0.5019608\""));
        let render = scene.into_render_scene();
        assert_eq!(render.paths[0].geometry.commands, commands);
        let bounds = render.bounds.unwrap();
        assert!(bounds.min_x <= -10. && bounds.max_y >= 100.);
    }
    #[test]
    fn invalid_geometry_and_metrics_are_rejected() {
        for commands in [
            vec![PathCommand::Close],
            vec![PathCommand::LineTo(VectorPoint::new(0., 0.))],
            vec![PathCommand::MoveTo(VectorPoint::new(f32::NAN, 0.))],
        ] {
            assert!(VectorPath::new(commands, VectorPaint::default()).is_err());
        }
        assert!(
            VectorPath::new(
                vec![],
                VectorPaint {
                    stroke_width: f32::INFINITY,
                    ..VectorPaint::default()
                }
            )
            .is_err()
        );
        assert!(VectorScene::new(VectorViewBox::new(0., 0., 0., 10.), vec![]).is_err());
    }
}
