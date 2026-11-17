//! Backend-independent vector commands, native elements, and SVG export.
use crate::{
    Color, LayoutBox, SvgBounds, SvgPathGeometry, SvgPathInstance, SvgPathPaint,
    SvgPathPaintSource, SvgPoint, SvgScene, SvgViewBox,
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

/// A small document-to-view transform for a native vector scene.
///
/// A scene can carry one of these as an optional viewport view. It maps authored
/// document coordinates into the scene's viewBox coordinate space:
///
/// ```text
/// view = document * zoom + offset
/// ```
///
/// Panning and zooming only replace this value (plus the scene's copy of it);
/// authored commands, their bounds, and the exported SVG are untouched. The
/// renderer composes the view with the existing layout/viewBox map and any CSS
/// transform, then rasterizes the original curves for the resulting matrix.
///
/// Construction rejects non-finite values and non-positive zooms. Finite zooms
/// are clamped to [`VectorView::MIN_ZOOM`]..=[`VectorView::MAX_ZOOM`], so a view
/// handled through these helpers can never be singular.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VectorView {
    zoom: f32,
    offset_x: f32,
    offset_y: f32,
}

impl VectorView {
    /// Untransformed view: document coordinates map 1:1 into the viewBox.
    pub const IDENTITY: Self = Self {
        zoom: 1.0,
        offset_x: 0.0,
        offset_y: 0.0,
    };
    /// Smallest accepted zoom (1/64). Smaller requests are clamped.
    pub const MIN_ZOOM: f32 = 1.0 / 64.0;
    /// Largest accepted zoom (64). Larger requests are clamped.
    pub const MAX_ZOOM: f32 = 64.0;

    pub const fn identity() -> Self {
        Self::IDENTITY
    }

    /// Validate and clamp a view. Non-finite zooms/offsets are rejected; a finite
    /// positive zoom outside the practical limits is clamped, not rejected.
    pub fn new(zoom: f32, offset: VectorPoint) -> Result<Self, VectorError> {
        if !zoom.is_finite() || zoom <= 0.0 {
            return Err(VectorError("view zoom must be finite and positive"));
        }
        if !offset.x.is_finite() || !offset.y.is_finite() {
            return Err(VectorError("view offset must be finite"));
        }
        Ok(Self {
            zoom: zoom.clamp(Self::MIN_ZOOM, Self::MAX_ZOOM),
            offset_x: offset.x,
            offset_y: offset.y,
        })
    }

    /// View at the given zoom with a zero offset.
    pub fn from_zoom(zoom: f32) -> Result<Self, VectorError> {
        Self::new(zoom, VectorPoint::new(0.0, 0.0))
    }

    pub fn zoom(self) -> f32 {
        self.zoom
    }

    pub fn offset(self) -> VectorPoint {
        VectorPoint::new(self.offset_x, self.offset_y)
    }

    pub fn is_identity(self) -> bool {
        self == Self::IDENTITY
    }

    /// Map an authored document point into viewBox coordinates.
    pub fn document_to_view(self, point: VectorPoint) -> VectorPoint {
        VectorPoint::new(
            point.x * self.zoom + self.offset_x,
            point.y * self.zoom + self.offset_y,
        )
    }

    /// Inverse of [`Self::document_to_view`]. Valid by construction because a
    /// `VectorView` always has a finite positive zoom.
    pub fn view_to_document(self, point: VectorPoint) -> VectorPoint {
        VectorPoint::new(
            (point.x - self.offset_x) / self.zoom,
            (point.y - self.offset_y) / self.zoom,
        )
    }

    /// Map an authored document point to viewport (logical layout/CSS pixel)
    /// coordinates for a scene laid out inside `layout`.
    ///
    /// Returns `None` when the layout or viewBox is non-finite or has a
    /// non-positive size; those are the only degenerate inputs, because the view
    /// itself is always invertible.
    pub fn document_to_viewport(
        self,
        view_box: VectorViewBox,
        layout: LayoutBox,
        point: VectorPoint,
    ) -> Option<VectorPoint> {
        let (scale_x, scale_y) = viewport_scales(view_box, layout)?;
        let view = self.document_to_view(point);
        Some(VectorPoint::new(
            layout.x + (view.x - view_box.min_x) * scale_x,
            layout.y + (view.y - view_box.min_y) * scale_y,
        ))
    }

    /// Convert a pointer position back into authored drawing coordinates.
    ///
    /// Returns `None` for degenerate layouts/viewBoxes (same conditions as
    /// [`Self::document_to_viewport`]).
    pub fn viewport_to_document(
        self,
        view_box: VectorViewBox,
        layout: LayoutBox,
        point: VectorPoint,
    ) -> Option<VectorPoint> {
        let (scale_x, scale_y) = viewport_scales(view_box, layout)?;
        let view = VectorPoint::new(
            view_box.min_x + (point.x - layout.x) / scale_x,
            view_box.min_y + (point.y - layout.y) / scale_y,
        );
        let document = self.view_to_document(view);
        (document.x.is_finite() && document.y.is_finite()).then_some(document)
    }

    /// Zoom by `factor` while keeping `document_anchor` under the same view
    /// coordinate. The resulting zoom is clamped to the practical limits, so the
    /// anchor stays fixed even at a limit.
    pub fn zoomed_about(
        self,
        document_anchor: VectorPoint,
        factor: f32,
    ) -> Result<Self, VectorError> {
        if !factor.is_finite() || factor <= 0.0 {
            return Err(VectorError("view zoom factor must be finite and positive"));
        }
        let view_anchor = self.document_to_view(document_anchor);
        let zoom = (self.zoom * factor).clamp(Self::MIN_ZOOM, Self::MAX_ZOOM);
        Self::new(
            zoom,
            VectorPoint::new(
                view_anchor.x - document_anchor.x * zoom,
                view_anchor.y - document_anchor.y * zoom,
            ),
        )
    }

    /// Zoom around a pointer position in viewport coordinates. The document
    /// point currently under the pointer stays under the pointer.
    pub fn zoomed_about_viewport_point(
        self,
        view_box: VectorViewBox,
        layout: LayoutBox,
        viewport_point: VectorPoint,
        factor: f32,
    ) -> Result<Self, VectorError> {
        let Some(anchor) = self.viewport_to_document(view_box, layout, viewport_point) else {
            return Err(VectorError(
                "view needs a finite layout and viewBox with positive size",
            ));
        };
        self.zoomed_about(anchor, factor)
    }

    /// Pan by a viewport-space delta (for example a pointer drag). The content
    /// follows the pointer: a positive `dx` moves the drawing right.
    pub fn panned_by_viewport_delta(
        self,
        view_box: VectorViewBox,
        layout: LayoutBox,
        dx: f32,
        dy: f32,
    ) -> Result<Self, VectorError> {
        if !dx.is_finite() || !dy.is_finite() {
            return Err(VectorError("view pan delta must be finite"));
        }
        let (scale_x, scale_y) = viewport_scales(view_box, layout).ok_or(VectorError(
            "view needs a finite layout and viewBox with positive size",
        ))?;
        Self::new(
            self.zoom,
            VectorPoint::new(self.offset_x + dx / scale_x, self.offset_y + dy / scale_y),
        )
    }

    /// Translate the view by a delta in viewBox units.
    pub fn translated_by_view(self, dx: f32, dy: f32) -> Result<Self, VectorError> {
        if !dx.is_finite() || !dy.is_finite() {
            return Err(VectorError("view translation must be finite"));
        }
        Self::new(
            self.zoom,
            VectorPoint::new(self.offset_x + dx, self.offset_y + dy),
        )
    }
}

impl Default for VectorView {
    fn default() -> Self {
        Self::IDENTITY
    }
}

fn viewport_scales(view_box: VectorViewBox, layout: LayoutBox) -> Option<(f32, f32)> {
    let finite = |value: f32| value.is_finite();
    if layout.width <= f32::EPSILON
        || layout.height <= f32::EPSILON
        || view_box.width <= f32::EPSILON
        || view_box.height <= f32::EPSILON
        || !finite(layout.x)
        || !finite(layout.y)
        || !finite(layout.width)
        || !finite(layout.height)
        || !finite(view_box.min_x)
        || !finite(view_box.min_y)
        || !finite(view_box.width)
        || !finite(view_box.height)
    {
        return None;
    }
    let scale_x = layout.width / view_box.width;
    let scale_y = layout.height / view_box.height;
    (scale_x.is_finite() && scale_y.is_finite() && scale_x > 0.0 && scale_y > 0.0)
        .then_some((scale_x, scale_y))
}

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
        // Deliberately omit the command slice: cache keys include this Debug
        // output, and the retained identity is enough to invalidate on edits.
        f.debug_struct("VectorGeometry")
            .field("identity", &self.identity())
            .field("commands", &self.commands().len())
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
    view: Option<VectorView>,
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
        Ok(Self {
            view_box,
            view: None,
            paths,
        })
    }
    pub fn view_box(&self) -> VectorViewBox {
        self.view_box
    }
    /// The optional viewport view. Changing it never rebuilds or copies paths.
    pub fn view(&self) -> Option<VectorView> {
        self.view
    }
    /// Attach a viewport view. The scene keeps the same authored paths.
    pub fn with_view(mut self, view: VectorView) -> Self {
        self.view = Some(view);
        self
    }
    /// Replace or clear the viewport view in place; this is O(1) and does not
    /// touch geometry. Prefer this over rebuilding a scene for view-only changes.
    pub fn set_view(&mut self, view: Option<VectorView>) {
        self.view = view;
    }
    pub fn paths(&self) -> &[VectorPath] {
        &self.paths
    }
    /// Export a standalone SVG using numeric user coordinates (default SVG units are px).
    /// The current paint subset uses nonzero fills and round stroke caps/joins.
    /// Any viewport view is deliberately omitted: export always writes authored,
    /// untransformed document geometry.
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
    /// The optional viewport view travels with the scene as render state; path
    /// geometry is moved (handles), not copied or rebuilt.
    pub fn into_render_scene(self) -> SvgScene {
        let mut scene = SvgScene::new(
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
        );
        scene.set_view(self.view);
        scene
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
    fn repeated_revisions_release_old_geometry() {
        let mut current = VectorGeometry::new(vec![
            PathCommand::MoveTo(VectorPoint::new(0., 0.)),
            PathCommand::LineTo(VectorPoint::new(1., 1.)),
        ])
        .unwrap();
        let mut replaced = Vec::new();
        let mut identities = std::collections::HashSet::new();
        identities.insert(current.identity());
        for step in 0..64 {
            replaced.push(Arc::downgrade(&current.0));
            let next = VectorGeometry::new(vec![
                PathCommand::MoveTo(VectorPoint::new(step as f32, 0.)),
                PathCommand::LineTo(VectorPoint::new(step as f32 + 1., 1.)),
            ])
            .unwrap();
            assert!(identities.insert(next.identity()), "identity must be fresh");
            current = next;
        }
        for weak in replaced {
            assert!(
                weak.upgrade().is_none(),
                "replaced geometry must be released, not retained by a registry"
            );
        }
        assert_eq!(Arc::strong_count(&current.0), 1);
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
