//! USD remains the authoritative scene model. GPU resources are derived caches.
pub use openusd::usd::Stage;

use super::*;
use glam::{Mat4, Vec3};
use openusd::{gf, sdf, usd as scene};
use openusd_schemas::geom::{
    self, CameraSchema, GprimSchema, ImageableExt, MeshSchema, PointBasedSchema, XformableExt,
};
use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    rc::Rc,
};
use wgpu::util::DeviceExt;

mod aa;
#[cfg(all(test, target_os = "linux"))]
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
        if self.info.physical_size != info.physical_size || self.info.format != info.format {
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
        handle_orbit_event(&self.camera_stage, &self.camera_path, orbit, event)
    }
    fn render(&mut self, gpu: &GpuContext, frame: GpuViewportFrame<'_>) -> Result<()> {
        let time = self.control.0.borrow().time;
        if self.revision != self.scene.revision.get() || self.evaluated_time != time {
            let cache = extract_meshes(&self.scene.stage, time)?;
            self.vertices = vertex_buffer(gpu, &cache.vertices);
            self.vertex_count = cache.vertices.len() as u32;
            self.revision = self.scene.revision.get();
            self.evaluated_time = time;
        }
        gpu.queue.write_buffer(
            &self.camera_buffer,
            0,
            bytemuck::bytes_of(&self.camera_uniform(time)?),
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
                        label: Some("USD mesh strip"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: color.as_ref().unwrap_or(frame.target),
                            resolve_target: resolve.as_ref(),
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
    }
}

fn handle_orbit_event(
    stage: &Stage,
    path: &sdf::Path,
    orbit: &mut Orbit,
    event: &EngineEvent,
) -> bool {
    match event {
        EngineEvent::PointerButton {
            button: crate::PointerButton::Primary,
            state,
            ..
        } => {
            orbit.drag = *state == crate::ButtonState::Pressed;
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
    // Author only actual orbit/zoom changes in the viewport-owned USD stage.
    if let Err(e) = update_orbit_camera(stage, path, orbit) {
        eprintln!("cssimpler: {e}");
    }
    true
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
        for count in counts {
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
                for p in [polygon[0], polygon[i], polygon[i + 1]] {
                    if !p.is_finite() {
                        return Err(error(format!("{path}: non-finite point")));
                    }
                    cache.minimum = cache.minimum.min(p);
                    cache.maximum = cache.maximum.max(p);
                    cache.vertices.push(Vertex {
                        position: p.to_array(),
                        normal: normal.to_array(),
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
            assert!(!handle_orbit_event(
                &stage,
                &path,
                &mut orbit,
                &moved(x as f32)
            ));
        }
        assert_eq!(writes.get(), 0);
        assert_eq!((orbit.yaw, orbit.pitch), (0.5, 0.25));
        assert!(handle_orbit_event(
            &stage,
            &path,
            &mut orbit,
            &button(crate::ButtonState::Pressed)
        ));
        assert_eq!(writes.get(), 0);
        assert!(handle_orbit_event(&stage, &path, &mut orbit, &moved(110.0)));
        assert!(writes.get() > 0);
        let orbit_writes = writes.get();
        assert!(handle_orbit_event(
            &stage,
            &path,
            &mut orbit,
            &button(crate::ButtonState::Released)
        ));
        assert!(!handle_orbit_event(
            &stage,
            &path,
            &mut orbit,
            &moved(120.0)
        ));
        assert_eq!(writes.get(), orbit_writes);
        assert!(handle_orbit_event(
            &stage,
            &path,
            &mut orbit,
            &EngineEvent::Scroll {
                delta: crate::ScrollDelta::Lines { x: 0.0, y: 1.0 },
                modifiers: Default::default()
            }
        ));
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
