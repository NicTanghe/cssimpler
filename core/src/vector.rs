//! Backend-independent vector commands, native elements, and SVG export.
use crate::{
    Color, SvgBounds, SvgPathGeometry, SvgPathInstance, SvgPathPaint, SvgPathPaintSource, SvgPoint,
    SvgScene, SvgViewBox,
};
use std::{
    error::Error,
    fmt::{self, Write},
    ops::Deref,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
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

/// Immutable authored commands and their conservative control-point bounds.
/// Clones share storage. Construct a replacement to edit geometry.
#[derive(Clone)]
pub struct VectorGeometry(Arc<GeometryData>);
struct GeometryData {
    commands: Vec<PathCommand>,
    bounds: Option<SvgBounds>,
    identity: u64,
}
static NEXT_GEOMETRY_ID: AtomicU64 = AtomicU64::new(1);
impl VectorGeometry {
    pub fn new(commands: Vec<PathCommand>) -> Result<Self, VectorError> {
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
        let bounds = commands
            .iter()
            .flat_map(PathCommand::points)
            .map(|p| SvgBounds::new(p.x, p.y, p.x, p.y))
            .reduce(SvgBounds::union);
        #[allow(deprecated)] // `try_update` is not available on older supported toolchains.
        let identity = NEXT_GEOMETRY_ID
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .expect("vector geometry identity exhausted");
        Ok(Self(Arc::new(GeometryData {
            commands,
            bounds,
            identity,
        })))
    }
    pub fn commands(&self) -> &[PathCommand] {
        &self.0.commands
    }
    pub fn bounds(&self) -> Option<SvgBounds> {
        self.0.bounds
    }
    /// Process-local identity, stable for the lifetime of this geometry and its clones.
    pub fn identity(&self) -> u64 {
        self.0.identity
    }
    pub fn shares_storage_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Default for VectorGeometry {
    fn default() -> Self {
        static EMPTY: OnceLock<VectorGeometry> = OnceLock::new();
        EMPTY.get_or_init(|| Self::new(Vec::new()).unwrap()).clone()
    }
}
impl Deref for VectorGeometry {
    type Target = [PathCommand];
    fn deref(&self) -> &Self::Target {
        self.commands()
    }
}
impl<'a> IntoIterator for &'a VectorGeometry {
    type Item = &'a PathCommand;
    type IntoIter = std::slice::Iter<'a, PathCommand>;
    fn into_iter(self) -> Self::IntoIter {
        self.commands().iter()
    }
}
impl PartialEq for VectorGeometry {
    fn eq(&self, other: &Self) -> bool {
        self.shares_storage_with(other) || self.commands() == other.commands()
    }
}
impl fmt::Debug for VectorGeometry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VectorGeometry")
            .field("identity", &self.identity())
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct VectorPath {
    geometry: VectorGeometry,
    paint: VectorPaint,
}
impl VectorPath {
    pub fn new(commands: Vec<PathCommand>, paint: VectorPaint) -> Result<Self, VectorError> {
        Self::from_geometry(VectorGeometry::new(commands)?, paint)
    }
    pub fn from_geometry(
        geometry: VectorGeometry,
        paint: VectorPaint,
    ) -> Result<Self, VectorError> {
        if !paint.stroke_width.is_finite() || paint.stroke_width < 0.0 {
            return Err(VectorError("stroke width must be finite and nonnegative"));
        }
        Ok(Self { geometry, paint })
    }
    pub fn geometry(&self) -> &VectorGeometry {
        &self.geometry
    }
    pub fn commands(&self) -> &[PathCommand] {
        self.geometry.commands()
    }
    pub fn paint(&self) -> VectorPaint {
        self.paint
    }
    /// Serialize authored commands without flattening curves or rounding coordinates.
    pub fn to_svg_path_data(&self) -> String {
        let mut d = String::new();
        for c in &self.geometry {
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
                .map(|p| SvgPathInstance {
                    geometry: SvgPathGeometry {
                        contours: vec![],
                        bounds: p.geometry.bounds(),
                        commands: p.geometry,
                    },
                    paint: SvgPathPaint {
                        fill: p.paint.fill.map(SvgPathPaintSource::Color),
                        stroke: p.paint.stroke.map(SvgPathPaintSource::Color),
                        stroke_width: p.paint.stroke_width,
                    },
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
        assert_eq!(render.paths[0].geometry.commands.commands(), commands);
        let bounds = render.bounds.unwrap();
        assert!(bounds.min_x <= -10. && bounds.max_y >= 100.);
    }
    #[test]
    fn legacy_svg_empty_commands_are_canonical() {
        let a = SvgPathGeometry::new(vec![]);
        let b = SvgPathGeometry::new(vec![]);
        assert!(a.commands.shares_storage_with(&b.commands));
        assert_eq!(format!("{a:?}"), format!("{b:?}"));
    }
    #[test]
    fn shared_instances_keep_paint_and_release_replaced_geometry() {
        let geometry = VectorGeometry::new(vec![
            PathCommand::MoveTo(VectorPoint::new(1., 2.)),
            PathCommand::QuadTo {
                control: VectorPoint::new(5., 9.),
                end: VectorPoint::new(3., 4.),
            },
        ])
        .unwrap();
        let weak = Arc::downgrade(&geometry.0);
        let first = VectorPath::from_geometry(geometry.clone(), VectorPaint::default()).unwrap();
        let second = VectorPath::from_geometry(
            geometry.clone(),
            VectorPaint {
                fill: None,
                stroke: Some(Color::BLACK),
                stroke_width: 4.,
            },
        )
        .unwrap();
        let scene = VectorScene::new(
            VectorViewBox::new(0., 0., 10., 10.),
            vec![first.clone(), second.clone()],
        )
        .unwrap();
        let svg = scene.to_svg();
        assert!(svg.contains("Q 5 9 3 4"));
        assert!(svg.contains("fill=\"none\""));
        let render = scene.into_render_scene();
        assert!(
            render.paths[0]
                .geometry
                .commands
                .shares_storage_with(&geometry)
        );
        assert!(
            render.paths[1]
                .geometry
                .commands
                .shares_storage_with(&geometry)
        );
        assert_ne!(render.paths[0].paint, render.paths[1].paint);
        assert_ne!(render.paths[0].bounds(), render.paths[1].bounds());
        let replacement =
            VectorGeometry::new(vec![PathCommand::MoveTo(VectorPoint::new(8., 9.))]).unwrap();
        assert_ne!(geometry.identity(), replacement.identity());
        assert_ne!(geometry.bounds(), replacement.bounds());
        drop((geometry, first, second, render));
        assert!(weak.upgrade().is_none());
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

/// Uniform document-to-viewport transform for a native vector drawing.
///
/// Coordinates are logical pixels relative to the vector element's content box;
/// CSS transforms and device scaling are applied by the renderer afterwards.
/// Keep the authored scene separately for export. Apply [`Self::view_box`] to
/// its render scene, and clip the element with an overflow-hidden parent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VectorView {
    scale: f32,
    translation: VectorPoint,
    min_scale: f32,
    max_scale: f32,
}

impl VectorView {
    /// Create an identity view with inclusive, positive finite zoom limits.
    pub fn new(min_scale: f32, max_scale: f32) -> Result<Self, VectorError> {
        if !min_scale.is_finite()
            || !max_scale.is_finite()
            || min_scale <= 0.0
            || min_scale > 1.0
            || max_scale < 1.0
            || max_scale < min_scale
        {
            return Err(VectorError(
                "zoom limits must be finite, positive, and contain 1",
            ));
        }
        Ok(Self {
            scale: 1.0,
            translation: VectorPoint::new(0.0, 0.0),
            min_scale,
            max_scale,
        })
    }

    pub fn scale(&self) -> f32 {
        self.scale
    }
    pub fn translation(&self) -> VectorPoint {
        self.translation
    }

    /// Map document coordinates to element-local logical pixels.
    pub fn to_viewport(&self, point: VectorPoint) -> Option<VectorPoint> {
        finite_point(VectorPoint::new(
            point.x * self.scale + self.translation.x,
            point.y * self.scale + self.translation.y,
        ))
    }

    /// Map element-local logical pixels back to document coordinates.
    pub fn to_document(&self, point: VectorPoint) -> Option<VectorPoint> {
        finite_point(VectorPoint::new(
            (point.x - self.translation.x) / self.scale,
            (point.y - self.translation.y) / self.scale,
        ))
    }

    /// Translate in element-local logical pixels. Invalid or overflowing input is rejected.
    pub fn pan(&mut self, delta: VectorPoint) -> Result<(), VectorError> {
        let translation = finite_point(VectorPoint::new(
            self.translation.x + delta.x,
            self.translation.y + delta.y,
        ))
        .ok_or(VectorError("pan must remain finite"))?;
        self.translation = translation;
        Ok(())
    }

    /// Multiply zoom, clamped to the configured limits, keeping `anchor` fixed.
    /// Invalid factors, anchors, and overflowing results leave the view unchanged.
    pub fn zoom_at(&mut self, factor: f32, anchor: VectorPoint) -> Result<(), VectorError> {
        if !factor.is_finite() || factor <= 0.0 {
            return Err(VectorError("zoom factor must be finite and positive"));
        }
        let document = self
            .to_document(anchor)
            .ok_or(VectorError("invalid zoom anchor"))?;
        // Compute in f64 so large finite factors clamp instead of overflowing.
        let scale = ((self.scale as f64 * factor as f64)
            .clamp(self.min_scale as f64, self.max_scale as f64)) as f32;
        let translation = finite_point(VectorPoint::new(
            anchor.x - document.x * scale,
            anchor.y - document.y * scale,
        ))
        .ok_or(VectorError("zoom must remain finite"))?;
        self.scale = scale;
        self.translation = translation;
        Ok(())
    }

    /// Return the renderer's view box for a viewport of this logical size.
    /// The supplied size must match the vector element's content box. Resizing
    /// preserves the scale and translation rather than fitting the document.
    pub fn view_box(&self, width: f32, height: f32) -> Result<VectorViewBox, VectorError> {
        let origin = self
            .to_document(VectorPoint::new(0.0, 0.0))
            .ok_or(VectorError("view origin must be finite"))?;
        let w = width / self.scale;
        let h = height / self.scale;
        if !width.is_finite()
            || !height.is_finite()
            || !w.is_finite()
            || !h.is_finite()
            || w <= f32::EPSILON
            || h <= f32::EPSILON
        {
            return Err(VectorError(
                "viewport dimensions must produce a finite, non-singular view box",
            ));
        }
        Ok(VectorViewBox::new(origin.x, origin.y, w, h))
    }
}

fn finite_point(point: VectorPoint) -> Option<VectorPoint> {
    (point.x.is_finite() && point.y.is_finite()).then_some(point)
}

#[cfg(test)]
mod view_tests {
    use super::*;
    fn near(a: f32, b: f32) {
        assert!((a - b).abs() < 0.002, "{a} != {b}");
    }
    #[test]
    fn anchored_zoom_pan_and_inverse() {
        let mut view = VectorView::new(0.1, 10.0).unwrap();
        let anchor = VectorPoint::new(23.0, 71.0);
        view.pan(VectorPoint::new(12.0, -7.0)).unwrap();
        let document = view.to_document(anchor).unwrap();
        for _ in 0..100 {
            view.zoom_at(1.01, anchor).unwrap();
            let at = view.to_viewport(document).unwrap();
            near(at.x, anchor.x);
            near(at.y, anchor.y);
        }
        let mapped = view
            .to_document(view.to_viewport(document).unwrap())
            .unwrap();
        near(mapped.x, document.x);
        near(mapped.y, document.y);
        let vb = view.view_box(200.0, 100.0).unwrap();
        near((document.x - vb.min_x) * 200.0 / vb.width, anchor.x);
        near((document.y - vb.min_y) * 100.0 / vb.height, anchor.y);
    }
    #[test]
    fn view_changes_retain_geometry_and_export() {
        let path = VectorPath::new(
            vec![
                PathCommand::MoveTo(VectorPoint::new(1.0, 2.0)),
                PathCommand::CubicTo {
                    control1: VectorPoint::new(3.0, 4.0),
                    control2: VectorPoint::new(5.0, 6.0),
                    end: VectorPoint::new(7.0, 8.0),
                },
            ],
            VectorPaint::default(),
        )
        .unwrap();
        let authored =
            VectorScene::new(VectorViewBox::new(0.0, 0.0, 100.0, 100.0), vec![path]).unwrap();
        let svg = authored.to_svg();
        let mut render = authored.clone().into_render_scene();
        let mut view = VectorView::new(0.1, 10.0).unwrap();
        for i in 0..1000 {
            view.pan(VectorPoint::new(if i % 2 == 0 { 1.0 } else { -1.0 }, 0.0))
                .unwrap();
            view.zoom_at(
                if i % 2 == 0 { 1.01 } else { 1.0 / 1.01 },
                VectorPoint::new(50.0, 50.0),
            )
            .unwrap();
            render.view_box = view.view_box(100.0, 100.0).unwrap();
            assert!(
                render.paths[0]
                    .geometry
                    .commands
                    .shares_storage_with(authored.paths()[0].geometry())
            );
        }
        assert_eq!(authored.to_svg(), svg);
    }
    #[test]
    fn invalid_operations_are_atomic_and_zoom_is_clamped() {
        assert!(VectorView::new(0.0, 2.0).is_err());
        assert!(VectorView::new(0.1, f32::INFINITY).is_err());
        let mut view = VectorView::new(0.25, 4.0).unwrap();
        let original = view;
        assert!(view.zoom_at(0.0, VectorPoint::new(1.0, 2.0)).is_err());
        assert!(view.zoom_at(2.0, VectorPoint::new(f32::NAN, 2.0)).is_err());
        assert!(view.pan(VectorPoint::new(f32::INFINITY, 0.0)).is_err());
        assert_eq!(view, original);
        assert!(view.view_box(0.0, 100.0).is_err());
        view.zoom_at(f32::MAX, VectorPoint::new(0.0, 0.0)).unwrap();
        assert_eq!(view.scale(), 4.0);
        view.zoom_at(f32::MIN_POSITIVE, VectorPoint::new(0.0, 0.0))
            .unwrap();
        assert_eq!(view.scale(), 0.25);
    }
}
