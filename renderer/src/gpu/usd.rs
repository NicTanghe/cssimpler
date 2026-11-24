//! USD remains the authoritative scene model. GPU resources are derived caches.
pub use openusd::usd::Stage;

use super::*;
use glam::{DVec3, Mat4, Vec3};
use openusd::{gf, sdf, usd as scene};
use openusd_schemas::geom::{
    self, AttributeMetadata, CameraSchema, GprimSchema, ImageableExt, MeshSchema, PointBasedSchema,
    PrimvarsAPI, XformableExt,
};
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, hash_map::Entry},
    ops::Range,
    path::{Path, PathBuf},
    rc::Rc,
};
use wgpu::util::DeviceExt;

mod aa;
mod appearance;
mod studio;
#[cfg(test)]
mod studio_tests;
use appearance::{MaterialOverride, Materials};
pub use appearance::{
    UsdColorTexture, UsdMaterial, UsdShadowSettings, UsdStudioLighting, UsdSurface, UsdUvMapping,
};
use studio::Studio;
#[cfg(test)]
mod aa_tests;
#[cfg(test)]
mod wireframe_tests;

/// An orbit viewer or a camera authored in the USD stage.
#[derive(Clone, Debug, Default)]
pub enum UsdCameraMode {
    #[default]
    Orbit,
    Authored(String),
}

/// How the polygon cage is drawn over the shaded surface.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WireframeMode {
    /// Shaded surface only.
    #[default]
    Off,
    /// Every polygon-cage edge, occluded by the surface.
    Edges,
    /// Silhouette and open-boundary edges only, occluded by the surface.
    Silhouette,
    /// Every polygon-cage edge drawn through the surface.
    Overlay,
}

impl WireframeMode {
    /// Cycle order used by the example toggle buttons.
    pub fn next(self) -> Self {
        match self {
            Self::Off => Self::Edges,
            Self::Edges => Self::Silhouette,
            Self::Silhouette => Self::Overlay,
            Self::Overlay => Self::Off,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Edges => "edges",
            Self::Silhouette => "silhouette",
            Self::Overlay => "overlay",
        }
    }

    /// Parse the value accepted by `--wireframe=` in the examples.
    pub fn from_label(label: &str) -> Option<Self> {
        match label {
            "off" => Some(Self::Off),
            "edges" => Some(Self::Edges),
            "silhouette" => Some(Self::Silhouette),
            "overlay" => Some(Self::Overlay),
            _ => None,
        }
    }
}

/// Stroke appearance. The band is centred on each edge, so it is visible on
/// both sides of a silhouette; equal `core` and `outline` produce a solid band.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WireframeStyle {
    /// Linear-RGB colour of the stroke interior.
    pub core: [f32; 3],
    /// Linear-RGB colour of the band either side of the core.
    pub outline: [f32; 3],
    /// Total band width in physical pixels.
    pub width_px: f32,
}

impl Default for WireframeStyle {
    fn default() -> Self {
        // A solid 2px black band straddles the edge, half on each side.
        Self {
            core: [0.0, 0.0, 0.0],
            outline: [0.0, 0.0, 0.0],
            width_px: 2.0,
        }
    }
}

/// Wireframe selection for one mesh, or the viewport default when a mesh has no
/// override.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Wireframe {
    pub mode: WireframeMode,
    pub style: WireframeStyle,
}

/// A cheap, event-thread-only shared USD stage. No second scene graph is created.
#[derive(Clone)]
pub struct UsdScene {
    stage: Stage,
    revision: Rc<Cell<u64>>,
}

impl UsdScene {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path
            .as_ref()
            .to_str()
            .ok_or_else(|| error("USD path must be UTF-8"))?;
        let stage = Stage::builder()
            .schema_registry(openusd_schemas::schema_registry())
            .open(path)
            .map_err(error)?;
        Self::from_stage(stage)
    }
    /// Use the schema registry from `openusd_schemas` when constructing the stage.
    pub fn from_stage(stage: Stage) -> Result<Self> {
        let revision = Rc::new(Cell::new(1u64));
        let changes = revision.clone();
        stage.add_sink(move |_: &Stage, _: &scene::CommittedChange| {
            changes.set(changes.get().wrapping_add(1));
        });
        Ok(Self { stage, revision })
    }
    pub fn stage(&self) -> &Stage {
        &self.stage
    }
}

pub enum UsdSource {
    File(PathBuf),
    Scene(UsdScene),
}
impl From<&str> for UsdSource {
    fn from(path: &str) -> Self {
        Self::File(path.into())
    }
}
impl From<String> for UsdSource {
    fn from(path: String) -> Self {
        Self::File(path.into())
    }
}
impl From<PathBuf> for UsdSource {
    fn from(path: PathBuf) -> Self {
        Self::File(path)
    }
}
impl From<UsdScene> for UsdSource {
    fn from(scene: UsdScene) -> Self {
        Self::Scene(scene)
    }
}
impl From<Stage> for UsdSource {
    fn from(stage: Stage) -> Self {
        Self::Scene(UsdScene::from_stage(stage).expect("stage observer registration is infallible"))
    }
}

/// Factory accepted by `with_gpu_viewport`, including transparent viewport options.
pub fn factory(
    source: impl Into<UsdSource>,
    camera: UsdCameraMode,
) -> impl FnOnce(GpuViewportInit<'_>) -> Result<Box<dyn GpuViewportRenderer>> + 'static {
    let source = source.into();
    move |init| {
        let scene = match source {
            UsdSource::File(path) => UsdScene::open(path)?,
            UsdSource::Scene(scene) => scene,
        };
        Ok(Box::new(UsdRenderer::new(init, scene, camera)?))
    }
}

#[derive(Clone)]
pub struct UsdViewControl(Rc<RefCell<ViewState>>);
impl UsdViewControl {
    /// Change evaluation time without changing another view of the same stage.
    pub fn set_time(&self, time: Option<f64>) -> Result<()> {
        if time.is_some_and(|t| !t.is_finite()) {
            return Err(error("USD time must be finite"));
        }
        let mut view = self.0.borrow_mut();
        view.time = time.map(scene::TimeCode::new);
        view.redraw.request_redraw();
        Ok(())
    }
}

struct ViewState {
    time: Option<scene::TimeCode>,
    redraw: GpuRedrawHandle,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    position: [f32; 3],
    normal: [f32; 3],
    color: [f32; 3],
    uv: [f32; 3],
    local: [f32; 3],
}

const VERTEX_LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
    array_stride: std::mem::size_of::<Vertex>() as u64,
    step_mode: wgpu::VertexStepMode::Vertex,
    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x3, 3 => Float32x3, 4 => Float32x3],
};
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CameraUniform {
    view_projection: [[f32; 4]; 4],
    view: [[f32; 4]; 4],
    viewport: [f32; 4],
    projection_params: [f32; 4],
}

/// One unique polygon-cage edge. `adjacency[0]` counts the adjacent faces that
/// contributed a normal; fewer than two is an open boundary edge.
/// `adjacency[1]` is the model that owns the edge.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct EdgeRecord {
    a: [f32; 4],
    b: [f32; 4],
    n0: [f32; 4],
    n1: [f32; 4],
    adjacency: [u32; 4],
}

impl EdgeRecord {
    const ZERO: Self = Self {
        a: [0.0; 4],
        b: [0.0; 4],
        n0: [0.0; 4],
        n1: [0.0; 4],
        adjacency: [0; 4],
    };
}

/// Matches `ModelWire` in `usd/viewport.wesl`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ModelWire {
    core: [f32; 4],
    outline: [f32; 4],
    width_px: f32,
    mode: u32,
    pad: [u32; 2],
}

#[derive(Clone, Copy, PartialEq)]
struct CameraKey {
    revision: u64,
    time: Option<scene::TimeCode>,
    size: [u32; 2],
    target_height: u32,
    format: wgpu::TextureFormat,
}

struct MeshCache {
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
    edges: Vec<EdgeRecord>,
    /// Mesh prim paths, in the order used as the per-edge model id.
    models: Vec<String>,
    ranges: Vec<Range<u32>>,
    shadow_ranges: Vec<Range<u32>>,
    shadow_bounds: Option<[Vec3; 2]>,
    minimum: Vec3,
    maximum: Vec3,
    up: Vec3,
}

/// Initial USD-to-wgpu backend. The stage and camera USD prims are authoritative;
/// vertex arrays and uniforms below are disposable rendering caches.
pub struct UsdRenderer {
    scene: UsdScene,
    control: UsdViewControl,
    camera_stage: Stage,
    camera_path: sdf::Path,
    orbit: Option<Orbit>,
    revision: u64,
    evaluated_time: Option<scene::TimeCode>,
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
    pipeline: wgpu::RenderPipeline,
    camera_buffer: wgpu::Buffer,
    camera_binding: wgpu::BindGroup,
    camera_revision: Rc<Cell<u64>>,
    camera_sink: scene::StageSinkId,
    camera_key: Option<CameraKey>,
    targets: aa::Targets,
    pipeline_layout: wgpu::PipelineLayout,
    studio: Studio,
    materials: Materials,
    model_materials: HashMap<String, MaterialOverride>,
    model_ranges: Vec<Range<u32>>,
    info: GpuViewportInfo,
    redraw_sink: scene::StageSinkId,
    wireframe: Wireframe,
    model_wireframes: HashMap<String, Wireframe>,
    models: Vec<String>,
    wire_generation: u64,
    wire_uploaded: Option<u64>,
    wire_occluded: bool,
    wire_overlay: bool,
    edges: wgpu::Buffer,
    edge_vertices: u32,
    model_wires: wgpu::Buffer,
    wire_binding: wgpu::BindGroup,
    wire_layout: wgpu::BindGroupLayout,
    wire_pipeline_layout: wgpu::PipelineLayout,
    wire_pipeline: wgpu::RenderPipeline,
    wire_overlay_pipeline: wgpu::RenderPipeline,
}

struct Orbit {
    center: Vec3,
    up: Vec3,
    radius: f32,
    yaw: f32,
    pitch: f32,
    drag: bool,
    last: Option<[f32; 2]>,
    camera_dirty: bool,
}

impl UsdRenderer {
    pub fn new(init: GpuViewportInit<'_>, scene: UsdScene, mode: UsdCameraMode) -> Result<Self> {
        Self::new_with_camera_stage(init, scene, mode, None)
    }

    /// Use a camera on a separate stage so its edits redraw the view without
    /// rebuilding the scene's mesh buffers or light-space depth map.
    pub fn new_with_authored_camera(
        init: GpuViewportInit<'_>,
        scene: UsdScene,
        camera_stage: Stage,
        path: &str,
    ) -> Result<Self> {
        Self::new_with_camera_stage(
            init,
            scene,
            UsdCameraMode::Authored(path.into()),
            Some(camera_stage),
        )
    }

    fn new_with_camera_stage(
        init: GpuViewportInit<'_>,
        scene: UsdScene,
        mode: UsdCameraMode,
        camera_stage: Option<Stage>,
    ) -> Result<Self> {
        let initial_time = Some(scene::TimeCode::new(scene.stage.start_time_code()));
        let cache = extract_meshes(&scene.stage, initial_time)?;
        let redraw = init.redraw.clone();
        let camera_redraw = camera_stage.is_some().then(|| redraw.clone());
        let control = UsdViewControl(Rc::new(RefCell::new(ViewState {
            time: initial_time,
            redraw: init.redraw,
        })));
        let (camera_stage, camera_path, orbit) = match mode {
            UsdCameraMode::Authored(path) => {
                let path = sdf::Path::new(&path).map_err(error)?;
                let camera_stage = camera_stage.unwrap_or_else(|| scene.stage.clone());
                if geom::Camera::get(&camera_stage, &path)
                    .map_err(error)?
                    .is_none()
                {
                    return Err(error(format!("USD camera not found: {path}")));
                }
                (camera_stage, path, None)
            }
            UsdCameraMode::Orbit => {
                let camera_stage = Stage::builder()
                    .schema_registry(openusd_schemas::schema_registry())
                    .in_memory("cssimpler-viewport-camera.usda")
                    .map_err(error)?;
                let camera_path = sdf::Path::new("/Camera").map_err(error)?;
                let camera = geom::Camera::define(&camera_stage, &camera_path).map_err(error)?;
                camera
                    .create_focal_length_attr()
                    .map_err(error)?
                    .set(35.0_f32)
                    .map_err(error)?;
                camera
                    .create_vertical_aperture_attr()
                    .map_err(error)?
                    .set(24.0_f32)
                    .map_err(error)?;
                camera
                    .create_horizontal_aperture_attr()
                    .map_err(error)?
                    .set(36.0_f32)
                    .map_err(error)?;
                let center = (cache.minimum + cache.maximum) * 0.5;
                let aspect = init.info.physical_size[0] as f32 / init.info.physical_size[1] as f32;
                let radius =
                    (cache.maximum - cache.minimum).length().max(0.01) * 1.6 / aspect.min(1.0);
                let orbit = Orbit {
                    center,
                    up: cache.up,
                    radius,
                    yaw: 0.5,
                    pitch: 0.25,
                    drag: false,
                    last: None,
                    camera_dirty: false,
                };
                update_orbit_camera(&camera_stage, &camera_path, &orbit)?;
                (camera_stage, camera_path, Some(orbit))
            }
        };
        let gpu = init.gpu;
        let vertices = vertex_buffer(gpu, &cache.vertices);
        let indices = index_buffer(gpu, &cache.indices);
        let camera_buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("USD camera uniforms"),
            size: std::mem::size_of::<CameraUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = gpu
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("USD camera layout"),
                entries: &[
                    appearance::uniform_entry(0, wgpu::ShaderStages::VERTEX_FRAGMENT),
                    appearance::uniform_entry(1, wgpu::ShaderStages::VERTEX_FRAGMENT),
                ],
            });
        let studio = Studio::new(gpu, &layout, &cache);
        let materials = Materials::new(gpu);
        let camera_binding = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("USD view uniforms"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: camera_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: studio.uniform.as_entire_binding(),
                },
            ],
        });
        let mut mesh_layouts = vec![Some(&layout), Some(&materials.layout)];
        if let Some(shadow_layout) = studio.shadow_layout() {
            mesh_layouts.push(Some(shadow_layout));
        }
        let pipeline_layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("USD renderer"),
                bind_group_layouts: &mesh_layouts,
                immediate_size: 0,
            });
        let targets = aa::Targets::new(gpu, init.info);
        let pipeline = mesh_pipeline(
            gpu,
            &pipeline_layout,
            init.info,
            targets.samples,
            studio.shadow_layout().is_some(),
        );
        let wire_layout = wire_bind_group_layout(&gpu.device);
        let wire_pipeline_layout =
            gpu.device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("USD wireframe"),
                    bind_group_layouts: &[Some(&layout), Some(&wire_layout)],
                    immediate_size: 0,
                });
        let (wire_pipeline, wire_overlay_pipeline) =
            wire_pipelines(gpu, &wire_pipeline_layout, init.info, targets.samples);
        let model_wires = model_wire_buffer(gpu, cache.models.len());
        let (edges, wire_binding, edge_vertices) =
            wire_resources(gpu, &wire_layout, &model_wires, &cache.edges);
        let redraw_sink = scene
            .stage
            .add_sink(move |_: &Stage, _: &scene::CommittedChange| {
                redraw.request_redraw();
            });
        let camera_revision = Rc::new(Cell::new(0u64));
        let changes = camera_revision.clone();
        let camera_sink = camera_stage.add_sink(move |_: &Stage, _: &scene::CommittedChange| {
            changes.set(changes.get().wrapping_add(1));
            if let Some(redraw) = &camera_redraw {
                redraw.request_redraw();
            }
        });
        Ok(Self {
            revision: scene.revision.get(),
            evaluated_time: initial_time,
            scene,
            control,
            camera_stage,
            camera_path,
            orbit,
            vertices,
            indices,
            index_count: cache.indices.len() as u32,
            pipeline,
            camera_buffer,
            camera_binding,
            camera_revision,
            camera_sink,
            camera_key: None,
            targets,
            pipeline_layout,
            studio,
            materials,
            model_materials: HashMap::new(),
            model_ranges: cache.ranges,
            info: init.info,
            redraw_sink,
            wireframe: Wireframe::default(),
            model_wireframes: HashMap::new(),
            models: cache.models.clone(),
            wire_generation: 0,
            wire_uploaded: None,
            wire_occluded: false,
            wire_overlay: false,
            edges,
            edge_vertices,
            model_wires,
            wire_binding,
            wire_layout,
            wire_pipeline_layout,
            wire_pipeline,
            wire_overlay_pipeline,
        })
    }

    pub fn control(&self) -> UsdViewControl {
        self.control.clone()
    }

    /// Set view-local studio lighting. Pass `None` for the original camera-relative
    /// lighting and background. The USD stage and other views are left alone.
    pub fn set_studio_lighting(&mut self, settings: Option<UsdStudioLighting>) -> Result<()> {
        if let Some(settings) = settings {
            settings.validate()?;
        }
        if self.studio.set_settings(settings) {
            self.control.0.borrow().redraw.request_redraw();
        }
        Ok(())
    }

    /// Read the current studio key's shadow settings.
    pub fn shadow_settings(&self) -> Option<UsdShadowSettings> {
        self.studio.settings.and_then(|settings| settings.shadows)
    }

    /// Change the existing studio key's shadow sampling parameters at runtime.
    /// Adjusting radius, strength or bias reuses the existing depth map.
    pub fn set_shadow_settings(&mut self, shadows: UsdShadowSettings) -> Result<()> {
        let mut settings = self
            .studio
            .settings
            .ok_or_else(|| error("studio lighting is disabled"))?;
        settings.shadows = Some(shadows);
        self.set_studio_lighting(Some(settings))
    }

    /// Whether the viewport's GPU supports a sampled key-light depth map.
    pub fn supports_shadow_mapping(&self) -> bool {
        self.studio.shadow_layout().is_some()
    }

    /// Assign a render-only material for an exact USD mesh prim path. An
    /// assignment may precede the mesh's appearance in the evaluated stage.
    pub fn set_model_material(&mut self, path: &str, material: UsdMaterial) -> Result<()> {
        material.validate(self.materials.max_dimension)?;
        if self.model_material(path) != Some(&material) {
            self.model_materials.insert(
                path.to_owned(),
                MaterialOverride {
                    material,
                    binding: None,
                },
            );
            self.control.0.borrow().redraw.request_redraw();
        }
        Ok(())
    }

    pub fn model_material(&self, path: &str) -> Option<&UsdMaterial> {
        self.model_materials.get(path).map(|entry| &entry.material)
    }

    pub fn clear_model_material(&mut self, path: &str) {
        if self.model_materials.remove(path).is_some() {
            self.control.0.borrow().redraw.request_redraw();
        }
    }

    fn wire_changed(&mut self) {
        self.wire_generation = self.wire_generation.wrapping_add(1);
        self.control.0.borrow().redraw.request_redraw();
    }

    /// Set the default mode for meshes without their own override. The default
    /// style is kept.
    pub fn set_wireframe(&mut self, mode: WireframeMode) {
        let next = Wireframe {
            mode,
            ..self.wireframe
        };
        self.set_wireframe_default(next);
    }

    /// Replace the viewport defaults applied to meshes without an override.
    pub fn set_wireframe_default(&mut self, wireframe: Wireframe) {
        if self.wireframe == wireframe {
            return;
        }
        self.wireframe = wireframe;
        self.wire_changed();
    }

    pub fn wireframe(&self) -> Wireframe {
        self.wireframe
    }

    /// Set the default stroke width and colours, keeping the default mode.
    pub fn set_wireframe_style(&mut self, style: WireframeStyle) {
        let next = Wireframe {
            style,
            ..self.wireframe
        };
        self.set_wireframe_default(next);
    }

    pub fn wireframe_style(&self) -> WireframeStyle {
        self.wireframe.style
    }

    /// Override the wireframe for the mesh at `path`, which is a prim path such
    /// as `/World/Preview`. Paths are matched exactly and unknown paths are
    /// kept until the mesh appears.
    pub fn set_model_wireframe(&mut self, path: &str, wireframe: Wireframe) {
        if self.model_wireframes.get(path) == Some(&wireframe) {
            return;
        }
        self.model_wireframes.insert(path.to_owned(), wireframe);
        self.wire_changed();
    }

    pub fn clear_model_wireframe(&mut self, path: &str) {
        if self.model_wireframes.remove(path).is_some() {
            self.wire_changed();
        }
    }

    pub fn model_wireframe(&self, path: &str) -> Option<Wireframe> {
        self.model_wireframes.get(path).copied()
    }

    /// Mesh prim paths, in the order used as the per-edge model id.
    pub fn models(&self) -> &[String] {
        &self.models
    }

    /// Upload the per-model stroke parameters when the model list or the
    /// wireframe settings have changed since the last frame.
    fn sync_wire(&mut self, gpu: &GpuContext) {
        if self.wire_uploaded == Some(self.wire_generation) {
            return;
        }
        let paths = self.models.clone();
        let mut params = Vec::with_capacity(paths.len());
        let mut occluded = false;
        let mut overlay = false;
        for path in &paths {
            let wireframe = self
                .model_wireframes
                .get(path)
                .copied()
                .unwrap_or(self.wireframe);
            let mode = match wireframe.mode {
                WireframeMode::Off => 0,
                WireframeMode::Edges => 1,
                WireframeMode::Silhouette => 2,
                WireframeMode::Overlay => 3,
            };
            occluded |= mode == 1 || mode == 2;
            overlay |= mode == 3;
            params.push(model_wire(wireframe, mode));
        }
        self.wire_occluded = occluded;
        self.wire_overlay = overlay;
        if !params.is_empty() {
            gpu.queue
                .write_buffer(&self.model_wires, 0, bytemuck::cast_slice(&params));
        }
        self.wire_uploaded = Some(self.wire_generation);
    }

    fn camera_uniform(&self, time: Option<scene::TimeCode>) -> Result<CameraUniform> {
        let camera = geom::Camera::get(&self.camera_stage, &self.camera_path)
            .map_err(error)?
            .ok_or_else(|| error("USD camera was removed"))?;
        let mut xforms = geom::XformCache::new(time);
        let prim = self.camera_stage.prim(&self.camera_path).map_err(error)?;
        let world = xforms.local_to_world_transform(&prim).map_err(error)?;
        let view = Mat4::from_cols_array(&world.0.map(|v| v as f32)).inverse();
        let aspect = self.info.physical_size[0] as f32 / self.info.physical_size[1] as f32;
        let clip = camera
            .clipping_range_attr()
            .get_at::<gf::Vec2f>(time)
            .map_err(error)?
            .unwrap_or(gf::vec2f(0.01, 1_000_000.0));
        let near = clip.x.max(0.0001);
        let far = clip.y.max(near + 0.001);
        let aperture = camera
            .vertical_aperture_attr()
            .get_at::<f32>(time)
            .map_err(error)?
            .unwrap_or(15.2908)
            .max(0.001);
        let focal = camera
            .focal_length_attr()
            .get_at::<f32>(time)
            .map_err(error)?
            .unwrap_or(50.0)
            .max(0.001);
        let projection = camera
            .projection_attr()
            .get_at::<geom::Projection>(time)
            .map_err(error)?
            .unwrap_or(geom::Projection::Perspective);
        let orthographic = projection == geom::Projection::Orthographic;
        let projection = if orthographic {
            let height = aperture * 0.1;
            glam::camera::rh::proj::directx::orthographic(
                -height * aspect * 0.5,
                height * aspect * 0.5,
                -height * 0.5,
                height * 0.5,
                near,
                far,
            )
        } else {
            glam::camera::rh::proj::directx::perspective(
                2.0 * (aperture / (2.0 * focal)).atan(),
                aspect,
                near,
                far,
            )
        };
        Ok(CameraUniform {
            view_projection: (projection * view).to_cols_array_2d(),
            view: view.to_cols_array_2d(),
            viewport: [
                self.info.physical_size[0] as f32,
                self.info.physical_size[1] as f32,
                self.targets.height as f32,
                if self.info.format.is_srgb() { 0.0 } else { 1.0 },
            ],
            projection_params: [f32::from(orthographic), 0.0, 0.0, 0.0],
        })
    }
}

impl GpuViewportRenderer for UsdRenderer {
    fn resize(&mut self, gpu: &GpuContext, info: GpuViewportInfo) -> Result<()> {
        if self.info.physical_size != info.physical_size
            || self.info.format != info.format
            || self.info.texture_usage != info.texture_usage
        {
            let targets = aa::Targets::new(gpu, info);
            if self.targets.samples != targets.samples || self.info.format != info.format {
                self.pipeline = mesh_pipeline(
                    gpu,
                    &self.pipeline_layout,
                    info,
                    targets.samples,
                    self.supports_shadow_mapping(),
                );
                let (wire, overlay) =
                    wire_pipelines(gpu, &self.wire_pipeline_layout, info, targets.samples);
                self.wire_pipeline = wire;
                self.wire_overlay_pipeline = overlay;
            }
            self.targets = targets;
        }
        self.info = info;
        Ok(())
    }
    fn event(&mut self, event: &EngineEvent) -> bool {
        let Some(orbit) = &mut self.orbit else {
            return false;
        };
        handle_orbit_event(orbit, event)
    }
    fn render(&mut self, gpu: &GpuContext, frame: GpuViewportFrame<'_>) -> Result<()> {
        if let Some(orbit) = &mut self.orbit {
            flush_orbit_camera(&self.camera_stage, &self.camera_path, orbit)?;
        }
        let time = self.control.0.borrow().time;
        if self.revision != self.scene.revision.get() || self.evaluated_time != time {
            let cache = extract_meshes(&self.scene.stage, time)?;
            self.vertices = vertex_buffer(gpu, &cache.vertices);
            self.indices = index_buffer(gpu, &cache.indices);
            self.index_count = cache.indices.len() as u32;
            self.model_wires = model_wire_buffer(gpu, cache.models.len());
            let (edges, wire_binding, edge_vertices) =
                wire_resources(gpu, &self.wire_layout, &self.model_wires, &cache.edges);
            self.edges = edges;
            self.wire_binding = wire_binding;
            self.edge_vertices = edge_vertices;
            self.studio.update_mesh(&cache);
            self.models = cache.models;
            self.model_ranges = cache.ranges;
            // The model list changed, so per-model parameters must be re-uploaded.
            self.wire_uploaded = None;
            self.revision = self.scene.revision.get();
            self.evaluated_time = time;
        }
        self.sync_wire(gpu);
        self.studio.sync(gpu);
        for path in &self.models {
            if let Some(entry) = self.model_materials.get_mut(path)
                && entry.binding.is_none()
            {
                entry.binding = Some(self.materials.binding(gpu, &entry.material));
            }
        }
        let key = CameraKey {
            revision: self.camera_revision.get(),
            time,
            size: self.info.physical_size,
            target_height: self.targets.height,
            format: self.info.format,
        };
        if self.camera_key != Some(key) {
            gpu.queue.write_buffer(
                &self.camera_buffer,
                0,
                bytemuck::bytes_of(&self.camera_uniform(time)?),
            );
            self.camera_key = Some(key);
        }
        self.studio.render_shadow(
            frame.encoder,
            &self.camera_binding,
            &self.vertices,
            &self.indices,
        );
        let depth = self.targets.depth.create_view(&Default::default());
        let color = self
            .targets
            .color
            .as_ref()
            .map(|t| t.create_view(&Default::default()));
        let resolve = self
            .targets
            .resolve
            .as_ref()
            .map(|t| t.create_view(&Default::default()));
        let clear = clear_color(frame.info);
        let height = frame.info.physical_size[1];
        for (strip, row) in (0..height)
            .step_by(self.targets.height as usize)
            .enumerate()
        {
            {
                let mut pass = frame
                    .encoder
                    .begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("USD mesh"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: color.as_ref().unwrap_or(frame.target),
                            resolve_target: resolve
                                .as_ref()
                                .or_else(|| color.as_ref().map(|_| frame.target)),
                            depth_slice: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(clear),
                                store: if color.is_some() {
                                    wgpu::StoreOp::Discard
                                } else {
                                    wgpu::StoreOp::Store
                                },
                            },
                        })],
                        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                            view: &depth,
                            depth_ops: Some(wgpu::Operations {
                                load: wgpu::LoadOp::Clear(1.0),
                                store: wgpu::StoreOp::Discard,
                            }),
                            stencil_ops: None,
                        }),
                        ..Default::default()
                    });
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &self.camera_binding, &[]);
                if let Some(binding) = self.studio.shadow_binding() {
                    pass.set_bind_group(2, binding, &[]);
                }
                let instance = strip as u32..strip as u32 + 1;
                pass.set_vertex_buffer(0, self.vertices.slice(..));
                pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                let default_material = &self.materials.default_binding;
                if self.model_materials.is_empty() {
                    pass.set_bind_group(1, default_material, &[]);
                    pass.draw_indexed(0..self.index_count, 0, instance.clone());
                } else {
                    for (path, range) in self.models.iter().zip(&self.model_ranges) {
                        let material = self
                            .model_materials
                            .get(path)
                            .and_then(|entry| entry.binding.as_ref())
                            .unwrap_or(default_material);
                        pass.set_bind_group(1, material, &[]);
                        pass.draw_indexed(range.clone(), 0, instance.clone());
                    }
                }
                if let Some(floor) = &self.studio.floor {
                    pass.set_bind_group(1, default_material, &[]);
                    pass.set_vertex_buffer(0, floor.slice(..));
                    pass.draw(0..6, instance);
                }
                // Per-model modes resolve on the GPU from the model array: the
                // occluded pipeline emits modes 1 and 2, the through-surface
                // pipeline emits mode 3. Both draw the same edge range, and the
                // stroke pass binds no vertex buffer.
                if self.wire_occluded {
                    pass.set_pipeline(&self.wire_pipeline);
                    pass.set_bind_group(1, &self.wire_binding, &[]);
                    pass.draw(0..self.edge_vertices, strip as u32..strip as u32 + 1);
                }
                if self.wire_overlay {
                    pass.set_pipeline(&self.wire_overlay_pipeline);
                    pass.set_bind_group(1, &self.wire_binding, &[]);
                    pass.draw(0..self.edge_vertices, strip as u32..strip as u32 + 1);
                }
            }
            if let Some(resolve) = &self.targets.resolve {
                frame.encoder.copy_texture_to_texture(
                    resolve.as_image_copy(),
                    wgpu::TexelCopyTextureInfo {
                        origin: wgpu::Origin3d { x: 0, y: row, z: 0 },
                        ..frame.texture.as_image_copy()
                    },
                    wgpu::Extent3d {
                        width: frame.info.physical_size[0],
                        height: self.targets.height.min(height - row),
                        depth_or_array_layers: 1,
                    },
                );
            }
        }
        Ok(())
    }
}

impl Drop for UsdRenderer {
    fn drop(&mut self) {
        self.scene.stage.remove_sink(self.redraw_sink);
        self.camera_stage.remove_sink(self.camera_sink);
    }
}

fn handle_orbit_event(orbit: &mut Orbit, event: &EngineEvent) -> bool {
    match event {
        EngineEvent::PointerButton {
            button: crate::PointerButton::Primary,
            state,
            ..
        } => {
            orbit.drag = *state == crate::ButtonState::Pressed;
            if !orbit.drag {
                orbit.last = None;
            }
            // Capture changes alone do not change the camera pose.
            return true;
        }
        EngineEvent::PointerMoved { position, .. } => {
            let next = [position.x, position.y];
            let previous = orbit.last.replace(next);
            if !orbit.drag {
                // Includes cursor movement while the OS is dragging the window:
                // do not author USD or request a GPU redraw for hover events.
                return false;
            }
            let Some(last) = previous else {
                return true;
            };
            if last == next {
                return true;
            }
            orbit.yaw -= (next[0] - last[0]) * 0.01;
            orbit.pitch = (orbit.pitch + (next[1] - last[1]) * 0.01).clamp(-1.5, 1.5);
        }
        EngineEvent::Scroll { delta, .. } => {
            if !orbit.drag {
                // Includes trackpad scroll momentum delivered after mouseUp.
                return false;
            }
            let dy = match delta {
                crate::ScrollDelta::Lines { y, .. } => *y,
                crate::ScrollDelta::Pixels { y, .. } => *y / 40.0,
            };
            if dy == 0.0 {
                return false;
            }
            orbit.radius = (orbit.radius * (-dy * 0.1).exp()).clamp(0.001, 1e10);
        }
        EngineEvent::FocusChanged(false) | EngineEvent::Suspended => {
            orbit.drag = false;
            orbit.last = None;
            return false;
        }
        _ => return false,
    }
    // Accumulate input cheaply; author the canonical USD camera once before
    // rendering. Hundreds of queued samples must not delay the release event.
    orbit.camera_dirty = true;
    true
}

fn flush_orbit_camera(stage: &Stage, path: &sdf::Path, orbit: &mut Orbit) -> Result<()> {
    if orbit.camera_dirty {
        update_orbit_camera(stage, path, orbit)?;
        orbit.camera_dirty = false;
    }
    Ok(())
}

fn update_orbit_camera(stage: &Stage, path: &sdf::Path, orbit: &Orbit) -> Result<()> {
    let direction = if orbit.up == Vec3::Z {
        Vec3::new(
            orbit.yaw.sin() * orbit.pitch.cos(),
            orbit.yaw.cos() * orbit.pitch.cos(),
            orbit.pitch.sin(),
        )
    } else {
        Vec3::new(
            orbit.yaw.sin() * orbit.pitch.cos(),
            orbit.pitch.sin(),
            orbit.yaw.cos() * orbit.pitch.cos(),
        )
    };
    let world = glam::camera::rh::view::look_at_mat4(
        orbit.center + direction * orbit.radius,
        orbit.center,
        orbit.up,
    )
    .inverse();
    let camera = geom::Camera::get(stage, path)
        .map_err(error)?
        .ok_or_else(|| error("orbit camera missing"))?;
    camera
        .create_clipping_range_attr()
        .map_err(error)?
        .set(gf::vec2f(
            (orbit.radius * 0.001).max(0.0001),
            orbit.radius * 1000.0,
        ))
        .map_err(error)?;
    camera
        .set_transform(gf::Matrix4d(world.to_cols_array().map(f64::from)))
        .map_err(error)?;
    Ok(())
}

fn mesh_pipeline(
    gpu: &GpuContext,
    pipeline_layout: &wgpu::PipelineLayout,
    info: GpuViewportInfo,
    samples: u32,
    shadows_supported: bool,
) -> wgpu::RenderPipeline {
    let shader = usd_shader(gpu, shadows_supported);
    gpu.device
        .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("USD mesh pipeline"),
            layout: Some(pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex"),
                compilation_options: Default::default(),
                buffers: &[Some(VERTEX_LAYOUT)],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fragment"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: info.format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: samples,
                ..Default::default()
            },
            multiview_mask: None,
            cache: None,
        })
}

fn usd_shader(gpu: &GpuContext, shadows_supported: bool) -> wgpu::ShaderModule {
    let source = if shadows_supported {
        include_str!(concat!(env!("OUT_DIR"), "/usd_shadows.wgsl"))
    } else {
        include_str!(concat!(env!("OUT_DIR"), "/usd_no_shadows.wgsl"))
    };
    gpu.device
        .create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("USD studio shading"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        })
}

fn wire_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let storage = |binding: u32| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    };
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("USD wireframe layout"),
        entries: &[storage(0), storage(1)],
    })
}

/// Wireframe pipelines share one stroke shader and differ in depth comparison
/// and which model modes they emit. The overlay variant draws through the
/// surface; both leave depth writes disabled so the stroke never occludes
/// geometry drawn later in the same pass.
fn wire_pipelines(
    gpu: &GpuContext,
    pipeline_layout: &wgpu::PipelineLayout,
    info: GpuViewportInfo,
    samples: u32,
) -> (wgpu::RenderPipeline, wgpu::RenderPipeline) {
    let shader = usd_shader(gpu, false);
    let build = |label: &str, entry: &str, compare: wgpu::CompareFunction| {
        gpu.device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("wire_fragment"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: info.format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(compare),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState {
                    count: samples,
                    ..Default::default()
                },
                multiview_mask: None,
                cache: None,
            })
    };
    (
        build(
            "USD wireframe pipeline",
            "wire_vertex_occluded",
            wgpu::CompareFunction::LessEqual,
        ),
        build(
            "USD wireframe overlay pipeline",
            "wire_vertex_overlay",
            wgpu::CompareFunction::Always,
        ),
    )
}

fn model_wire_buffer(gpu: &GpuContext, models: usize) -> wgpu::Buffer {
    // At least one element keeps the runtime-sized array bindable when a stage
    // has no models; the shader then draws nothing.
    gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("USD wireframe models"),
        size: (models.max(1) * std::mem::size_of::<ModelWire>()) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn wire_resources(
    gpu: &GpuContext,
    layout: &wgpu::BindGroupLayout,
    models: &wgpu::Buffer,
    edges: &[EdgeRecord],
) -> (wgpu::Buffer, wgpu::BindGroup, u32) {
    // A zero-length buffer cannot be bound, and the stage is validated to have
    // geometry, so this fallback only guards degenerate extractions.
    let fallback = [EdgeRecord::ZERO];
    let contents = if edges.is_empty() {
        bytemuck::cast_slice(&fallback)
    } else {
        bytemuck::cast_slice(edges)
    };
    let buffer = gpu
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("USD wireframe edges"),
            contents,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });
    let binding = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("USD wireframe"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: models.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: buffer.as_entire_binding(),
            },
        ],
    });
    (buffer, binding, (edges.len() * 6) as u32)
}

fn model_wire(wireframe: Wireframe, mode: u32) -> ModelWire {
    let WireframeStyle {
        core,
        outline,
        width_px,
    } = wireframe.style;
    ModelWire {
        core: [core[0], core[1], core[2], 1.0],
        outline: [outline[0], outline[1], outline[2], 1.0],
        width_px: width_px.max(1.0),
        mode,
        pad: [0; 2],
    }
}

fn clear_color(info: GpuViewportInfo) -> wgpu::Color {
    let Some(color) = info.background else {
        return wgpu::Color::TRANSPARENT;
    };
    let [r, g, b] = if info.format.is_srgb() {
        let linear = color.to_linear_rgba();
        [linear.r as f64, linear.g as f64, linear.b as f64]
    } else {
        [
            color.r as f64 / 255.0,
            color.g as f64 / 255.0,
            color.b as f64 / 255.0,
        ]
    };
    wgpu::Color { r, g, b, a: 1.0 }
}

fn vertex_buffer(gpu: &GpuContext, vertices: &[Vertex]) -> wgpu::Buffer {
    gpu.device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("USD derived mesh vertices"),
            contents: bytemuck::cast_slice(vertices),
            usage: wgpu::BufferUsages::VERTEX,
        })
}

fn index_buffer(gpu: &GpuContext, indices: &[u32]) -> wgpu::Buffer {
    gpu.device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("USD derived mesh indices"),
            contents: bytemuck::cast_slice(indices),
            usage: wgpu::BufferUsages::INDEX,
        })
}

fn extract_meshes(stage: &Stage, time: Option<scene::TimeCode>) -> Result<MeshCache> {
    let mut paths = Vec::new();
    stage
        .traverse(scene::PrimPredicate::DEFAULT, |path| {
            paths.push(path.clone())
        })
        .map_err(error)?;
    let mut xforms = geom::XformCache::new(time);
    let mut cache = MeshCache {
        vertices: Vec::new(),
        indices: Vec::new(),
        edges: Vec::new(),
        models: Vec::new(),
        ranges: Vec::new(),
        shadow_ranges: Vec::new(),
        shadow_bounds: None,
        up: studio::stage_up(stage)?,
        minimum: Vec3::splat(f32::INFINITY),
        maximum: Vec3::splat(f32::NEG_INFINITY),
    };
    // Flat normals computed on either side of a shared edge can differ only
    // in the sign of a zero component.
    let component_bits = |component: f32| {
        if component == 0.0 {
            0
        } else {
            component.to_bits()
        }
    };
    for path in paths {
        let Some(mesh) = geom::Mesh::get(stage, &path).map_err(error)? else {
            continue;
        };
        if mesh.compute_visibility(time).map_err(error)? == geom::Visibility::Invisible {
            continue;
        }
        let points = mesh
            .points_attr()
            .get_at::<Vec<gf::Vec3f>>(time)
            .map_err(error)?
            .ok_or_else(|| error(format!("{path}: missing points")))?;
        let counts = mesh
            .face_vertex_counts_attr()
            .get_at::<Vec<i32>>(time)
            .map_err(error)?
            .ok_or_else(|| error(format!("{path}: missing faceVertexCounts")))?;
        let indices = mesh
            .face_vertex_indices_attr()
            .get_at::<Vec<i32>>(time)
            .map_err(error)?
            .ok_or_else(|| error(format!("{path}: missing faceVertexIndices")))?;
        let prim = stage.prim(&path).map_err(error)?;
        let casts_shadow = prim
            .attribute("cssimpler:castsShadow")
            .get_at::<bool>(time)
            .map_err(error)?
            .unwrap_or(true);
        let world = xforms.local_to_world_transform(&prim).map_err(error)?;
        let normals_attr = mesh.normals_attr();
        let authored_normals = normals_attr
            .get_at::<Vec<gf::Vec3f>>(time)
            .map_err(|e| error(format!("{path}: cannot read normals: {e}")))?;
        let interpolation = normals_attr.interpolation().map_err(error)?;
        // USD normals default to vertex interpolation when metadata is absent.
        let interpolation = interpolation.as_deref().unwrap_or("vertex");
        let authored_normals = if let Some(normals) = authored_normals {
            let expected = match interpolation {
                "constant" => 1,
                "uniform" => counts.len(),
                "vertex" | "varying" => points.len(),
                "faceVarying" => indices.len(),
                other => {
                    return Err(error(format!(
                        "{path}: unsupported normals interpolation {other}"
                    )));
                }
            };
            if normals.len() != expected {
                return Err(error(format!(
                    "{path}: normals count {} does not match {interpolation} count {expected}",
                    normals.len()
                )));
            }
            let inverse = world.inverse().ok_or_else(|| {
                error(format!(
                    "{path}: cannot transform normals through a singular transform"
                ))
            })?;
            let m = inverse.0;
            Some(
                normals
                    .into_iter()
                    .map(|n| {
                        // Gf uses row vectors: multiply by the inverse transpose.
                        let n = DVec3::new(
                            f64::from(n.x) * m[0] + f64::from(n.y) * m[1] + f64::from(n.z) * m[2],
                            f64::from(n.x) * m[4] + f64::from(n.y) * m[5] + f64::from(n.z) * m[6],
                            f64::from(n.x) * m[8] + f64::from(n.y) * m[9] + f64::from(n.z) * m[10],
                        );
                        let scale = n.abs().max_element();
                        if !n.is_finite() || scale == 0.0 {
                            return Err(error(format!("{path}: invalid authored normal")));
                        }
                        // Scale before taking a length: valid f32 normals can be
                        // too large/small to normalize safely in f32 arithmetic.
                        Ok((n / scale).normalize().as_vec3())
                    })
                    .collect::<Result<Vec<_>>>()?,
            )
        } else {
            None
        };
        let uvs = mesh_uvs(&prim, time, [points.len(), counts.len(), indices.len()]);
        let mut local_min = Vec3::splat(f32::INFINITY);
        let mut local_max = Vec3::splat(f32::NEG_INFINITY);
        for p in &points {
            let p = Vec3::new(p.x, p.y, p.z);
            local_min = local_min.min(p);
            local_max = local_max.max(p);
        }
        let local_center = local_min * 0.5 + local_max * 0.5;
        let world_points: Vec<Vec3> = points
            .iter()
            .map(|p| {
                let p = world.transform_point(*p);
                Vec3::new(p.x, p.y, p.z)
            })
            .collect();
        let colors = mesh
            .display_color_attr()
            .get_at::<Vec<gf::Vec3f>>(time)
            .map_err(error)?;
        let color = colors
            .and_then(|v| v.first().copied())
            .map(|v| [v.x, v.y, v.z])
            .unwrap_or([0.62, 0.68, 0.78]);
        // Cage edges are collected per mesh and deduplicated by point index, so
        // a shared edge is stored once with both adjacent face normals. Fan
        // diagonals never appear because only real polygon edges are emitted.
        // The prim path is this mesh's model id, used to select stroke settings.
        let model = cache.models.len() as u32;
        cache.models.push(path.to_string());
        let first_index = cache.indices.len() as u32;
        let mut vertex_lookup = HashMap::new();
        let mut face_vertices = Vec::new();
        let mut mesh_edges: Vec<EdgeRecord> = Vec::new();
        let mut edge_lookup: HashMap<(u32, u32), usize> = HashMap::new();
        let mut offset = 0usize;
        for (face_index, count) in counts.into_iter().enumerate() {
            if count < 3 {
                return Err(error(format!("{path}: face has fewer than three vertices")));
            }
            let end = offset
                .checked_add(count as usize)
                .ok_or_else(|| error("mesh topology overflow"))?;
            let face = indices
                .get(offset..end)
                .ok_or_else(|| error(format!("{path}: face counts exceed index array")))?;
            let polygon: Vec<Vec3> = face
                .iter()
                .map(|i| {
                    world_points
                        .get(*i as usize)
                        .copied()
                        .ok_or_else(|| error(format!("{path}: invalid point index {i}")))
                })
                .collect::<Result<_>>()?;
            // Initial backend supports convex polygon cages. Refuse concave faces
            // instead of drawing an incorrect fan; subdivision is not evaluated.
            let normal = (polygon[1] - polygon[0])
                .cross(polygon[2] - polygon[0])
                .normalize_or_zero();
            for i in 0..polygon.len() {
                let turn = (polygon[(i + 1) % polygon.len()] - polygon[i])
                    .cross(polygon[(i + 2) % polygon.len()] - polygon[(i + 1) % polygon.len()]);
                if turn.dot(normal) < -1e-6 {
                    return Err(error(format!(
                        "{path}: concave faces are not supported yet"
                    )));
                }
            }
            let normal_array = [normal.x, normal.y, normal.z, 0.0];
            for i in 0..polygon.len() {
                let next = (i + 1) % polygon.len();
                let (first, second) = (face[i], face[next]);
                let key = (first.min(second) as u32, first.max(second) as u32);
                let start = polygon[i].to_array();
                let end = polygon[next].to_array();
                match edge_lookup.entry(key) {
                    Entry::Vacant(slot) => {
                        slot.insert(mesh_edges.len());
                        mesh_edges.push(EdgeRecord {
                            a: [start[0], start[1], start[2], 1.0],
                            b: [end[0], end[1], end[2], 1.0],
                            n0: normal_array,
                            n1: normal_array,
                            adjacency: [1, model, 0, 0],
                        });
                    }
                    Entry::Occupied(slot) => {
                        let edge = &mut mesh_edges[*slot.get()];
                        if edge.adjacency[0] == 1 {
                            edge.n1 = normal_array;
                        }
                        edge.adjacency[0] = edge.adjacency[0].saturating_add(1);
                    }
                }
            }
            face_vertices.clear();
            for corner in 0..polygon.len() {
                let p = polygon[corner];
                let shading_normal = authored_normals.as_ref().map_or(normal, |normals| {
                    let index = match interpolation {
                        "constant" => 0,
                        "uniform" => face_index,
                        "vertex" | "varying" => face[corner] as usize,
                        "faceVarying" => offset + corner,
                        _ => unreachable!(), // Validated above.
                    };
                    normals[index]
                });
                if !p.is_finite() {
                    return Err(error(format!("{path}: non-finite point")));
                }
                cache.minimum = cache.minimum.min(p);
                cache.maximum = cache.maximum.max(p);
                if casts_shadow {
                    let bounds = cache.shadow_bounds.get_or_insert([p, p]);
                    bounds[0] = bounds[0].min(p);
                    bounds[1] = bounds[1].max(p);
                }
                let point_index = face[corner] as usize;
                let uv = uvs.as_ref().map_or([0.0; 3], |(interpolation, values)| {
                    let index = match interpolation {
                        geom::Interpolation::Constant => 0,
                        geom::Interpolation::Uniform => face_index,
                        geom::Interpolation::Vertex | geom::Interpolation::Varying => point_index,
                        geom::Interpolation::FaceVarying => offset + corner,
                    };
                    [values[index].x, values[index].y, 1.0]
                });
                let p_local = points[point_index];
                let local = Vec3::new(p_local.x, p_local.y, p_local.z) - local_center;
                let local = if cache.up == Vec3::Z {
                    Vec3::new(local.x, local.z, -local.y)
                } else {
                    local
                };
                let normal = shading_normal.to_array();
                // Position and local coordinates are fixed for this point within
                // the prim. The resolved normal and UV can differ at seams.
                let key = (
                    point_index,
                    normal.map(component_bits),
                    uv.map(component_bits),
                );
                let index = *vertex_lookup.entry(key).or_insert_with(|| {
                    let index = cache.vertices.len() as u32;
                    cache.vertices.push(Vertex {
                        position: p.to_array(),
                        normal,
                        color,
                        uv,
                        local: local.to_array(),
                    });
                    index
                });
                face_vertices.push(index);
            }
            for pair in face_vertices[1..].windows(2) {
                cache.indices.extend([face_vertices[0], pair[0], pair[1]]);
            }
            offset = end;
        }
        cache.edges.extend(mesh_edges);
        let range = first_index..cache.indices.len() as u32;
        if casts_shadow && !range.is_empty() {
            cache.shadow_ranges.push(range.clone());
        }
        cache.ranges.push(range);
        if offset != indices.len() {
            return Err(error(format!(
                "{path}: face counts do not consume index array"
            )));
        }
    }
    if cache.vertices.is_empty() {
        return Err(error(
            "USD stage has no visible supported Mesh geometry; use a stage with the openusd_schemas registry",
        ));
    }
    Ok(cache)
}

/// Usable indexed or unindexed authored `st`, resolved at the geometry's time.
/// A USD asset can carry other primvar types without affecting plain rendering.
fn mesh_uvs(
    prim: &scene::Prim,
    time: Option<scene::TimeCode>,
    [points, faces, corners]: [usize; 3],
) -> Option<(geom::Interpolation, Vec<gf::Vec2f>)> {
    let api = PrimvarsAPI::from_prim_unchecked(prim.clone());
    let uv = api.find_primvar_with_inheritance("st").ok()??;
    let interpolation = uv.interpolation().ok()?;
    let expected = match interpolation {
        geom::Interpolation::Constant => 1,
        geom::Interpolation::Uniform => faces,
        geom::Interpolation::Vertex | geom::Interpolation::Varying => points,
        geom::Interpolation::FaceVarying => corners,
    };
    let values = match uv.compute_flattened::<Vec<gf::Vec2f>>(time) {
        Ok(Some(values)) => values,
        Err(_) if interpolation == geom::Interpolation::Constant => {
            vec![uv.compute_flattened::<gf::Vec2f>(time).ok()??]
        }
        _ => return None,
    };
    (values.len() == expected && values.iter().all(|v| v.x.is_finite() && v.y.is_finite()))
        .then_some((interpolation, values))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hover_and_button_edges_do_not_author_camera_but_orbit_and_zoom_do() {
        let stage = Stage::builder()
            .schema_registry(openusd_schemas::schema_registry())
            .in_memory("camera-test.usda")
            .unwrap();
        let path = sdf::Path::new("/Camera").unwrap();
        geom::Camera::define(&stage, &path).unwrap();
        let mut orbit = Orbit {
            center: Vec3::ZERO,
            up: Vec3::Y,
            radius: 5.0,
            yaw: 0.5,
            pitch: 0.25,
            drag: false,
            last: None,
            camera_dirty: false,
        };
        update_orbit_camera(&stage, &path, &orbit).unwrap();
        let writes = Rc::new(Cell::new(0));
        let observed = writes.clone();
        stage.add_sink(move |_: &Stage, _: &scene::CommittedChange| {
            observed.set(observed.get() + 1)
        });
        let moved = |x| EngineEvent::PointerMoved {
            position: crate::PointerPosition { x, y: 10.0 },
            modifiers: Default::default(),
        };
        let button = |state| EngineEvent::PointerButton {
            button: crate::PointerButton::Primary,
            state,
            modifiers: Default::default(),
        };
        for x in 0..100 {
            assert!(!handle_orbit_event(&mut orbit, &moved(x as f32)));
        }
        assert_eq!(writes.get(), 0);
        assert_eq!((orbit.yaw, orbit.pitch), (0.5, 0.25));
        assert!(handle_orbit_event(
            &mut orbit,
            &button(crate::ButtonState::Pressed)
        ));
        assert_eq!(writes.get(), 0);
        assert!(handle_orbit_event(&mut orbit, &moved(110.0)));
        // A high-rate pointer burst is accumulated without composing USD for
        // each sample. The camera is committed once when the frame is rendered.
        for x in 111..1111 {
            assert!(handle_orbit_event(&mut orbit, &moved(x as f32)));
        }
        assert_eq!(writes.get(), 0);
        assert!(orbit.camera_dirty);
        flush_orbit_camera(&stage, &path, &mut orbit).unwrap();
        assert!(writes.get() > 0);
        let orbit_writes = writes.get();
        flush_orbit_camera(&stage, &path, &mut orbit).unwrap();
        assert_eq!(
            writes.get(),
            orbit_writes,
            "unchanged frames must not author the camera"
        );
        assert!(handle_orbit_event(
            &mut orbit,
            &button(crate::ButtonState::Released)
        ));
        assert!(!handle_orbit_event(&mut orbit, &moved(120.0)));
        assert_eq!(writes.get(), orbit_writes);
        let released_pose = (orbit.yaw, orbit.pitch, orbit.radius);
        for delta in [
            crate::ScrollDelta::Lines { x: 0.0, y: 1.0 },
            crate::ScrollDelta::Pixels { x: 0.0, y: 40.0 },
        ] {
            assert!(!handle_orbit_event(
                &mut orbit,
                &EngineEvent::Scroll {
                    delta,
                    modifiers: Default::default()
                }
            ));
        }
        assert_eq!((orbit.yaw, orbit.pitch, orbit.radius), released_pose);
        assert!(
            !orbit.camera_dirty,
            "release must stop both rotation and scroll momentum"
        );
        assert!(handle_orbit_event(
            &mut orbit,
            &button(crate::ButtonState::Pressed)
        ));
        assert!(handle_orbit_event(
            &mut orbit,
            &EngineEvent::Scroll {
                delta: crate::ScrollDelta::Lines { x: 0.0, y: 1.0 },
                modifiers: Default::default()
            }
        ));
        assert_eq!(writes.get(), orbit_writes);
        flush_orbit_camera(&stage, &path, &mut orbit).unwrap();
        assert!(writes.get() > orbit_writes);
        assert!(orbit.radius < 5.0);
    }

    fn triangle() -> Stage {
        let stage = Stage::builder()
            .schema_registry(openusd_schemas::schema_registry())
            .in_memory("test.usda")
            .unwrap();
        let mesh = geom::Mesh::define(&stage, "/Cloth").unwrap();
        mesh.create_points_attr()
            .unwrap()
            .set(vec![
                gf::vec3f(0.0, 0.0, 0.0),
                gf::vec3f(1.0, 0.0, 0.0),
                gf::vec3f(0.0, 1.0, 0.0),
            ])
            .unwrap();
        mesh.create_face_vertex_counts_attr()
            .unwrap()
            .set(vec![3_i32])
            .unwrap();
        mesh.create_face_vertex_indices_attr()
            .unwrap()
            .set(vec![0_i32, 1, 2])
            .unwrap();
        stage
    }
    #[test]
    fn authored_normals_follow_usd_interpolation_and_fan_corners() {
        for (mode, values, expected) in [
            ("constant", vec![[1., 0., 0.]], vec![0, 0, 0, 0, 0, 0]),
            ("uniform", vec![[1., 0., 0.]], vec![0, 0, 0, 0, 0, 0]),
            (
                "vertex",
                vec![[1., 0., 0.], [0., 1., 0.], [0., 0., 1.], [-1., 0., 0.]],
                vec![2, 3, 0, 2, 0, 1],
            ),
            (
                "varying",
                vec![[1., 0., 0.], [0., 1., 0.], [0., 0., 1.], [-1., 0., 0.]],
                vec![2, 3, 0, 2, 0, 1],
            ),
            (
                "faceVarying",
                vec![[1., 0., 0.], [0., 1., 0.], [0., 0., 1.], [-1., 0., 0.]],
                vec![0, 1, 2, 0, 2, 3],
            ),
        ] {
            let stage = triangle();
            let mesh = geom::Mesh::get(&stage, "/Cloth").unwrap().unwrap();
            mesh.points_attr()
                .set(vec![
                    gf::vec3f(0., 0., 0.),
                    gf::vec3f(1., 0., 0.),
                    gf::vec3f(1., 1., 0.),
                    gf::vec3f(0., 1., 0.),
                ])
                .unwrap();
            mesh.face_vertex_counts_attr().set(vec![4_i32]).unwrap();
            mesh.face_vertex_indices_attr()
                .set(vec![2_i32, 3, 0, 1])
                .unwrap();
            let normals = mesh.create_normals_attr().unwrap();
            normals
                .clone()
                .set_interpolation(openusd::tf::Token::new(mode))
                .unwrap();
            normals
                .set(
                    values
                        .iter()
                        .map(|v| gf::vec3f(v[0], v[1], v[2]))
                        .collect::<Vec<_>>(),
                )
                .unwrap();
            let cache = extract_meshes(&stage, None).unwrap();
            assert_eq!(cache.vertices.len(), 4);
            assert_eq!(cache.indices, [0, 1, 2, 0, 2, 3]);
            for (vertex, index) in cache.indices.iter().zip(expected) {
                assert_eq!(
                    cache.vertices[*vertex as usize].normal, values[index],
                    "{mode}"
                );
            }
        }
    }

    #[test]
    fn missing_normals_generate_face_normals_but_invalid_authored_normals_fail() {
        let stage = triangle();
        let cache = extract_meshes(&stage, None).unwrap();
        assert!(cache.vertices.iter().all(|v| v.normal == [0., 0., 1.]));
        let mesh = geom::Mesh::get(&stage, "/Cloth").unwrap().unwrap();
        let normals = mesh.create_normals_attr().unwrap();
        normals.clone().set(vec![gf::vec3f(1., 0., 0.)]).unwrap();
        assert!(
            extract_meshes(&stage, None).is_err(),
            "default interpolation is vertex"
        );
        normals.set(vec![gf::vec3f(0., 0., 0.); 3]).unwrap();
        assert!(extract_meshes(&stage, None).is_err());
    }

    #[test]
    fn authored_normals_use_inverse_transpose_and_selected_time() {
        let stage = triangle();
        let mesh = geom::Mesh::get(&stage, "/Cloth").unwrap().unwrap();
        mesh.clone().set_scale(gf::vec3f(2., 1., 1.)).unwrap();
        let normals = mesh.create_normals_attr().unwrap();
        normals.clone().set(vec![gf::vec3f(1., 1., 0.); 3]).unwrap();
        let cache = extract_meshes(&stage, None).unwrap();
        let expected = Vec3::new(0.5, 1., 0.).normalize();
        assert!((Vec3::from_array(cache.vertices[0].normal) - expected).length() < 1e-6);
        normals
            .set_at(vec![gf::vec3f(0., 0., 1.); 3], scene::TimeCode::new(1.0))
            .unwrap();
        let cache = extract_meshes(&stage, Some(scene::TimeCode::new(1.0))).unwrap();
        assert_eq!(cache.vertices[0].normal, [0., 0., 1.]);
    }

    #[test]
    fn normals_are_read_from_an_authored_usda_file() {
        let path = std::env::temp_dir().join(format!(
            "cssimpler-authored-normals-{}.usda",
            std::process::id()
        ));
        std::fs::write(
            &path,
            r#"#usda 1.0

def Mesh "Cloth" {
    point3f[] points = [(0, 0, 0), (1, 0, 0), (0, 1, 0)]
    int[] faceVertexCounts = [3]
    int[] faceVertexIndices = [0, 1, 2]
    normal3f[] normals = [(0, 1, 0)] (
        interpolation = "constant"
    )
}
"#,
        )
        .unwrap();
        let stage = Stage::builder()
            .schema_registry(openusd_schemas::schema_registry())
            .open(path.to_str().unwrap())
            .unwrap();
        let result = extract_meshes(&stage, None);
        drop(stage);
        std::fs::remove_file(path).unwrap();
        let cache = result.unwrap();
        assert_eq!(cache.vertices.len(), 3);
        // The authored direction differs from the geometric +Z face normal.
        assert!(cache.vertices.iter().all(|v| v.normal == [0., 1., 0.]));
    }

    #[test]
    fn authored_normal_indices_follow_multiple_faces() {
        let directions = [
            [1., 0., 0.],
            [0., 1., 0.],
            [0., 0., 1.],
            [-1., 0., 0.],
            [0., -1., 0.],
            [0., 0., -1.],
        ];
        for mode in ["constant", "uniform", "vertex", "varying", "faceVarying"] {
            let stage = triangle();
            let mesh = geom::Mesh::get(&stage, "/Cloth").unwrap().unwrap();
            mesh.points_attr()
                .set(vec![
                    gf::vec3f(0., 0., 0.),
                    gf::vec3f(1., 0., 0.),
                    gf::vec3f(1., 1., 0.),
                    gf::vec3f(0., 1., 0.),
                ])
                .unwrap();
            mesh.face_vertex_counts_attr().set(vec![3_i32, 3]).unwrap();
            mesh.face_vertex_indices_attr()
                .set(vec![2_i32, 0, 1, 0, 2, 3])
                .unwrap();
            let (count, expected) = match mode {
                "constant" => (1, vec![0; 6]),
                "uniform" => (2, vec![0, 0, 0, 1, 1, 1]),
                "vertex" | "varying" => (4, vec![2, 0, 1, 0, 2, 3]),
                _ => (6, vec![0, 1, 2, 3, 4, 5]),
            };
            let normals = mesh.create_normals_attr().unwrap();
            normals
                .clone()
                .set_interpolation(openusd::tf::Token::new(mode))
                .unwrap();
            normals
                .set(
                    directions[..count]
                        .iter()
                        .map(|v| gf::vec3f(v[0], v[1], v[2]))
                        .collect::<Vec<_>>(),
                )
                .unwrap();
            let cache = extract_meshes(&stage, None).unwrap();
            let split = matches!(mode, "uniform" | "faceVarying");
            assert_eq!(cache.vertices.len(), if split { 6 } else { 4 });
            assert_eq!(cache.indices.len(), expected.len());
            for (vertex, i) in cache.indices.iter().zip(expected) {
                assert_eq!(
                    cache.vertices[*vertex as usize].normal, directions[i],
                    "{mode}"
                );
            }
        }
    }

    #[test]
    fn mesh_indices_preserve_face_varying_uv_seams() {
        let stage = triangle();
        let mesh = geom::Mesh::get(&stage, "/Cloth").unwrap().unwrap();
        mesh.points_attr()
            .set(vec![
                gf::vec3f(0., 0., 0.),
                gf::vec3f(1., 0., 0.),
                gf::vec3f(0., 1., 0.),
                gf::vec3f(1., 1., 0.),
            ])
            .unwrap();
        mesh.face_vertex_counts_attr().set(vec![3_i32, 3]).unwrap();
        mesh.face_vertex_indices_attr()
            .set(vec![0_i32, 1, 2, 1, 3, 2])
            .unwrap();
        let primvars = PrimvarsAPI::from_prim_unchecked(stage.prim("/Cloth").unwrap());
        let st = primvars
            .create_primvar("st", "texCoord2f[]")
            .unwrap()
            .set_interpolation(geom::Interpolation::FaceVarying)
            .unwrap();
        st.attribute()
            .clone()
            .set(vec![
                gf::vec2f(0., 0.),
                gf::vec2f(1., 0.),
                gf::vec2f(0., 1.),
                gf::vec2f(1., 1.),
                gf::vec2f(0.5, 0.),
            ])
            .unwrap();
        st.set_indices(vec![0, 1, 2, 4, 3, 2], None).unwrap();

        let cache = extract_meshes(&stage, None).unwrap();
        assert_eq!(cache.indices, [0, 1, 2, 3, 4, 2]);
        assert_eq!(cache.vertices.len(), 5);
        assert_eq!(cache.vertices[1].position, cache.vertices[3].position);
        assert_eq!(cache.vertices[1].uv, [1., 0., 1.]);
        assert_eq!(cache.vertices[3].uv, [0.5, 0., 1.]);
    }

    #[test]
    fn normal_transform_handles_shear_and_ignores_translation() {
        let stage = triangle();
        let mesh = geom::Mesh::get(&stage, "/Cloth").unwrap().unwrap();
        mesh.create_normals_attr()
            .unwrap()
            .set(vec![gf::vec3f(1., 0., 1.); 3])
            .unwrap();
        mesh.set_transform(gf::Matrix4d([
            2., 0., 1., 0., 1., 3., 0., 0., 0., 0., 4., 0., 10., 20., 30., 1.,
        ]))
        .unwrap();
        // Solve A*n' = n for the row-vector point transform; then normalize.
        let expected = Vec3::new(0.375, -0.125, 0.25).normalize();
        let cache = extract_meshes(&stage, None).unwrap();
        for v in cache.vertices {
            assert!((Vec3::from_array(v.normal) - expected).length() < 1e-6);
        }
    }

    #[test]
    fn authored_normal_magnitudes_normalize_without_overflow_or_underflow() {
        for magnitude in [f32::MAX, f32::MIN_POSITIVE, f32::from_bits(1)] {
            let stage = triangle();
            let mesh = geom::Mesh::get(&stage, "/Cloth").unwrap().unwrap();
            mesh.create_normals_attr()
                .unwrap()
                .set(vec![gf::vec3f(magnitude, magnitude, 0.); 3])
                .unwrap();
            let cache = extract_meshes(&stage, None).unwrap();
            let expected = Vec3::new(1., 1., 0.).normalize();
            for v in cache.vertices {
                let n = Vec3::from_array(v.normal);
                assert!(
                    n.is_finite() && (n - expected).length() < 1e-6,
                    "{magnitude}: {n:?}"
                );
            }
        }
    }

    #[test]
    fn malformed_authored_normals_never_silently_use_flat_shading() {
        for (mode, values, message) in [
            (
                "faceVarying",
                vec![gf::vec3f(1., 0., 0.); 2],
                "normals count",
            ),
            (
                "unexpected",
                vec![gf::vec3f(1., 0., 0.); 3],
                "unsupported normals interpolation",
            ),
            (
                "vertex",
                vec![gf::vec3f(f32::NAN, 0., 0.); 3],
                "invalid authored normal",
            ),
            (
                "vertex",
                vec![gf::vec3f(f32::INFINITY, 0., 0.); 3],
                "invalid authored normal",
            ),
            (
                "vertex",
                vec![gf::vec3f(0., 0., 0.); 3],
                "invalid authored normal",
            ),
        ] {
            let stage = triangle();
            let mesh = geom::Mesh::get(&stage, "/Cloth").unwrap().unwrap();
            let normals = mesh.create_normals_attr().unwrap();
            normals
                .clone()
                .set_interpolation(openusd::tf::Token::new(mode))
                .unwrap();
            normals.set(values).unwrap();
            let message_actual = extract_meshes(&stage, None)
                .err()
                .expect("bad normals should fail")
                .to_string();
            assert!(
                message_actual.contains("/Cloth") && message_actual.contains(message),
                "{message_actual}"
            );
        }
        let stage = triangle();
        let mesh = geom::Mesh::get(&stage, "/Cloth").unwrap().unwrap();
        mesh.create_normals_attr()
            .unwrap()
            .set(vec![gf::vec3f(0., 0., 1.); 3])
            .unwrap();
        mesh.set_scale(gf::vec3f(0., 1., 1.)).unwrap();
        assert!(
            extract_meshes(&stage, None)
                .err()
                .unwrap()
                .to_string()
                .contains("singular")
        );
    }

    #[test]
    fn normal_only_edits_invalidate_derived_geometry_and_time_samples_are_normalized() {
        let scene = UsdScene::from_stage(triangle()).unwrap();
        let mesh = geom::Mesh::get(scene.stage(), "/Cloth").unwrap().unwrap();
        let normals = mesh.create_normals_attr().unwrap();
        normals.clone().set(vec![gf::vec3f(1., 0., 0.); 3]).unwrap();
        let before = extract_meshes(scene.stage(), None).unwrap();
        let revision = scene.revision.get();
        normals.clone().set(vec![gf::vec3f(0., 1., 0.); 3]).unwrap();
        assert!(scene.revision.get() > revision);
        let after = extract_meshes(scene.stage(), None).unwrap();
        assert_eq!(before.vertices[0].position, after.vertices[0].position);
        assert_eq!(before.vertices[0].normal, [1., 0., 0.]);
        assert_eq!(after.vertices[0].normal, [0., 1., 0.]);
        normals
            .clone()
            .set_at(vec![gf::vec3f(1., 0., 0.); 3], scene::TimeCode::new(1.))
            .unwrap();
        normals
            .set_at(vec![gf::vec3f(0., 1., 0.); 3], scene::TimeCode::new(3.))
            .unwrap();
        let sampled = extract_meshes(scene.stage(), Some(scene::TimeCode::new(2.))).unwrap();
        let expected = Vec3::new(1., 1., 0.).normalize();
        assert!((Vec3::from_array(sampled.vertices[0].normal) - expected).length() < 1e-6);
    }

    #[test]
    fn mesh_edits_keep_usd_identity_and_update_derived_vertices() {
        let scene = UsdScene::from_stage(triangle()).unwrap();
        let before = extract_meshes(scene.stage(), None).unwrap();
        assert_eq!(before.vertices.len(), 3);
        let revision = scene.revision.get();
        let mesh = geom::Mesh::get(scene.stage(), "/Cloth").unwrap().unwrap();
        mesh.set_translate(gf::vec3d(2.0, 0.0, 0.0)).unwrap();
        assert!(scene.revision.get() > revision);
        let after = extract_meshes(scene.stage(), None).unwrap();
        assert_eq!(after.minimum.x, 2.0);
        assert!(
            scene
                .stage()
                .prim("/Cloth")
                .unwrap()
                .type_name()
                .unwrap()
                .is_some()
        );
    }
    #[test]
    fn invalid_topology_returns_an_error_instead_of_indexing_out_of_bounds() {
        let stage = triangle();
        stage
            .attribute("/Cloth.faceVertexIndices")
            .unwrap()
            .set(vec![0_i32, 1, 100])
            .unwrap();
        assert!(extract_meshes(&stage, None).is_err());
    }
    #[test]
    fn time_samples_are_evaluated_in_usd() {
        let stage = triangle();
        stage
            .attribute("/Cloth.points")
            .unwrap()
            .set_at(
                vec![
                    gf::vec3f(0.0, 0.0, 2.0),
                    gf::vec3f(1.0, 0.0, 2.0),
                    gf::vec3f(0.0, 1.0, 2.0),
                ],
                scene::TimeCode::new(12.0),
            )
            .unwrap();
        let mesh = extract_meshes(&stage, Some(scene::TimeCode::new(12.0))).unwrap();
        assert_eq!(mesh.minimum.z, 2.0);
    }
    #[test]
    fn bundled_usd_file_contains_renderable_mesh_and_authored_camera() {
        let scene = UsdScene::open(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../examples/assets/usd-preview.usda"
        ))
        .unwrap();
        let cache = extract_meshes(scene.stage(), None).unwrap();
        assert_eq!(cache.indices.len(), 36);
        assert_eq!(cache.vertices.len(), 24);
        assert!(
            geom::Camera::get(scene.stage(), "/World/Camera")
                .unwrap()
                .is_some()
        );
    }
}
