use std::borrow::Cow;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;
use std::time::Instant;

use bytemuck::{Pod, Zeroable};
use cssimpler_core::fonts::{FontFamily, FontStyle, GenericFontFamily, LineHeight, TextTransform};
use cssimpler_core::{
    BackgroundLayer, BoxShadow, Color, CornerRadius, ExtractedPaintItem, ExtractedPaintKind,
    ExtractedScene, GradientDirection, GradientHorizontal, GradientInterpolation, GradientPoint,
    GradientStop, GradientVertical, Insets, LayoutBox, LengthPercentageValue, LinearRgba,
    RadialShape, RenderNode, ShapeExtent, SvgPathGeometry, SvgPathInstance, SvgPoint, SvgScene,
    TransformMatrix3d, TransformStyleMode,
};
use wgpu::util::DeviceExt;
use winit::window::Window;

use crate::fonts::{RasterizedTextMask, rasterize_text_color_texture, rasterize_text_mask};
use crate::shadow::{rasterize_shadow_texture_uncached, shadow_bounds};
use crate::shapes::{inset_corner_radius, inset_layout, offset_layout};
use crate::svg::rasterize_svg_scene_texture;
use crate::transform::{
    AffineTransform, PerspectiveContext, node_local_transform_matrix,
    project_world_transform_matrix,
};
use crate::{RasterizedColorTexture, RendererError, Result};
use crate::{gradient::rasterize_background_layer_texture_uncached, shadow::shadow_effect_bounds};

const MAX_TEXT_TEXTURE_CACHE_ENTRIES: usize = 256;
const MAX_COLOR_TEXTURE_CACHE_ENTRIES: usize = 512;

#[derive(Clone, Copy, Debug)]
struct BackendCapabilities {
    background_layers: bool,
    shadows: bool,
    filter_effects: bool,
    backdrop_blur: bool,
    svg: bool,
    scrollbars: bool,
    transforms: bool,
    text_effects: bool,
    auto_scroll_indicator: bool,
}

impl BackendCapabilities {
    const GPU_BASELINE: Self = Self {
        background_layers: true,
        shadows: true,
        filter_effects: true,
        backdrop_blur: false,
        svg: true,
        scrollbars: false,
        transforms: false,
        text_effects: true,
        auto_scroll_indicator: false,
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ScissorRect {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct ViewportUniform {
    size: [f32; 2],
    _padding: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct FillInstance {
    rect: [f32; 4],
    radii: [f32; 4],
    color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct BorderInstance {
    outer_rect: [f32; 4],
    inner_rect: [f32; 4],
    outer_radii: [f32; 4],
    inner_radii: [f32; 4],
    color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct TextInstance {
    rect: [f32; 4],
    color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct TextureInstance {
    rect: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct ProjectedTextureInstance {
    screen_rect: [f32; 4],
    source_rect: [f32; 4],
    inverse_row0: [f32; 4],
    inverse_row1: [f32; 4],
    inverse_row2: [f32; 4],
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct TextTextureKey {
    text: String,
    families: Vec<TextFontFamilyKey>,
    size_bits: u32,
    weight: u16,
    style: u8,
    line_height: TextLineHeightKey,
    letter_spacing_bits: u32,
    text_transform: u8,
    width_bits: u32,
    origin_x_bits: u32,
    origin_y_bits: u32,
}

impl TextTextureKey {
    fn from_item(text: &str, item: &ExtractedPaintItem, layout: LayoutBox) -> Self {
        let offset_x = layout.x.floor() as i32;
        let offset_y = layout.y.floor() as i32;
        let relative_x = layout.x - offset_x as f32;
        let relative_y = layout.y - offset_y as f32;
        let text_style = &item.style.text;

        Self {
            text: text.to_string(),
            families: text_style
                .families
                .iter()
                .map(text_font_family_key)
                .collect(),
            size_bits: text_style.size_px.to_bits(),
            weight: text_style.weight,
            style: text_font_style_key(text_style.style),
            line_height: text_line_height_key(&text_style.line_height),
            letter_spacing_bits: text_style.letter_spacing_px.to_bits(),
            text_transform: text_transform_key(text_style.text_transform),
            width_bits: layout.width.to_bits(),
            origin_x_bits: relative_x.to_bits(),
            origin_y_bits: relative_y.to_bits(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum TextFontFamilyKey {
    Named(String),
    Generic(u8),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum TextLineHeightKey {
    Normal,
    Px(u32),
    Scale(u32),
}

#[derive(Clone)]
struct CachedTextTexture {
    bind_group: wgpu::BindGroup,
    last_used: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct ColorTextureKey {
    kind: u8,
    fingerprint: u64,
}

#[derive(Clone)]
struct CachedColorTexture {
    bind_group: wgpu::BindGroup,
    last_used: u64,
}

enum GpuPaintCommand {
    Fill {
        scissor: ScissorRect,
        instance_start: u32,
        instance_count: u32,
    },
    Border {
        scissor: ScissorRect,
        instance_start: u32,
        instance_count: u32,
    },
    Text {
        scissor: ScissorRect,
        instance_start: u32,
        instance_count: u32,
    },
    ColorTexture {
        scissor: ScissorRect,
        instance_start: u32,
        instance_count: u32,
    },
    ProjectedTexture {
        scissor: ScissorRect,
        instance_start: u32,
        instance_count: u32,
    },
}

struct PreparedTextCommand {
    instance: TextInstance,
    key: TextTextureKey,
    mask: RasterizedTextMask,
}

struct PreparedColorTextureCommand {
    instance: TextureInstance,
    key: ColorTextureKey,
    raster: Option<RasterizedColorTexture>,
}

struct PreparedProjectedTextureCommand {
    instance: ProjectedTextureInstance,
    key: ColorTextureKey,
    raster: Option<RasterizedColorTexture>,
}

struct PendingProjectedTextureCommand {
    sort_key: u64,
    path: Vec<usize>,
    scissor: ScissorRect,
    command: PreparedProjectedTextureCommand,
}

struct PreparedGpuScene {
    commands: Vec<GpuPaintCommand>,
    fills: Vec<FillInstance>,
    borders: Vec<BorderInstance>,
    texts: Vec<PreparedTextCommand>,
    color_textures: Vec<PreparedColorTextureCommand>,
    projected_textures: Vec<PreparedProjectedTextureCommand>,
}

pub(crate) enum GpuPresentOutcome {
    Presented { paint_us: u64, present_us: u64 },
    Skipped,
    Fallback(String),
}

pub(crate) struct GpuRuntimeBackend {
    instance: wgpu::Instance,
    surface: Option<wgpu::Surface<'static>>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface_config: wgpu::SurfaceConfiguration,
    surface_size: (u32, u32),
    srgb_surface: bool,
    viewport_buffer: wgpu::Buffer,
    viewport_bind_group: wgpu::BindGroup,
    fill_pipeline: wgpu::RenderPipeline,
    border_pipeline: wgpu::RenderPipeline,
    text_pipeline: wgpu::RenderPipeline,
    color_texture_pipeline: wgpu::RenderPipeline,
    projected_texture_pipeline: wgpu::RenderPipeline,
    sampled_texture_bind_group_layout: wgpu::BindGroupLayout,
    text_sampler: wgpu::Sampler,
    color_sampler: wgpu::Sampler,
    text_textures: HashMap<TextTextureKey, CachedTextTexture>,
    color_textures: HashMap<ColorTextureKey, CachedColorTexture>,
    next_texture_use: u64,
}

impl GpuRuntimeBackend {
    pub(crate) fn new(window: &Arc<Window>) -> Result<Self> {
        let instance = wgpu::Instance::default();
        let surface = instance.create_surface(window.clone()).map_err(|error| {
            RendererError::Backend(format!("gpu surface creation failed: {error}"))
        })?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))
        .map_err(|error| RendererError::Backend(format!("gpu adapter request failed: {error}")))?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("cssimpler gpu device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
        }))
        .map_err(|error| RendererError::Backend(format!("gpu device request failed: {error}")))?;

        let capabilities = surface.get_capabilities(&adapter);
        let surface_format = select_surface_format(&capabilities.formats).ok_or_else(|| {
            RendererError::Backend("gpu surface reported no supported formats".to_string())
        })?;
        let srgb_surface = surface_format.is_srgb();
        let inner_size = window.inner_size();
        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: inner_size.width.max(1),
            height: inner_size.height.max(1),
            present_mode: select_present_mode(&capabilities.present_modes),
            alpha_mode: capabilities
                .alpha_modes
                .first()
                .copied()
                .unwrap_or(wgpu::CompositeAlphaMode::Auto),
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &surface_config);

        let viewport_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("cssimpler gpu viewport"),
            contents: bytemuck::bytes_of(&ViewportUniform {
                size: [surface_config.width as f32, surface_config.height as f32],
                _padding: [0.0, 0.0],
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let viewport_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("cssimpler gpu viewport layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
        let viewport_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cssimpler gpu viewport bind group"),
            layout: &viewport_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: viewport_buffer.as_entire_binding(),
            }],
        });
        let sampled_texture_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("cssimpler gpu sampled texture layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            multisampled: false,
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });
        let text_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("cssimpler gpu text sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..wgpu::SamplerDescriptor::default()
        });
        let color_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("cssimpler gpu color texture sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..wgpu::SamplerDescriptor::default()
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cssimpler gpu shader"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!("gpu.wgsl"))),
        });
        let fill_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("cssimpler gpu fill layout"),
            bind_group_layouts: &[&viewport_bind_group_layout],
            push_constant_ranges: &[],
        });
        let border_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("cssimpler gpu border layout"),
                bind_group_layouts: &[&viewport_bind_group_layout],
                push_constant_ranges: &[],
            });
        let text_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("cssimpler gpu text pipeline layout"),
            bind_group_layouts: &[
                &viewport_bind_group_layout,
                &sampled_texture_bind_group_layout,
            ],
            push_constant_ranges: &[],
        });
        let color_texture_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("cssimpler gpu color texture pipeline layout"),
                bind_group_layouts: &[
                    &viewport_bind_group_layout,
                    &sampled_texture_bind_group_layout,
                ],
                push_constant_ranges: &[],
            });
        let projected_texture_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("cssimpler gpu projected texture pipeline layout"),
                bind_group_layouts: &[
                    &viewport_bind_group_layout,
                    &sampled_texture_bind_group_layout,
                ],
                push_constant_ranges: &[],
            });
        let fill_pipeline =
            create_fill_pipeline(&device, &fill_pipeline_layout, &shader, surface_format);
        let border_pipeline =
            create_border_pipeline(&device, &border_pipeline_layout, &shader, surface_format);
        let text_pipeline =
            create_text_pipeline(&device, &text_pipeline_layout, &shader, surface_format);
        let color_texture_pipeline = create_color_texture_pipeline(
            &device,
            &color_texture_pipeline_layout,
            &shader,
            surface_format,
        );
        let projected_texture_pipeline = create_projected_texture_pipeline(
            &device,
            &projected_texture_pipeline_layout,
            &shader,
            surface_format,
        );

        Ok(Self {
            instance,
            surface: Some(surface),
            device,
            queue,
            surface_config,
            surface_size: (inner_size.width.max(1), inner_size.height.max(1)),
            srgb_surface,
            viewport_buffer,
            viewport_bind_group,
            fill_pipeline,
            border_pipeline,
            text_pipeline,
            color_texture_pipeline,
            projected_texture_pipeline,
            sampled_texture_bind_group_layout,
            text_sampler,
            color_sampler,
            text_textures: HashMap::new(),
            color_textures: HashMap::new(),
            next_texture_use: 0,
        })
    }

    pub(crate) fn recreate_surface(&mut self, window: &Arc<Window>) -> Result<()> {
        let surface = self
            .instance
            .create_surface(window.clone())
            .map_err(|error| {
                RendererError::Backend(format!("gpu surface recreation failed: {error}"))
            })?;
        self.surface = Some(surface);
        self.configure_surface(self.surface_size.0.max(1), self.surface_size.1.max(1));
        Ok(())
    }

    pub(crate) fn resize_surface(&mut self, width: u32, height: u32) -> Result<bool> {
        let width = width.max(1);
        let height = height.max(1);
        if self.surface_size == (width, height) {
            return Ok(false);
        }
        self.configure_surface(width, height);
        Ok(true)
    }

    pub(crate) fn suspend(&mut self) {
        self.surface = None;
    }

    pub(crate) fn has_surface(&self) -> bool {
        self.surface.is_some()
    }

    pub(crate) fn present(
        &mut self,
        scene: &ExtractedScene,
        clear_color: Color,
        auto_scroll_indicator_active: bool,
    ) -> Result<GpuPresentOutcome> {
        if auto_scroll_indicator_active && !BackendCapabilities::GPU_BASELINE.auto_scroll_indicator
        {
            return Ok(GpuPresentOutcome::Fallback(
                "middle-button auto-scroll indicator is not supported by the GPU backend yet"
                    .to_string(),
            ));
        }

        let Some(_) = self.surface.as_ref() else {
            return Ok(GpuPresentOutcome::Skipped);
        };
        let paint_start = Instant::now();
        let prepared = match prepare_gpu_scene(
            scene,
            self.surface_size.0,
            self.surface_size.1,
            self.srgb_surface,
            &self.color_textures,
        ) {
            Ok(prepared) => prepared,
            Err(reason) => return Ok(GpuPresentOutcome::Fallback(reason)),
        };

        for text in &prepared.texts {
            self.ensure_text_texture(&text.key, &text.mask);
        }
        for color_texture in &prepared.color_textures {
            self.ensure_color_texture(&color_texture.key, color_texture.raster.as_ref());
        }
        for projected_texture in &prepared.projected_textures {
            self.ensure_color_texture(&projected_texture.key, projected_texture.raster.as_ref());
        }

        let fill_buffer = (!prepared.fills.is_empty()).then(|| {
            self.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("cssimpler gpu fills"),
                    contents: bytemuck::cast_slice(&prepared.fills),
                    usage: wgpu::BufferUsages::VERTEX,
                })
        });
        let border_buffer = (!prepared.borders.is_empty()).then(|| {
            self.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("cssimpler gpu borders"),
                    contents: bytemuck::cast_slice(&prepared.borders),
                    usage: wgpu::BufferUsages::VERTEX,
                })
        });
        let text_instances = prepared
            .texts
            .iter()
            .map(|command| command.instance)
            .collect::<Vec<_>>();
        let text_buffer = (!text_instances.is_empty()).then(|| {
            self.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("cssimpler gpu texts"),
                    contents: bytemuck::cast_slice(&text_instances),
                    usage: wgpu::BufferUsages::VERTEX,
                })
        });
        let color_texture_instances = prepared
            .color_textures
            .iter()
            .map(|command| command.instance)
            .collect::<Vec<_>>();
        let color_texture_buffer = (!color_texture_instances.is_empty()).then(|| {
            self.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("cssimpler gpu color textures"),
                    contents: bytemuck::cast_slice(&color_texture_instances),
                    usage: wgpu::BufferUsages::VERTEX,
                })
        });
        let projected_texture_instances = prepared
            .projected_textures
            .iter()
            .map(|command| command.instance)
            .collect::<Vec<_>>();
        let projected_texture_buffer = (!projected_texture_instances.is_empty()).then(|| {
            self.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("cssimpler gpu projected textures"),
                    contents: bytemuck::cast_slice(&projected_texture_instances),
                    usage: wgpu::BufferUsages::VERTEX,
                })
        });

        let Some(surface_texture) = self.acquire_surface_texture()? else {
            return Ok(GpuPresentOutcome::Skipped);
        };
        let view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("cssimpler gpu encoder"),
            });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("cssimpler gpu pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(color_to_wgpu(clear_color, self.srgb_surface)),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
            });
            render_pass.set_bind_group(0, &self.viewport_bind_group, &[]);
            let mut command_index = 0_usize;
            while let Some(command) = prepared.commands.get(command_index) {
                match command {
                    GpuPaintCommand::Fill {
                        scissor,
                        instance_start,
                        instance_count,
                    } => {
                        let Some(fill_buffer) = fill_buffer.as_ref() else {
                            command_index += 1;
                            continue;
                        };
                        apply_scissor(&mut render_pass, *scissor);
                        render_pass.set_pipeline(&self.fill_pipeline);
                        render_pass.set_vertex_buffer(0, fill_buffer.slice(..));
                        render_pass.draw(0..6, *instance_start..*instance_start + *instance_count);
                    }
                    GpuPaintCommand::Border {
                        scissor,
                        instance_start,
                        instance_count,
                    } => {
                        let Some(border_buffer) = border_buffer.as_ref() else {
                            command_index += 1;
                            continue;
                        };
                        apply_scissor(&mut render_pass, *scissor);
                        render_pass.set_pipeline(&self.border_pipeline);
                        render_pass.set_vertex_buffer(0, border_buffer.slice(..));
                        render_pass.draw(0..6, *instance_start..*instance_start + *instance_count);
                    }
                    GpuPaintCommand::Text {
                        scissor,
                        instance_start,
                        instance_count,
                    } => {
                        let Some(text_buffer) = text_buffer.as_ref() else {
                            command_index += 1;
                            continue;
                        };
                        let Some(text_command) = prepared.texts.get(*instance_start as usize)
                        else {
                            command_index += 1;
                            continue;
                        };
                        let Some(texture) = self.text_textures.get_mut(&text_command.key) else {
                            command_index += 1;
                            continue;
                        };
                        texture.last_used = next_texture_use(&mut self.next_texture_use);
                        apply_scissor(&mut render_pass, *scissor);
                        render_pass.set_pipeline(&self.text_pipeline);
                        render_pass.set_bind_group(1, &texture.bind_group, &[]);
                        render_pass.set_vertex_buffer(0, text_buffer.slice(..));
                        render_pass.draw(0..6, *instance_start..*instance_start + *instance_count);
                    }
                    GpuPaintCommand::ColorTexture {
                        scissor,
                        instance_start,
                        instance_count,
                    } => {
                        let Some(color_texture_buffer) = color_texture_buffer.as_ref() else {
                            command_index += 1;
                            continue;
                        };
                        let Some(texture_command) =
                            prepared.color_textures.get(*instance_start as usize)
                        else {
                            command_index += 1;
                            continue;
                        };
                        let Some(texture) = self.color_textures.get_mut(&texture_command.key)
                        else {
                            command_index += 1;
                            continue;
                        };
                        texture.last_used = next_texture_use(&mut self.next_texture_use);
                        apply_scissor(&mut render_pass, *scissor);
                        render_pass.set_pipeline(&self.color_texture_pipeline);
                        render_pass.set_bind_group(1, &texture.bind_group, &[]);
                        render_pass.set_vertex_buffer(0, color_texture_buffer.slice(..));
                        render_pass.draw(0..6, *instance_start..*instance_start + *instance_count);
                    }
                    GpuPaintCommand::ProjectedTexture {
                        scissor,
                        instance_start,
                        instance_count,
                    } => {
                        let Some(projected_texture_buffer) = projected_texture_buffer.as_ref()
                        else {
                            command_index += 1;
                            continue;
                        };
                        let Some(texture_command) =
                            prepared.projected_textures.get(*instance_start as usize)
                        else {
                            command_index += 1;
                            continue;
                        };
                        let Some(texture) = self.color_textures.get_mut(&texture_command.key)
                        else {
                            command_index += 1;
                            continue;
                        };
                        texture.last_used = next_texture_use(&mut self.next_texture_use);
                        apply_scissor(&mut render_pass, *scissor);
                        render_pass.set_pipeline(&self.projected_texture_pipeline);
                        render_pass.set_bind_group(1, &texture.bind_group, &[]);
                        render_pass.set_vertex_buffer(0, projected_texture_buffer.slice(..));
                        render_pass.draw(0..6, *instance_start..*instance_start + *instance_count);
                    }
                }
                command_index += 1;
            }
        }

        self.queue.submit(Some(encoder.finish()));
        let paint_us = paint_start.elapsed().as_micros() as u64;
        let present_start = Instant::now();
        surface_texture.present();
        Ok(GpuPresentOutcome::Presented {
            paint_us,
            present_us: present_start.elapsed().as_micros() as u64,
        })
    }

    fn acquire_surface_texture(&mut self) -> Result<Option<wgpu::SurfaceTexture>> {
        let Some(surface) = self.surface.as_ref() else {
            return Ok(None);
        };

        match surface.get_current_texture() {
            Ok(texture) => Ok(Some(texture)),
            Err(wgpu::SurfaceError::Outdated | wgpu::SurfaceError::Lost) => {
                self.configure_surface(self.surface_size.0.max(1), self.surface_size.1.max(1));
                let Some(surface) = self.surface.as_ref() else {
                    return Ok(None);
                };
                match surface.get_current_texture() {
                    Ok(texture) => Ok(Some(texture)),
                    Err(wgpu::SurfaceError::Timeout) => Ok(None),
                    Err(error) => Err(RendererError::Backend(format!(
                        "gpu surface acquire failed after reconfigure: {error}"
                    ))),
                }
            }
            Err(wgpu::SurfaceError::Timeout) => Ok(None),
            Err(error) => Err(RendererError::Backend(format!(
                "gpu surface acquire failed: {error}"
            ))),
        }
    }

    fn configure_surface(&mut self, width: u32, height: u32) {
        self.surface_config.width = width.max(1);
        self.surface_config.height = height.max(1);
        self.surface_size = (self.surface_config.width, self.surface_config.height);
        self.queue.write_buffer(
            &self.viewport_buffer,
            0,
            bytemuck::bytes_of(&ViewportUniform {
                size: [self.surface_size.0 as f32, self.surface_size.1 as f32],
                _padding: [0.0, 0.0],
            }),
        );
        if let Some(surface) = self.surface.as_ref() {
            surface.configure(&self.device, &self.surface_config);
        }
    }

    fn ensure_text_texture(&mut self, key: &TextTextureKey, mask: &RasterizedTextMask) {
        let last_used = next_texture_use(&mut self.next_texture_use);
        if let Some(texture) = self.text_textures.get_mut(key) {
            texture.last_used = last_used;
            return;
        }

        while self.text_textures.len() >= MAX_TEXT_TEXTURE_CACHE_ENTRIES {
            evict_lru_text_texture(&mut self.text_textures);
        }

        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("cssimpler gpu text texture"),
            size: wgpu::Extent3d {
                width: mask.width().max(1) as u32,
                height: mask.height().max(1) as u32,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            mask.alpha(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(mask.width().max(1) as u32),
                rows_per_image: Some(mask.height().max(1) as u32),
            },
            wgpu::Extent3d {
                width: mask.width().max(1) as u32,
                height: mask.height().max(1) as u32,
                depth_or_array_layers: 1,
            },
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cssimpler gpu text bind group"),
            layout: &self.sampled_texture_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.text_sampler),
                },
            ],
        });
        self.text_textures.insert(
            key.clone(),
            CachedTextTexture {
                bind_group,
                last_used,
            },
        );
    }

    fn ensure_color_texture(
        &mut self,
        key: &ColorTextureKey,
        raster: Option<&RasterizedColorTexture>,
    ) {
        let last_used = next_texture_use(&mut self.next_texture_use);
        if let Some(texture) = self.color_textures.get_mut(key) {
            texture.last_used = last_used;
            return;
        }
        let Some(raster) = raster else {
            return;
        };

        while self.color_textures.len() >= MAX_COLOR_TEXTURE_CACHE_ENTRIES {
            evict_lru_color_texture(&mut self.color_textures);
        }

        let texture = self.create_color_texture(raster, last_used);
        self.color_textures.insert(*key, texture);
    }

    fn create_color_texture(
        &self,
        raster: &RasterizedColorTexture,
        last_used: u64,
    ) -> CachedColorTexture {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("cssimpler gpu color texture"),
            size: wgpu::Extent3d {
                width: raster.width.max(1) as u32,
                height: raster.height.max(1) as u32,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let texels = premultiplied_linear_rgba_texels(&raster.pixels);
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &texels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(raster.width.max(1).saturating_mul(4) as u32),
                rows_per_image: Some(raster.height.max(1) as u32),
            },
            wgpu::Extent3d {
                width: raster.width.max(1) as u32,
                height: raster.height.max(1) as u32,
                depth_or_array_layers: 1,
            },
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cssimpler gpu color texture bind group"),
            layout: &self.sampled_texture_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.color_sampler),
                },
            ],
        });
        CachedColorTexture {
            bind_group,
            last_used,
        }
    }
}

fn prepare_gpu_scene(
    scene: &ExtractedScene,
    viewport_width: u32,
    viewport_height: u32,
    srgb_surface: bool,
    cached_color_textures: &HashMap<ColorTextureKey, CachedColorTexture>,
) -> std::result::Result<PreparedGpuScene, String> {
    let mut commands = Vec::new();
    let mut fills = Vec::new();
    let mut borders = Vec::new();
    let mut texts = Vec::new();
    let mut color_textures = Vec::new();
    let mut projected_textures = Vec::new();
    let mut paint_item_indices = HashMap::new();
    let mut pending_projected = collect_projected_surface_commands(
        &scene.roots,
        viewport_width,
        viewport_height,
        cached_color_textures,
    );
    for projected in &mut pending_projected {
        projected.sort_key = projected_sort_key(scene, &projected.path);
    }
    pending_projected.sort_by_key(|command| command.sort_key);
    let projected_paths = pending_projected
        .iter()
        .map(|command| command.path.clone())
        .collect::<Vec<_>>();
    let mut pending_projected_index = 0_usize;
    let mut prepared_color_texture_keys = HashSet::new();

    for item in &scene.items {
        while let Some(projected) = pending_projected.get(pending_projected_index) {
            if projected.sort_key > item.stable_sort_key {
                break;
            }
            let instance_index = projected_textures.len() as u32;
            projected_textures.push(PreparedProjectedTextureCommand {
                instance: projected.command.instance,
                key: projected.command.key,
                raster: projected.command.raster.clone(),
            });
            push_projected_texture_command(
                &mut commands,
                &projected_textures,
                projected.scissor,
                instance_index,
            );
            pending_projected_index += 1;
        }
        if path_is_under_projected_subtree(&item.path, &projected_paths) {
            continue;
        }
        if let Some(reason) = unsupported_gpu_reason(item) {
            return Err(reason);
        }
        let Some(scissor) = scissor_rect(item.clip, viewport_width, viewport_height) else {
            continue;
        };

        let paint_item_index = paint_item_sequence_index(&mut paint_item_indices, item);

        match item.kind {
            ExtractedPaintKind::Background => {
                let fill_layout = if item.style.border.widths.is_zero() {
                    item.layout
                } else {
                    inset_layout(item.layout, item.style.border.widths)
                };
                let fill_radius = if item.style.border.widths.is_zero() {
                    item.style.corner_radius
                } else {
                    inset_corner_radius(
                        item.layout,
                        item.style.corner_radius,
                        item.style.border.widths,
                    )
                };
                if let Some(color) = item.style.background
                    && color.a > 0
                    && fill_layout.width > 0.0
                    && fill_layout.height > 0.0
                {
                    let instance_index = fills.len() as u32;
                    fills.push(FillInstance {
                        rect: layout_rect(fill_layout),
                        radii: corner_radii(fill_layout, fill_radius),
                        color: packed_color(color, srgb_surface),
                    });
                    push_fill_command(&mut commands, scissor, instance_index);
                }
                for layer in item.style.background_layers.iter().rev() {
                    let key = background_layer_texture_key(fill_layout, fill_radius, layer);
                    let raster = if cached_color_textures.contains_key(&key)
                        || !prepared_color_texture_keys.insert(key)
                    {
                        None
                    } else {
                        rasterize_background_layer_texture_uncached(fill_layout, fill_radius, layer)
                    };
                    let instance_index = color_textures.len() as u32;
                    let instance = raster
                        .as_ref()
                        .map(texture_instance_from_raster)
                        .unwrap_or_else(|| texture_instance_from_layout(fill_layout));
                    color_textures.push(PreparedColorTextureCommand {
                        instance,
                        key,
                        raster,
                    });
                    push_color_texture_command(
                        &mut commands,
                        &color_textures,
                        scissor,
                        instance_index,
                    );
                }
            }
            ExtractedPaintKind::Border => {
                if item.style.border.widths.is_zero()
                    || item.style.border.color.a == 0
                    || item.layout.width <= 0.0
                    || item.layout.height <= 0.0
                {
                    continue;
                }
                let inner_rect = inset_layout(item.layout, item.style.border.widths);
                let instance = BorderInstance {
                    outer_rect: layout_rect(item.layout),
                    inner_rect: layout_rect(inner_rect),
                    outer_radii: corner_radii(
                        item.layout,
                        inset_corner_radius(item.layout, item.style.corner_radius, Insets::ZERO),
                    ),
                    inner_radii: corner_radii(
                        inner_rect,
                        inset_corner_radius(
                            item.layout,
                            item.style.corner_radius,
                            item.style.border.widths,
                        ),
                    ),
                    color: packed_color(item.style.border.color, srgb_surface),
                };
                let instance_index = borders.len() as u32;
                borders.push(instance);
                push_border_command(&mut commands, scissor, instance_index);
            }
            ExtractedPaintKind::TextRun => {
                let Some(text) = item.text.as_deref() else {
                    continue;
                };
                if text.is_empty() {
                    continue;
                }
                let text_layout = text_paint_layout(item);
                if text_layout.width <= 0.0 || text_layout.height <= 0.0 {
                    continue;
                }
                let Some(text_scissor) =
                    scissor_rect(text_clip_layout(item), viewport_width, viewport_height)
                else {
                    continue;
                };
                if text_requires_color_texture(item) {
                    let Some(raster) = rasterize_text_color_texture(
                        text_layout,
                        text,
                        item.text_layout.as_ref(),
                        &item.style,
                    ) else {
                        continue;
                    };
                    if raster.width == 0 || raster.height == 0 {
                        continue;
                    }
                    let instance_index = color_textures.len() as u32;
                    color_textures.push(PreparedColorTextureCommand {
                        instance: texture_instance_from_raster(&raster),
                        key: text_effect_texture_key(text, item, text_layout),
                        raster: Some(raster),
                    });
                    push_color_texture_command(
                        &mut commands,
                        &color_textures,
                        text_scissor,
                        instance_index,
                    );
                } else {
                    if item.style.foreground.a == 0 {
                        continue;
                    }
                    let Some(mask) = rasterize_text_mask(
                        text_layout,
                        text,
                        item.text_layout.as_ref(),
                        &item.style,
                    ) else {
                        continue;
                    };
                    if mask.width() == 0 || mask.height() == 0 {
                        continue;
                    }
                    let instance_index = texts.len() as u32;
                    texts.push(PreparedTextCommand {
                        instance: TextInstance {
                            rect: [
                                mask.origin_x() as f32,
                                mask.origin_y() as f32,
                                mask.width() as f32,
                                mask.height() as f32,
                            ],
                            color: packed_color(item.style.foreground, srgb_surface),
                        },
                        key: TextTextureKey::from_item(text, item, text_layout),
                        mask,
                    });
                    push_text_command(&mut commands, &texts, text_scissor, instance_index);
                }
            }
            ExtractedPaintKind::BoxShadow => {
                let Some(shadow) = item.style.shadows.get(paint_item_index).copied() else {
                    continue;
                };
                let Some(bounds) = shadow_bounds(item.layout, shadow) else {
                    continue;
                };
                let key = shadow_texture_key(item.layout, item.style.corner_radius, shadow);
                let raster = if cached_color_textures.contains_key(&key)
                    || !prepared_color_texture_keys.insert(key)
                {
                    None
                } else {
                    rasterize_shadow_texture_uncached(item.layout, item.style.corner_radius, shadow)
                };
                let instance_index = color_textures.len() as u32;
                color_textures.push(PreparedColorTextureCommand {
                    instance: texture_instance_from_clip(bounds),
                    key,
                    raster,
                });
                push_color_texture_command(&mut commands, &color_textures, scissor, instance_index);
            }
            ExtractedPaintKind::FilterDropShadow => {
                let Some(shadow) = item
                    .style
                    .filter_drop_shadows
                    .get(paint_item_index)
                    .copied()
                else {
                    continue;
                };
                let Some(bounds) = shadow_effect_bounds(item.layout, shadow) else {
                    continue;
                };
                let box_shadow = BoxShadow {
                    color: shadow.color.unwrap_or(item.style.foreground),
                    offset_x: shadow.offset_x,
                    offset_y: shadow.offset_y,
                    blur_radius: shadow.blur_radius,
                    spread: shadow.spread,
                };
                let key = shadow_texture_key(item.layout, item.style.corner_radius, box_shadow);
                let raster = if cached_color_textures.contains_key(&key)
                    || !prepared_color_texture_keys.insert(key)
                {
                    None
                } else {
                    rasterize_shadow_texture_uncached(
                        item.layout,
                        item.style.corner_radius,
                        box_shadow,
                    )
                };
                let instance_index = color_textures.len() as u32;
                color_textures.push(PreparedColorTextureCommand {
                    instance: texture_instance_from_clip(bounds),
                    key,
                    raster,
                });
                push_color_texture_command(&mut commands, &color_textures, scissor, instance_index);
            }
            ExtractedPaintKind::Svg => {
                let Some(svg_scene) = item.svg_scene.as_ref() else {
                    continue;
                };
                let key = svg_texture_key(item.layout, svg_scene);
                let raster = if cached_color_textures.contains_key(&key)
                    || !prepared_color_texture_keys.insert(key)
                {
                    None
                } else {
                    rasterize_svg_scene_texture(item.layout, svg_scene)
                };
                let instance_index = color_textures.len() as u32;
                color_textures.push(PreparedColorTextureCommand {
                    instance: texture_instance_from_layout(item.layout),
                    key,
                    raster,
                });
                push_color_texture_command(&mut commands, &color_textures, scissor, instance_index);
            }
            ExtractedPaintKind::BackdropBlur | ExtractedPaintKind::Scrollbar => {}
        }
    }

    while let Some(projected) = pending_projected.get(pending_projected_index) {
        let instance_index = projected_textures.len() as u32;
        projected_textures.push(PreparedProjectedTextureCommand {
            instance: projected.command.instance,
            key: projected.command.key,
            raster: projected.command.raster.clone(),
        });
        push_projected_texture_command(
            &mut commands,
            &projected_textures,
            projected.scissor,
            instance_index,
        );
        pending_projected_index += 1;
    }

    Ok(PreparedGpuScene {
        commands,
        fills,
        borders,
        texts,
        color_textures,
        projected_textures,
    })
}

fn collect_projected_surface_commands(
    roots: &[RenderNode],
    viewport_width: u32,
    viewport_height: u32,
    cached_color_textures: &HashMap<ColorTextureKey, CachedColorTexture>,
) -> Vec<PendingProjectedTextureCommand> {
    let mut commands = Vec::new();
    for (root_index, root) in roots.iter().enumerate() {
        collect_projected_surface_commands_for_node(
            root,
            vec![root_index],
            None,
            TransformMatrix3d::IDENTITY,
            None,
            false,
            viewport_width,
            viewport_height,
            cached_color_textures,
            &mut commands,
        );
    }
    commands
}

fn collect_projected_surface_commands_for_node(
    node: &RenderNode,
    path: Vec<usize>,
    inherited_clip: Option<LayoutBox>,
    parent_world_matrix: TransformMatrix3d,
    parent_perspective: Option<PerspectiveContext>,
    in_3d_context: bool,
    viewport_width: u32,
    viewport_height: u32,
    cached_color_textures: &HashMap<ColorTextureKey, CachedColorTexture>,
    commands: &mut Vec<PendingProjectedTextureCommand>,
) {
    let clip = combine_layout_clips(
        inherited_clip,
        node.style.overflow.clips_any_axis().then_some(node.layout),
    );
    let world_matrix = parent_world_matrix.multiply(node_local_transform_matrix(
        node.layout,
        &node.style.transform,
    ));

    if crate::node_requires_projected_path(node) {
        let Some(matrix) =
            project_world_transform_matrix(node.layout, world_matrix, parent_perspective)
        else {
            return;
        };
        let Some(surface) =
            crate::cached_promoted_surface(node, viewport_width as usize, viewport_height as usize)
        else {
            return;
        };
        let Some(projected_bounds) =
            crate::transform::transform_clip_rect(surface.source_bounds, matrix)
        else {
            return;
        };
        let Some(scissor) = scissor_rect_from_clip_rect(
            clip_rect_from_layout_box(clip).unwrap_or_else(|| {
                crate::ClipRect::full(viewport_width as f32, viewport_height as f32)
            }),
            projected_bounds,
            viewport_width,
            viewport_height,
        ) else {
            return;
        };
        let key = projected_surface_texture_key(node);
        let raster = if cached_color_textures.contains_key(&key) {
            None
        } else {
            Some(rasterized_color_texture_from_surface(surface.as_ref()))
        };
        let Some(inverse) = matrix.invert() else {
            return;
        };
        commands.push(PendingProjectedTextureCommand {
            sort_key: 0,
            path,
            scissor,
            command: PreparedProjectedTextureCommand {
                instance: projected_texture_instance(surface.as_ref(), projected_bounds, inverse),
                key,
                raster,
            },
        });
        return;
    }

    let child_context = crate::child_3d_context(node, in_3d_context);
    let child_perspective =
        crate::active_child_perspective(node, parent_perspective, child_context);
    let child_entries = if child_context || node.style.perspective.is_some() {
        crate::projected_children(
            node,
            world_matrix,
            child_perspective,
            crate::node_sorts_projected_children(node, child_context),
        )
        .into_iter()
        .map(|entry| entry.index)
        .collect::<Vec<_>>()
    } else {
        (0..node.children.len()).collect::<Vec<_>>()
    };
    for child_index in child_entries {
        let child = &node.children[child_index];
        let mut child_path = path.clone();
        child_path.push(child_index);
        collect_projected_surface_commands_for_node(
            child,
            child_path,
            clip,
            world_matrix,
            child_perspective,
            child_context,
            viewport_width,
            viewport_height,
            cached_color_textures,
            commands,
        );
    }
}

fn path_is_under_projected_subtree(path: &[usize], projected_paths: &[Vec<usize>]) -> bool {
    projected_paths
        .iter()
        .any(|projected_path| path.starts_with(projected_path))
}

fn combine_layout_clips(left: Option<LayoutBox>, right: Option<LayoutBox>) -> Option<LayoutBox> {
    match (left, right) {
        (Some(left), Some(right)) => {
            let x0 = left.x.max(right.x);
            let y0 = left.y.max(right.y);
            let x1 = (left.x + left.width).min(right.x + right.width);
            let y1 = (left.y + left.height).min(right.y + right.height);
            Some(LayoutBox::new(
                x0,
                y0,
                (x1 - x0).max(0.0),
                (y1 - y0).max(0.0),
            ))
        }
        (Some(clip), None) | (None, Some(clip)) => Some(clip),
        (None, None) => None,
    }
}

fn clip_rect_from_layout_box(layout: Option<LayoutBox>) -> Option<crate::ClipRect> {
    layout.map(|layout| crate::ClipRect {
        x0: layout.x,
        y0: layout.y,
        x1: layout.x + layout.width,
        y1: layout.y + layout.height,
    })
}

fn scissor_rect_from_clip_rect(
    coarse_clip: crate::ClipRect,
    projected_bounds: crate::ClipRect,
    viewport_width: u32,
    viewport_height: u32,
) -> Option<ScissorRect> {
    let clip = coarse_clip.intersect(projected_bounds)?;
    let x0 = clip.x0.floor().clamp(0.0, viewport_width as f32) as u32;
    let y0 = clip.y0.floor().clamp(0.0, viewport_height as f32) as u32;
    let x1 = clip.x1.ceil().clamp(0.0, viewport_width as f32) as u32;
    let y1 = clip.y1.ceil().clamp(0.0, viewport_height as f32) as u32;
    let width = x1.saturating_sub(x0);
    let height = y1.saturating_sub(y0);
    (width > 0 && height > 0).then_some(ScissorRect {
        x: x0,
        y: y0,
        width,
        height,
    })
}

fn projected_sort_key(scene: &ExtractedScene, projected_path: &[usize]) -> u64 {
    scene
        .items
        .iter()
        .find(|item| compare_tree_entry_path(projected_path, &item.path) != Ordering::Greater)
        .map(|item| item.stable_sort_key)
        .unwrap_or(u64::MAX)
}

fn compare_tree_entry_path(entry_path: &[usize], item_path: &[usize]) -> Ordering {
    for (entry_segment, item_segment) in entry_path.iter().zip(item_path.iter()) {
        let ordering = entry_segment.cmp(item_segment);
        if ordering != Ordering::Equal {
            return ordering;
        }
    }

    entry_path.len().cmp(&item_path.len())
}

fn projected_surface_texture_key(node: &RenderNode) -> ColorTextureKey {
    color_texture_key(4, |state| {
        crate::hash_surface_subtree(&crate::neutralized_surface_root(node)).hash(state)
    })
}

fn rasterized_color_texture_from_surface(
    surface: &crate::CachedSubtreeSurface,
) -> RasterizedColorTexture {
    RasterizedColorTexture {
        origin_x: surface.source_bounds.x0 as i32,
        origin_y: surface.source_bounds.y0 as i32,
        width: surface.width,
        height: surface.height,
        pixels: surface.pixels.clone(),
    }
}

fn projected_texture_instance(
    surface: &crate::CachedSubtreeSurface,
    projected_bounds: crate::ClipRect,
    inverse: AffineTransform,
) -> ProjectedTextureInstance {
    ProjectedTextureInstance {
        screen_rect: [
            projected_bounds.x0,
            projected_bounds.y0,
            projected_bounds.x1 - projected_bounds.x0,
            projected_bounds.y1 - projected_bounds.y0,
        ],
        source_rect: [
            surface.source_bounds.x0,
            surface.source_bounds.y0,
            surface.source_bounds.x1 - surface.source_bounds.x0,
            surface.source_bounds.y1 - surface.source_bounds.y0,
        ],
        inverse_row0: [inverse.a, inverse.c, inverse.e, 0.0],
        inverse_row1: [inverse.b, inverse.d, inverse.f, 0.0],
        inverse_row2: [inverse.g, inverse.h, inverse.i, 0.0],
    }
}

fn unsupported_gpu_reason(item: &ExtractedPaintItem) -> Option<String> {
    let capabilities = BackendCapabilities::GPU_BASELINE;
    if !item.style.transform.is_identity()
        || item.style.perspective.is_some()
        || matches!(item.style.transform_style, TransformStyleMode::Preserve3d)
    {
        if !capabilities.transforms {
            return Some("transforms are not supported by the GPU backend yet".to_string());
        }
    }

    match item.kind {
        ExtractedPaintKind::Background
            if !item.style.background_layers.is_empty() && !capabilities.background_layers =>
        {
            Some("background gradients are not supported by the GPU backend yet".to_string())
        }
        ExtractedPaintKind::BackdropBlur if !capabilities.backdrop_blur => {
            Some("backdrop blur is not supported by the GPU backend yet".to_string())
        }
        ExtractedPaintKind::BoxShadow if !capabilities.shadows => {
            Some("box shadows are not supported by the GPU backend yet".to_string())
        }
        ExtractedPaintKind::FilterDropShadow if !capabilities.filter_effects => {
            Some("filter drop shadows are not supported by the GPU backend yet".to_string())
        }
        ExtractedPaintKind::Svg if !capabilities.svg => {
            Some("SVG paint items are not supported by the GPU backend yet".to_string())
        }
        ExtractedPaintKind::TextRun
            if text_requires_color_texture(item) && !capabilities.text_effects =>
        {
            Some("text effects are not supported by the GPU backend yet".to_string())
        }
        ExtractedPaintKind::Scrollbar if !capabilities.scrollbars => {
            Some("scrollbar paint items are not supported by the GPU backend yet".to_string())
        }
        _ => None,
    }
}

fn scissor_rect(
    clip: Option<LayoutBox>,
    viewport_width: u32,
    viewport_height: u32,
) -> Option<ScissorRect> {
    let clip = clip.unwrap_or(LayoutBox::new(
        0.0,
        0.0,
        viewport_width as f32,
        viewport_height as f32,
    ));
    let x0 = clip.x.floor().clamp(0.0, viewport_width as f32) as u32;
    let y0 = clip.y.floor().clamp(0.0, viewport_height as f32) as u32;
    let x1 = (clip.x + clip.width)
        .ceil()
        .clamp(0.0, viewport_width as f32) as u32;
    let y1 = (clip.y + clip.height)
        .ceil()
        .clamp(0.0, viewport_height as f32) as u32;
    let width = x1.saturating_sub(x0);
    let height = y1.saturating_sub(y0);
    (width > 0 && height > 0).then_some(ScissorRect {
        x: x0,
        y: y0,
        width,
        height,
    })
}

fn text_paint_layout(item: &ExtractedPaintItem) -> LayoutBox {
    let mut layout = inset_layout(item.layout, item.content_inset);
    if let Some(scrollbars) = item.scrollbars {
        layout.width = (layout.width + scrollbars.metrics.max_offset_x).max(0.0);
        layout.height = (layout.height + scrollbars.metrics.max_offset_y).max(0.0);
        layout = offset_layout(
            layout,
            -scrollbars.metrics.offset_x,
            -scrollbars.metrics.offset_y,
        );
    }
    layout
}

fn text_viewport_layout(item: &ExtractedPaintItem) -> LayoutBox {
    let mut viewport = inset_layout(item.layout, item.content_inset);
    if let Some(scrollbars) = item.scrollbars {
        viewport.width = (viewport.width - scrollbars.metrics.reserved_width).max(0.0);
        viewport.height = (viewport.height - scrollbars.metrics.reserved_height).max(0.0);
    }
    viewport
}

fn text_clip_layout(item: &ExtractedPaintItem) -> Option<LayoutBox> {
    if item.style.overflow.clips_any_axis() || item.scrollbars.is_some() {
        combine_layout_clips(item.clip, Some(text_viewport_layout(item)))
    } else {
        item.clip
    }
}

fn create_fill_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    surface_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("cssimpler gpu fill pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("fill_vs"),
            buffers: &[wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<FillInstance>() as wgpu::BufferAddress,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x4],
            }],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fill_fs"),
            targets: &[Some(color_target_state(surface_format))],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
        cache: None,
    })
}

fn create_border_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    surface_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("cssimpler gpu border pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("border_vs"),
            buffers: &[wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<BorderInstance>() as wgpu::BufferAddress,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &wgpu::vertex_attr_array![
                    0 => Float32x4,
                    1 => Float32x4,
                    2 => Float32x4,
                    3 => Float32x4,
                    4 => Float32x4
                ],
            }],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("border_fs"),
            targets: &[Some(color_target_state(surface_format))],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
        cache: None,
    })
}

fn create_text_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    surface_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("cssimpler gpu text pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("text_vs"),
            buffers: &[wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<TextInstance>() as wgpu::BufferAddress,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4],
            }],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("text_fs"),
            targets: &[Some(color_target_state(surface_format))],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
        cache: None,
    })
}

fn create_color_texture_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    surface_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("cssimpler gpu color texture pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("texture_vs"),
            buffers: &[wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<TextureInstance>() as wgpu::BufferAddress,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &wgpu::vertex_attr_array![0 => Float32x4],
            }],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("texture_fs"),
            targets: &[Some(color_texture_target_state(surface_format))],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
        cache: None,
    })
}

fn create_projected_texture_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    surface_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("cssimpler gpu projected texture pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("projected_texture_vs"),
            buffers: &[wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<ProjectedTextureInstance>()
                    as wgpu::BufferAddress,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &wgpu::vertex_attr_array![
                    0 => Float32x4,
                    1 => Float32x4,
                    2 => Float32x4,
                    3 => Float32x4,
                    4 => Float32x4
                ],
            }],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("projected_texture_fs"),
            targets: &[Some(color_texture_target_state(surface_format))],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
        cache: None,
    })
}

fn color_target_state(surface_format: wgpu::TextureFormat) -> wgpu::ColorTargetState {
    wgpu::ColorTargetState {
        format: surface_format,
        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
        write_mask: wgpu::ColorWrites::ALL,
    }
}

fn color_texture_target_state(surface_format: wgpu::TextureFormat) -> wgpu::ColorTargetState {
    wgpu::ColorTargetState {
        format: surface_format,
        blend: Some(wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
        }),
        write_mask: wgpu::ColorWrites::ALL,
    }
}

fn apply_scissor(render_pass: &mut wgpu::RenderPass<'_>, scissor: ScissorRect) {
    render_pass.set_scissor_rect(scissor.x, scissor.y, scissor.width, scissor.height);
}

fn texture_instance_from_layout(layout: LayoutBox) -> TextureInstance {
    let origin_x = layout.x.floor();
    let origin_y = layout.y.floor();
    TextureInstance {
        rect: [
            origin_x,
            origin_y,
            ((layout.x + layout.width).ceil() - origin_x).max(0.0),
            ((layout.y + layout.height).ceil() - origin_y).max(0.0),
        ],
    }
}

fn texture_instance_from_clip(clip: crate::ClipRect) -> TextureInstance {
    TextureInstance {
        rect: [
            clip.x0.floor(),
            clip.y0.floor(),
            (clip.x1.ceil() - clip.x0.floor()).max(0.0),
            (clip.y1.ceil() - clip.y0.floor()).max(0.0),
        ],
    }
}

fn texture_instance_from_raster(raster: &RasterizedColorTexture) -> TextureInstance {
    TextureInstance {
        rect: [
            raster.origin_x as f32,
            raster.origin_y as f32,
            raster.width as f32,
            raster.height as f32,
        ],
    }
}

fn text_requires_color_texture(item: &ExtractedPaintItem) -> bool {
    !item.style.text_shadows.is_empty()
        || !item.style.filter_drop_shadows.is_empty()
        || item.style.text_stroke.width > 0.0
}

fn paint_item_sequence_index(
    indices: &mut HashMap<(Vec<usize>, ExtractedPaintKind), usize>,
    item: &ExtractedPaintItem,
) -> usize {
    let entry = indices.entry((item.path.clone(), item.kind)).or_insert(0);
    let index = *entry;
    *entry = entry.saturating_add(1);
    index
}

fn push_fill_command(
    commands: &mut Vec<GpuPaintCommand>,
    scissor: ScissorRect,
    instance_index: u32,
) {
    if let Some(GpuPaintCommand::Fill {
        scissor: previous_scissor,
        instance_start,
        instance_count,
    }) = commands.last_mut()
        && *previous_scissor == scissor
        && *instance_start + *instance_count == instance_index
    {
        *instance_count += 1;
        return;
    }

    commands.push(GpuPaintCommand::Fill {
        scissor,
        instance_start: instance_index,
        instance_count: 1,
    });
}

fn push_border_command(
    commands: &mut Vec<GpuPaintCommand>,
    scissor: ScissorRect,
    instance_index: u32,
) {
    if let Some(GpuPaintCommand::Border {
        scissor: previous_scissor,
        instance_start,
        instance_count,
    }) = commands.last_mut()
        && *previous_scissor == scissor
        && *instance_start + *instance_count == instance_index
    {
        *instance_count += 1;
        return;
    }

    commands.push(GpuPaintCommand::Border {
        scissor,
        instance_start: instance_index,
        instance_count: 1,
    });
}

fn push_text_command(
    commands: &mut Vec<GpuPaintCommand>,
    texts: &[PreparedTextCommand],
    scissor: ScissorRect,
    instance_index: u32,
) {
    if let Some(GpuPaintCommand::Text {
        scissor: previous_scissor,
        instance_start,
        instance_count,
    }) = commands.last_mut()
        && *previous_scissor == scissor
        && *instance_start + *instance_count == instance_index
        && texts
            .get(*instance_start as usize)
            .is_some_and(|previous| previous.key == texts[instance_index as usize].key)
    {
        *instance_count += 1;
        return;
    }

    commands.push(GpuPaintCommand::Text {
        scissor,
        instance_start: instance_index,
        instance_count: 1,
    });
}

fn push_color_texture_command(
    commands: &mut Vec<GpuPaintCommand>,
    color_textures: &[PreparedColorTextureCommand],
    scissor: ScissorRect,
    instance_index: u32,
) {
    if let Some(GpuPaintCommand::ColorTexture {
        scissor: previous_scissor,
        instance_start,
        instance_count,
    }) = commands.last_mut()
        && *previous_scissor == scissor
        && *instance_start + *instance_count == instance_index
        && color_textures
            .get(*instance_start as usize)
            .is_some_and(|previous| previous.key == color_textures[instance_index as usize].key)
    {
        *instance_count += 1;
        return;
    }

    commands.push(GpuPaintCommand::ColorTexture {
        scissor,
        instance_start: instance_index,
        instance_count: 1,
    });
}

fn push_projected_texture_command(
    commands: &mut Vec<GpuPaintCommand>,
    projected_textures: &[PreparedProjectedTextureCommand],
    scissor: ScissorRect,
    instance_index: u32,
) {
    if let Some(GpuPaintCommand::ProjectedTexture {
        scissor: previous_scissor,
        instance_start,
        instance_count,
    }) = commands.last_mut()
        && *previous_scissor == scissor
        && *instance_start + *instance_count == instance_index
        && projected_textures
            .get(*instance_start as usize)
            .is_some_and(|previous| previous.key == projected_textures[instance_index as usize].key)
    {
        *instance_count += 1;
        return;
    }

    commands.push(GpuPaintCommand::ProjectedTexture {
        scissor,
        instance_start: instance_index,
        instance_count: 1,
    });
}

fn background_layer_texture_key(
    layout: LayoutBox,
    radius: CornerRadius,
    layer: &BackgroundLayer,
) -> ColorTextureKey {
    let (relative_layout, _, _) = split_layout_translation(layout);
    color_texture_key(0, |state| {
        hash_layout_box(relative_layout, state);
        hash_corner_radius(radius, state);
        hash_background_layer(layer, state);
    })
}

fn shadow_texture_key(
    layout: LayoutBox,
    radius: CornerRadius,
    shadow: BoxShadow,
) -> ColorTextureKey {
    let base_layout = crate::shapes::offset_layout(
        crate::shapes::expand_layout(layout, shadow.spread),
        shadow.offset_x,
        shadow.offset_y,
    );
    let base_radius = crate::shapes::expand_corner_radius(layout, radius, shadow.spread);
    let (relative_layout, _, _) = split_layout_translation(base_layout);
    color_texture_key(1, |state| {
        hash_layout_box(relative_layout, state);
        hash_corner_radius(base_radius, state);
        hash_box_shadow(shadow, state);
    })
}

fn svg_texture_key(layout: LayoutBox, scene: &SvgScene) -> ColorTextureKey {
    let (relative_layout, _, _) = split_layout_translation(layout);
    color_texture_key(2, |state| {
        hash_layout_box(relative_layout, state);
        hash_svg_scene(scene, state);
    })
}

fn text_effect_texture_key(
    text: &str,
    item: &ExtractedPaintItem,
    layout: LayoutBox,
) -> ColorTextureKey {
    let base_key = TextTextureKey::from_item(text, item, layout);
    color_texture_key(3, |state| {
        base_key.hash(state);
        hash_color(item.style.foreground, state);
        hash_text_stroke(
            item.style.text_stroke.width,
            item.style.text_stroke.color,
            state,
        );
        for shadow in item
            .style
            .filter_drop_shadows
            .iter()
            .chain(item.style.text_shadows.iter())
        {
            hash_shadow_effect(*shadow, state);
        }
    })
}

fn color_texture_key(kind: u8, hash: impl FnOnce(&mut DefaultHasher)) -> ColorTextureKey {
    let mut hasher = DefaultHasher::new();
    hash(&mut hasher);
    ColorTextureKey {
        kind,
        fingerprint: hasher.finish(),
    }
}

fn split_layout_translation(layout: LayoutBox) -> (LayoutBox, i32, i32) {
    let offset_x = layout.x.floor() as i32;
    let offset_y = layout.y.floor() as i32;
    (
        LayoutBox::new(
            layout.x - offset_x as f32,
            layout.y - offset_y as f32,
            layout.width,
            layout.height,
        ),
        offset_x,
        offset_y,
    )
}

fn hash_layout_box(layout: LayoutBox, state: &mut impl Hasher) {
    layout.x.to_bits().hash(state);
    layout.y.to_bits().hash(state);
    layout.width.to_bits().hash(state);
    layout.height.to_bits().hash(state);
}

fn hash_corner_radius(radius: CornerRadius, state: &mut impl Hasher) {
    radius.top_left.to_bits().hash(state);
    radius.top_right.to_bits().hash(state);
    radius.bottom_right.to_bits().hash(state);
    radius.bottom_left.to_bits().hash(state);
}

fn hash_color(color: Color, state: &mut impl Hasher) {
    color.r.hash(state);
    color.g.hash(state);
    color.b.hash(state);
    color.a.hash(state);
}

fn hash_background_layer(layer: &BackgroundLayer, state: &mut impl Hasher) {
    match layer {
        BackgroundLayer::LinearGradient(gradient) => {
            0_u8.hash(state);
            hash_gradient_direction(gradient.direction, state);
            hash_gradient_interpolation(gradient.interpolation, state);
            gradient.repeating.hash(state);
            hash_length_gradient_stops(&gradient.stops, state);
        }
        BackgroundLayer::RadialGradient(gradient) => {
            1_u8.hash(state);
            hash_radial_shape(gradient.shape, state);
            hash_gradient_point(gradient.center, state);
            hash_gradient_interpolation(gradient.interpolation, state);
            gradient.repeating.hash(state);
            hash_length_gradient_stops(&gradient.stops, state);
        }
        BackgroundLayer::ConicGradient(gradient) => {
            2_u8.hash(state);
            gradient.angle.to_bits().hash(state);
            hash_gradient_point(gradient.center, state);
            hash_gradient_interpolation(gradient.interpolation, state);
            gradient.repeating.hash(state);
            gradient.stops.len().hash(state);
            for stop in &gradient.stops {
                hash_color(stop.color, state);
                hash_angle_percentage(stop.position, state);
            }
        }
    }
}

fn hash_gradient_direction(direction: GradientDirection, state: &mut impl Hasher) {
    match direction {
        GradientDirection::Angle(angle) => {
            0_u8.hash(state);
            angle.to_bits().hash(state);
        }
        GradientDirection::Horizontal(horizontal) => {
            1_u8.hash(state);
            hash_gradient_horizontal(horizontal, state);
        }
        GradientDirection::Vertical(vertical) => {
            2_u8.hash(state);
            hash_gradient_vertical(vertical, state);
        }
        GradientDirection::Corner {
            horizontal,
            vertical,
        } => {
            3_u8.hash(state);
            hash_gradient_horizontal(horizontal, state);
            hash_gradient_vertical(vertical, state);
        }
    }
}

fn hash_gradient_horizontal(horizontal: GradientHorizontal, state: &mut impl Hasher) {
    match horizontal {
        GradientHorizontal::Left => 0_u8.hash(state),
        GradientHorizontal::Right => 1_u8.hash(state),
    }
}

fn hash_gradient_vertical(vertical: GradientVertical, state: &mut impl Hasher) {
    match vertical {
        GradientVertical::Top => 0_u8.hash(state),
        GradientVertical::Bottom => 1_u8.hash(state),
    }
}

fn hash_gradient_interpolation(interpolation: GradientInterpolation, state: &mut impl Hasher) {
    match interpolation {
        GradientInterpolation::LinearSrgb => 0_u8.hash(state),
        GradientInterpolation::Oklab => 1_u8.hash(state),
    }
}

fn hash_gradient_point(point: GradientPoint, state: &mut impl Hasher) {
    hash_length_percentage(point.x, state);
    hash_length_percentage(point.y, state);
}

fn hash_length_gradient_stops(
    stops: &[GradientStop<LengthPercentageValue>],
    state: &mut impl Hasher,
) {
    stops.len().hash(state);
    for stop in stops {
        hash_color(stop.color, state);
        hash_length_percentage(stop.position, state);
    }
}

fn hash_length_percentage(value: LengthPercentageValue, state: &mut impl Hasher) {
    value.px.to_bits().hash(state);
    value.fraction.to_bits().hash(state);
}

fn hash_angle_percentage(value: cssimpler_core::AnglePercentageValue, state: &mut impl Hasher) {
    value.degrees.to_bits().hash(state);
    value.turns.to_bits().hash(state);
}

fn hash_radial_shape(shape: RadialShape, state: &mut impl Hasher) {
    match shape {
        RadialShape::Circle(radius) => {
            0_u8.hash(state);
            match radius {
                cssimpler_core::CircleRadius::Explicit(value) => {
                    0_u8.hash(state);
                    value.to_bits().hash(state);
                }
                cssimpler_core::CircleRadius::Extent(extent) => {
                    1_u8.hash(state);
                    hash_shape_extent(extent, state);
                }
            }
        }
        RadialShape::Ellipse(radius) => {
            1_u8.hash(state);
            match radius {
                cssimpler_core::EllipseRadius::Explicit { x, y } => {
                    0_u8.hash(state);
                    hash_length_percentage(x, state);
                    hash_length_percentage(y, state);
                }
                cssimpler_core::EllipseRadius::Extent(extent) => {
                    1_u8.hash(state);
                    hash_shape_extent(extent, state);
                }
            }
        }
    }
}

fn hash_shape_extent(extent: ShapeExtent, state: &mut impl Hasher) {
    match extent {
        ShapeExtent::ClosestSide => 0_u8.hash(state),
        ShapeExtent::FarthestSide => 1_u8.hash(state),
        ShapeExtent::ClosestCorner => 2_u8.hash(state),
        ShapeExtent::FarthestCorner => 3_u8.hash(state),
    }
}

fn hash_box_shadow(shadow: BoxShadow, state: &mut impl Hasher) {
    hash_color(shadow.color, state);
    shadow.offset_x.to_bits().hash(state);
    shadow.offset_y.to_bits().hash(state);
    shadow.blur_radius.to_bits().hash(state);
    shadow.spread.to_bits().hash(state);
}

fn hash_shadow_effect(shadow: cssimpler_core::ShadowEffect, state: &mut impl Hasher) {
    shadow.offset_x.to_bits().hash(state);
    shadow.offset_y.to_bits().hash(state);
    shadow.blur_radius.to_bits().hash(state);
    shadow.spread.to_bits().hash(state);
    shadow.color.map(hashable_color).hash(state);
}

fn hashable_color(color: Color) -> (u8, u8, u8, u8) {
    (color.r, color.g, color.b, color.a)
}

fn hash_text_stroke(width: f32, color: Option<Color>, state: &mut impl Hasher) {
    width.to_bits().hash(state);
    color.map(hashable_color).hash(state);
}

fn hash_svg_scene(scene: &SvgScene, state: &mut impl Hasher) {
    scene.view_box.min_x.to_bits().hash(state);
    scene.view_box.min_y.to_bits().hash(state);
    scene.view_box.width.to_bits().hash(state);
    scene.view_box.height.to_bits().hash(state);
    scene.paths.len().hash(state);
    for path in &scene.paths {
        hash_svg_path(path, state);
    }
}

fn hash_svg_path(path: &SvgPathInstance, state: &mut impl Hasher) {
    hash_svg_geometry(&path.geometry, state);
    path.paint.fill.map(hashable_color).hash(state);
    path.paint.stroke.map(hashable_color).hash(state);
    path.paint.stroke_width.to_bits().hash(state);
}

fn hash_svg_geometry(geometry: &SvgPathGeometry, state: &mut impl Hasher) {
    geometry.contours.len().hash(state);
    for contour in &geometry.contours {
        contour.closed.hash(state);
        contour.points.len().hash(state);
        for point in &contour.points {
            hash_svg_point(point, state);
        }
    }
}

fn hash_svg_point(point: &SvgPoint, state: &mut impl Hasher) {
    point.x.to_bits().hash(state);
    point.y.to_bits().hash(state);
}

fn select_surface_format(formats: &[wgpu::TextureFormat]) -> Option<wgpu::TextureFormat> {
    formats
        .iter()
        .copied()
        .find(|format| {
            matches!(
                format,
                wgpu::TextureFormat::Bgra8UnormSrgb
                    | wgpu::TextureFormat::Rgba8UnormSrgb
                    | wgpu::TextureFormat::Bgra8Unorm
                    | wgpu::TextureFormat::Rgba8Unorm
            )
        })
        .or_else(|| formats.first().copied())
}

fn select_present_mode(modes: &[wgpu::PresentMode]) -> wgpu::PresentMode {
    if modes.contains(&wgpu::PresentMode::Fifo) {
        wgpu::PresentMode::Fifo
    } else {
        modes.first().copied().unwrap_or(wgpu::PresentMode::Fifo)
    }
}

fn next_texture_use(next_use: &mut u64) -> u64 {
    let last_used = *next_use;
    *next_use = next_use.saturating_add(1);
    last_used
}

fn evict_lru_text_texture(cache: &mut HashMap<TextTextureKey, CachedTextTexture>) {
    let lru_key = cache
        .iter()
        .min_by_key(|(_, texture)| texture.last_used)
        .map(|(key, _)| key.clone());
    if let Some(lru_key) = lru_key {
        cache.remove(&lru_key);
    }
}

fn evict_lru_color_texture(cache: &mut HashMap<ColorTextureKey, CachedColorTexture>) {
    let lru_key = cache
        .iter()
        .min_by_key(|(_, texture)| texture.last_used)
        .map(|(key, _)| *key);
    if let Some(lru_key) = lru_key {
        cache.remove(&lru_key);
    }
}

fn premultiplied_linear_rgba_texels(pixels: &[LinearRgba]) -> Vec<u8> {
    let mut texels = Vec::with_capacity(pixels.len().saturating_mul(4));
    for pixel in pixels {
        let alpha = pixel.a.clamp(0.0, 1.0);
        texels.push((pixel.r.clamp(0.0, 1.0) * alpha * 255.0).round() as u8);
        texels.push((pixel.g.clamp(0.0, 1.0) * alpha * 255.0).round() as u8);
        texels.push((pixel.b.clamp(0.0, 1.0) * alpha * 255.0).round() as u8);
        texels.push((alpha * 255.0).round() as u8);
    }
    texels
}

fn packed_color(color: Color, srgb_surface: bool) -> [f32; 4] {
    let color = if srgb_surface {
        color.to_linear_rgba()
    } else {
        LinearRgba {
            r: color.r as f32 / 255.0,
            g: color.g as f32 / 255.0,
            b: color.b as f32 / 255.0,
            a: color.a as f32 / 255.0,
        }
    };
    [color.r, color.g, color.b, color.a]
}

fn color_to_wgpu(color: Color, srgb_surface: bool) -> wgpu::Color {
    let color = if srgb_surface {
        color.to_linear_rgba()
    } else {
        LinearRgba {
            r: color.r as f32 / 255.0,
            g: color.g as f32 / 255.0,
            b: color.b as f32 / 255.0,
            a: color.a as f32 / 255.0,
        }
    };
    wgpu::Color {
        r: color.r as f64,
        g: color.g as f64,
        b: color.b as f64,
        a: color.a as f64,
    }
}

fn layout_rect(layout: LayoutBox) -> [f32; 4] {
    [
        layout.x,
        layout.y,
        layout.width.max(0.0),
        layout.height.max(0.0),
    ]
}

fn corner_radii(layout: LayoutBox, radius: CornerRadius) -> [f32; 4] {
    let max_radius = 0.5 * layout.width.min(layout.height).max(0.0);
    [
        resolved_corner_radius(radius.top_left, max_radius),
        resolved_corner_radius(radius.top_right, max_radius),
        resolved_corner_radius(radius.bottom_right, max_radius),
        resolved_corner_radius(radius.bottom_left, max_radius),
    ]
}

fn resolved_corner_radius(value: f32, max_radius: f32) -> f32 {
    if value < 0.0 {
        (-value * max_radius).min(max_radius).max(0.0)
    } else {
        value.min(max_radius).max(0.0)
    }
}

fn text_font_family_key(family: &FontFamily) -> TextFontFamilyKey {
    match family {
        FontFamily::Named(name) => TextFontFamilyKey::Named(name.clone()),
        FontFamily::Generic(family) => {
            TextFontFamilyKey::Generic(generic_font_family_key(family.clone()))
        }
    }
}

fn generic_font_family_key(family: GenericFontFamily) -> u8 {
    match family {
        GenericFontFamily::Serif => 0,
        GenericFontFamily::SansSerif => 1,
        GenericFontFamily::Cursive => 2,
        GenericFontFamily::Fantasy => 3,
        GenericFontFamily::Monospace => 4,
        GenericFontFamily::SystemUi => 5,
        GenericFontFamily::Emoji => 6,
        GenericFontFamily::Math => 7,
        GenericFontFamily::FangSong => 8,
        GenericFontFamily::UiSerif => 9,
        GenericFontFamily::UiSansSerif => 10,
        GenericFontFamily::UiMonospace => 11,
        GenericFontFamily::UiRounded => 12,
    }
}

fn text_font_style_key(style: FontStyle) -> u8 {
    match style {
        FontStyle::Normal => 0,
        FontStyle::Italic => 1,
        FontStyle::Oblique => 2,
    }
}

fn text_line_height_key(line_height: &LineHeight) -> TextLineHeightKey {
    match line_height {
        LineHeight::Normal => TextLineHeightKey::Normal,
        LineHeight::Px(value) => TextLineHeightKey::Px(value.to_bits()),
        LineHeight::Scale(value) => TextLineHeightKey::Scale(value.to_bits()),
    }
}

fn text_transform_key(text_transform: TextTransform) -> u8 {
    match text_transform {
        TextTransform::None => 0,
        TextTransform::Uppercase => 1,
        TextTransform::Lowercase => 2,
        TextTransform::Capitalize => 3,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use cssimpler_core::fonts::TextStyle;
    use cssimpler_core::{
        Color, ExtractedPaintKind, LayoutBox, Overflow, RenderNode, TextStrokeStyle, Transform2D,
        TransformOperation, VisualStyle,
    };

    use super::{prepare_gpu_scene, text_paint_layout, unsupported_gpu_reason};
    use crate::ExtractedScene;
    use cssimpler_core::Insets;

    #[test]
    fn gpu_scene_supports_basic_fill_border_and_text() {
        let scene = vec![
            RenderNode::container(LayoutBox::new(0.0, 0.0, 240.0, 120.0))
                .with_style(VisualStyle {
                    overflow: Overflow {
                        x: cssimpler_core::OverflowMode::Hidden,
                        y: cssimpler_core::OverflowMode::Hidden,
                    },
                    background: Some(Color::rgb(15, 23, 42)),
                    border: cssimpler_core::BorderStyle {
                        widths: cssimpler_core::Insets::all(2.0),
                        color: Color::rgb(148, 163, 184),
                    },
                    corner_radius: cssimpler_core::CornerRadius::all(12.0),
                    ..VisualStyle::default()
                })
                .with_child(
                    RenderNode::text(LayoutBox::new(24.0, 24.0, 120.0, 24.0), "gpu").with_style(
                        VisualStyle {
                            foreground: Color::WHITE,
                            text: TextStyle::default(),
                            ..VisualStyle::default()
                        },
                    ),
                ),
        ];

        let extracted = ExtractedScene::from_render_roots(&scene);
        let prepared = prepare_gpu_scene(&extracted, 240, 120, true, &HashMap::new())
            .expect("scene should stay supported");

        assert_eq!(prepared.fills.len(), 1);
        assert_eq!(prepared.borders.len(), 1);
        assert_eq!(prepared.texts.len(), 1);
        assert_eq!(prepared.commands.len(), 3);
    }

    #[test]
    fn gpu_scene_uses_inset_text_layout_for_padded_text_nodes() {
        let scene = vec![
            RenderNode::text(LayoutBox::new(10.0, 20.0, 100.0, 46.0), "spike")
                .with_style(VisualStyle {
                    foreground: Color::WHITE,
                    text: TextStyle::default(),
                    ..VisualStyle::default()
                })
                .with_content_inset(Insets {
                    top: 12.0,
                    right: 16.0,
                    bottom: 12.0,
                    left: 16.0,
                }),
        ];

        let extracted = ExtractedScene::from_render_roots(&scene);
        let text_item = extracted
            .items
            .iter()
            .find(|item| item.kind == ExtractedPaintKind::TextRun)
            .expect("text paint item should be extracted");
        let expected_layout = text_paint_layout(text_item);
        let prepared = prepare_gpu_scene(&extracted, 160, 100, true, &HashMap::new())
            .expect("padded text should stay supported");

        assert_eq!(prepared.texts.len(), 1);
        assert_eq!(
            prepared.texts[0].key.width_bits,
            expected_layout.width.to_bits()
        );
        assert!(
            prepared.texts[0].mask.origin_x() as f32 >= expected_layout.x,
            "text mask should start inside the padded content box"
        );
        assert!(
            prepared.texts[0].mask.origin_y() as f32 >= expected_layout.y,
            "text mask should start inside the padded content box"
        );
    }

    #[test]
    fn gpu_scene_routes_background_layers_through_color_textures() {
        let scene = vec![
            RenderNode::container(LayoutBox::new(0.0, 0.0, 160.0, 90.0)).with_style(VisualStyle {
                background: Some(Color::rgb(15, 23, 42)),
                background_layers: vec![cssimpler_core::BackgroundLayer::LinearGradient(
                    cssimpler_core::LinearGradient {
                        direction: cssimpler_core::GradientDirection::Angle(90.0),
                        interpolation: cssimpler_core::GradientInterpolation::Oklab,
                        repeating: false,
                        stops: vec![
                            cssimpler_core::GradientStop {
                                color: Color::rgb(59, 130, 246),
                                position: cssimpler_core::LengthPercentageValue::from_fraction(0.0),
                            },
                            cssimpler_core::GradientStop {
                                color: Color::rgb(16, 185, 129),
                                position: cssimpler_core::LengthPercentageValue::from_fraction(1.0),
                            },
                        ],
                    },
                )],
                ..VisualStyle::default()
            }),
        ];

        let extracted = ExtractedScene::from_render_roots(&scene);
        assert!(
            extracted
                .items
                .iter()
                .find_map(unsupported_gpu_reason)
                .is_none(),
            "gradient background should stay on the GPU path"
        );
        let prepared = prepare_gpu_scene(&extracted, 160, 90, true, &HashMap::new())
            .expect("gradient scene should stay supported");

        assert_eq!(prepared.fills.len(), 1);
        assert_eq!(prepared.color_textures.len(), 1);
        assert!(
            prepared
                .commands
                .iter()
                .any(|command| matches!(command, super::GpuPaintCommand::ColorTexture { .. }))
        );
    }

    #[test]
    fn gpu_scene_rasterizes_a_shared_gradient_texture_only_once_per_frame() {
        let gradient =
            cssimpler_core::BackgroundLayer::LinearGradient(cssimpler_core::LinearGradient {
                direction: cssimpler_core::GradientDirection::Angle(90.0),
                interpolation: cssimpler_core::GradientInterpolation::Oklab,
                repeating: false,
                stops: vec![
                    cssimpler_core::GradientStop {
                        color: Color::rgb(59, 130, 246),
                        position: cssimpler_core::LengthPercentageValue::from_fraction(0.0),
                    },
                    cssimpler_core::GradientStop {
                        color: Color::rgb(16, 185, 129),
                        position: cssimpler_core::LengthPercentageValue::from_fraction(1.0),
                    },
                ],
            });
        let scene = vec![
            RenderNode::container(LayoutBox::new(0.0, 0.0, 32.0, 20.0)).with_style(VisualStyle {
                background_layers: vec![gradient.clone()],
                ..VisualStyle::default()
            }),
            RenderNode::container(LayoutBox::new(64.0, 0.0, 32.0, 20.0)).with_style(VisualStyle {
                background_layers: vec![gradient],
                ..VisualStyle::default()
            }),
        ];

        let extracted = ExtractedScene::from_render_roots(&scene);
        let prepared = prepare_gpu_scene(&extracted, 128, 32, true, &HashMap::new())
            .expect("shared gradient textures should stay supported");

        assert_eq!(prepared.color_textures.len(), 2);
        assert_eq!(
            prepared
                .color_textures
                .iter()
                .filter(|command| command.raster.is_some())
                .count(),
            1
        );
    }

    #[test]
    fn gpu_scene_uses_accumulated_clip_for_scissor_bounds() {
        let scene = vec![
            RenderNode::container(LayoutBox::new(0.0, 0.0, 100.0, 100.0))
                .with_style(VisualStyle {
                    overflow: Overflow {
                        x: cssimpler_core::OverflowMode::Hidden,
                        y: cssimpler_core::OverflowMode::Hidden,
                    },
                    ..VisualStyle::default()
                })
                .with_child(
                    RenderNode::text(LayoutBox::new(80.0, 80.0, 64.0, 24.0), "clip").with_style(
                        VisualStyle {
                            foreground: Color::WHITE,
                            ..VisualStyle::default()
                        },
                    ),
                ),
        ];

        let extracted = ExtractedScene::from_render_roots(&scene);
        let prepared = prepare_gpu_scene(&extracted, 100, 100, true, &HashMap::new())
            .expect("scene should stay supported");
        let command = prepared
            .commands
            .iter()
            .find(|command| matches!(command, super::GpuPaintCommand::Text { .. }))
            .expect("text command should exist");

        match command {
            super::GpuPaintCommand::Text { scissor, .. } => {
                assert_eq!(scissor.width, 100);
                assert_eq!(scissor.height, 100);
            }
            _ => unreachable!("text command match already filtered"),
        }
    }

    #[test]
    fn gpu_scene_keeps_transparent_stroked_text_via_color_texture() {
        let scene = vec![
            RenderNode::text(LayoutBox::new(12.0, 12.0, 180.0, 48.0), "stroke").with_style(
                VisualStyle {
                    foreground: Color::rgba(255, 255, 255, 0),
                    text_stroke: TextStrokeStyle {
                        width: 1.0,
                        color: Some(Color::rgb(148, 163, 184)),
                    },
                    text: TextStyle::default(),
                    ..VisualStyle::default()
                },
            ),
        ];

        let extracted = ExtractedScene::from_render_roots(&scene);
        let prepared = prepare_gpu_scene(&extracted, 240, 120, true, &HashMap::new())
            .expect("stroked transparent text should stay supported");

        assert!(prepared.texts.is_empty());
        assert_eq!(prepared.color_textures.len(), 1);
        assert!(
            prepared
                .commands
                .iter()
                .any(|command| matches!(command, super::GpuPaintCommand::ColorTexture { .. }))
        );
    }

    #[test]
    fn gpu_scene_routes_projected_subtrees_through_projected_textures() {
        let _surface_cache_guard = crate::lock_subtree_surface_cache_for_tests();
        crate::clear_subtree_surface_cache_for_tests();

        let scene = vec![
            RenderNode::container(LayoutBox::new(24.0, 24.0, 160.0, 120.0))
                .with_style(VisualStyle {
                    background: Some(Color::rgb(15, 23, 42)),
                    transform: Transform2D {
                        operations: vec![TransformOperation::Rotate { degrees: 8.0 }],
                        ..Transform2D::default()
                    },
                    ..VisualStyle::default()
                })
                .with_child(
                    RenderNode::text(LayoutBox::new(36.0, 56.0, 96.0, 24.0), "gpu").with_style(
                        VisualStyle {
                            foreground: Color::WHITE,
                            text: TextStyle::default(),
                            ..VisualStyle::default()
                        },
                    ),
                ),
        ];

        let extracted = ExtractedScene::from_render_roots(&scene);
        let prepared = prepare_gpu_scene(&extracted, 240, 180, true, &HashMap::new())
            .expect("projected subtrees should stay supported");

        assert!(prepared.fills.is_empty());
        assert!(prepared.texts.is_empty());
        assert_eq!(prepared.projected_textures.len(), 1);
        assert!(
            prepared
                .commands
                .iter()
                .any(|command| matches!(command, super::GpuPaintCommand::ProjectedTexture { .. }))
        );
    }

    #[test]
    fn gpu_scene_clamps_large_corner_radii_for_small_fill_and_border_nodes() {
        let scene = vec![
            RenderNode::container(LayoutBox::new(8.0, 12.0, 20.0, 20.0)).with_style(VisualStyle {
                background: Some(Color::rgb(56, 189, 248)),
                border: cssimpler_core::BorderStyle {
                    widths: Insets::all(2.0),
                    color: Color::rgb(224, 242, 254),
                },
                corner_radius: cssimpler_core::CornerRadius::all(999.0),
                ..VisualStyle::default()
            }),
        ];

        let extracted = ExtractedScene::from_render_roots(&scene);
        let prepared = prepare_gpu_scene(&extracted, 64, 64, true, &HashMap::new())
            .expect("large radii should clamp instead of disappearing");

        assert_eq!(prepared.fills.len(), 1);
        assert_eq!(prepared.borders.len(), 1);
        assert_eq!(prepared.fills[0].radii, [8.0, 8.0, 8.0, 8.0]);
        assert_eq!(prepared.borders[0].outer_radii, [10.0, 10.0, 10.0, 10.0]);
        assert_eq!(prepared.borders[0].inner_radii, [8.0, 8.0, 8.0, 8.0]);
    }
}
