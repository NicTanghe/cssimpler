//! USD remains the authoritative scene model. GPU resources are derived caches.
pub use openusd::usd::Stage;

use super::*;
use glam::{DVec3, Mat4, Vec3};
use openusd::{gf, sdf, usd as scene};
use openusd_schemas::geom::{
    self, AttributeMetadata, CameraSchema, GprimSchema, ImageableExt, MeshSchema, PointBasedSchema,
    XformableExt,
};
use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    rc::Rc,
};
use wgpu::util::DeviceExt;

mod aa;
#[cfg(test)]
mod aa_tests;

/// An orbit viewer or a camera authored in the USD stage.
#[derive(Clone, Debug, Default)]
pub enum UsdCameraMode {
    #[default]
    Orbit,
    Authored(String),
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
}
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CameraUniform {
    view_projection: [[f32; 4]; 4],
    view: [[f32; 4]; 4],
    viewport: [f32; 4],
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
    minimum: Vec3,
    maximum: Vec3,
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
    vertex_count: u32,
    pipeline: wgpu::RenderPipeline,
    camera_buffer: wgpu::Buffer,
    camera_binding: wgpu::BindGroup,
    camera_revision: Rc<Cell<u64>>,
    camera_sink: scene::StageSinkId,
    camera_key: Option<CameraKey>,
    targets: aa::Targets,
    pipeline_layout: wgpu::PipelineLayout,
    info: GpuViewportInfo,
    redraw_sink: scene::StageSinkId,
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
        let initial_time = Some(scene::TimeCode::new(scene.stage.start_time_code()));
        let cache = extract_meshes(&scene.stage, initial_time)?;
        let redraw = init.redraw.clone();
        let control = UsdViewControl(Rc::new(RefCell::new(ViewState {
            time: initial_time,
            redraw: init.redraw,
        })));
        let (camera_stage, camera_path, orbit) = match mode {
            UsdCameraMode::Authored(path) => {
                let path = sdf::Path::new(&path).map_err(error)?;
                if geom::Camera::get(&scene.stage, &path)
                    .map_err(error)?
                    .is_none()
                {
                    return Err(error(format!("USD camera not found: {path}")));
                }
                (scene.stage.clone(), path, None)
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
                let up = match scene.stage.stage_metadata("upAxis").map_err(error)? {
                    Some(sdf::Value::Token(axis)) if axis.as_ref() == "Z" => Vec3::Z,
                    _ => Vec3::Y,
                };
                let orbit = Orbit {
                    center,
                    up,
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
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
        let camera_binding = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("USD camera"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });
        let pipeline_layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("USD renderer"),
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            });
        let targets = aa::Targets::new(gpu, init.info);
        let pipeline = mesh_pipeline(gpu, &pipeline_layout, init.info, targets.samples);
        let redraw_sink = scene
            .stage
            .add_sink(move |_: &Stage, _: &scene::CommittedChange| {
                redraw.request_redraw();
            });
        let camera_revision = Rc::new(Cell::new(0u64));
        let changes = camera_revision.clone();
        let camera_sink = camera_stage.add_sink(move |_: &Stage, _: &scene::CommittedChange| {
            changes.set(changes.get().wrapping_add(1));
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
            vertex_count: cache.vertices.len() as u32,
            pipeline,
            camera_buffer,
            camera_binding,
            camera_revision,
            camera_sink,
            camera_key: None,
            targets,
            pipeline_layout,
            info: init.info,
            redraw_sink,
        })
    }

    pub fn control(&self) -> UsdViewControl {
        self.control.clone()
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
        let projection = if projection == geom::Projection::Orthographic {
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
                self.pipeline = mesh_pipeline(gpu, &self.pipeline_layout, info, targets.samples);
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
            self.vertex_count = cache.vertices.len() as u32;
            self.revision = self.scene.revision.get();
            self.evaluated_time = time;
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
                pass.set_vertex_buffer(0, self.vertices.slice(..));
                pass.draw(0..self.vertex_count, strip as u32..strip as u32 + 1);
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
) -> wgpu::RenderPipeline {
    let shader = gpu
        .device
        .create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("USD studio shading"),
            source: wgpu::ShaderSource::Wgsl(include_str!("usd.wgsl").into()),
        });
    gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("USD mesh pipeline"), layout: Some(pipeline_layout),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vertex"), compilation_options: Default::default(), buffers: &[Some(wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<Vertex>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x3] })] },
            fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some("fragment"), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format: info.format, blend: None, write_mask: wgpu::ColorWrites::ALL })] }),
            primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState { format: wgpu::TextureFormat::Depth32Float, depth_write_enabled: Some(true), depth_compare: Some(wgpu::CompareFunction::Less), stencil: Default::default(), bias: Default::default() }),
            multisample: wgpu::MultisampleState { count: samples, ..Default::default() }, multiview_mask: None, cache: None,
        })
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
        minimum: Vec3::splat(f32::INFINITY),
        maximum: Vec3::splat(f32::NEG_INFINITY),
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
        let points: Vec<Vec3> = points
            .into_iter()
            .map(|p| {
                let p = world.transform_point(p);
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
                    points
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
            for i in 1..polygon.len() - 1 {
                for corner in [0, i, i + 1] {
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
                    cache.vertices.push(Vertex {
                        position: p.to_array(),
                        normal: shading_normal.to_array(),
                        color,
                    });
                }
            }
            offset = end;
        }
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
            for (vertex, index) in cache.vertices.iter().zip(expected) {
                assert_eq!(vertex.normal, values[index], "{mode}");
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
            assert_eq!(cache.vertices.len(), 6);
            for (v, i) in cache.vertices.iter().zip(expected) {
                assert_eq!(v.normal, directions[i], "{mode}");
            }
        }
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
        assert_eq!(
            extract_meshes(scene.stage(), None).unwrap().vertices.len(),
            36
        );
        assert!(
            geom::Camera::get(scene.stage(), "/World/Camera")
                .unwrap()
                .is_some()
        );
    }
}
