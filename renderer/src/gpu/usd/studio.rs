use super::*;

const SHADOW_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const SHADOW_USAGE: wgpu::TextureUsages =
    wgpu::TextureUsages::RENDER_ATTACHMENT.union(wgpu::TextureUsages::TEXTURE_BINDING);

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct StudioUniform {
    light_projection: [[f32; 4]; 4],
    key: [f32; 4],
    up: [f32; 4],
    lighting: [f32; 4],
    occlusion_plane: [f32; 4],
    shadows: [f32; 4],
}

struct ShadowMap {
    view: wgpu::TextureView,
    binding: wgpu::BindGroup,
    resolution: u32,
}

struct ShadowPass {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    map: ShadowMap,
    sampler: wgpu::Sampler,
    needs_render: bool,
}

pub(super) struct Studio {
    pub uniform: wgpu::Buffer,
    pub settings: Option<UsdStudioLighting>,
    pub floor: Option<wgpu::Buffer>,
    shadow: Option<ShadowPass>,
    bounds: [Vec3; 2],
    up: Vec3,
    pub dirty: bool,
}

impl Studio {
    pub fn new(gpu: &GpuContext, layout: &wgpu::BindGroupLayout, cache: &MeshCache) -> Self {
        let uniform = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("USD studio parameters"),
            size: std::mem::size_of::<StudioUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let depth = gpu.adapter.get_texture_format_features(SHADOW_FORMAT);
        let capabilities = gpu.adapter.get_downlevel_capabilities();
        let shadow = (depth.allowed_usages.contains(SHADOW_USAGE)
            && capabilities
                .flags
                .contains(wgpu::DownlevelFlags::COMPARISON_SAMPLERS))
        .then(|| ShadowPass::new(gpu, layout));
        Self {
            uniform,
            settings: None,
            floor: None,
            shadow,
            bounds: [cache.minimum, cache.maximum],
            up: cache.up,
            dirty: true,
        }
    }

    pub fn shadow_layout(&self) -> Option<&wgpu::BindGroupLayout> {
        self.shadow.as_ref().map(|shadow| &shadow.layout)
    }

    pub fn shadow_binding(&self) -> Option<&wgpu::BindGroup> {
        self.shadow.as_ref().map(|shadow| &shadow.map.binding)
    }

    pub fn update_mesh(&mut self, cache: &MeshCache) {
        self.bounds = [cache.minimum, cache.maximum];
        self.up = cache.up;
        self.dirty = true;
    }

    pub fn sync(&mut self, gpu: &GpuContext) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let mut params = StudioUniform {
            light_projection: Mat4::IDENTITY.to_cols_array_2d(),
            key: [0.0; 4],
            up: self.up.extend(0.0).to_array(),
            lighting: [0.0, 0.0, 1.0, 0.0],
            occlusion_plane: [0.0; 4],
            shadows: [0.0; 4],
        };
        let shadow_settings = self.settings.and_then(|settings| settings.shadows);
        if let Some(shadow) = &mut self.shadow {
            let resolution = shadow_settings.map_or(1, |s| {
                s.resolution
                    .min(gpu.device.limits().max_texture_dimension_2d)
            });
            if shadow.map.resolution != resolution {
                shadow.map = ShadowMap::new(gpu, &shadow.layout, &shadow.sampler, resolution);
            }
            shadow.needs_render = shadow_settings.is_some();
        }
        self.floor = None;
        if let Some(settings) = self.settings {
            let key = from_studio_axes(Vec3::from_array(settings.toward_key), self.up).normalize();
            let [minimum, maximum] = self.bounds;
            let center = minimum + (maximum - minimum) * 0.5;
            let radius = ((maximum - minimum) * 0.5).length().max(0.01);
            let height = minimum.dot(self.up) - radius * 0.015;
            params.key = key.extend(1.0).to_array();
            params.up = self.up.extend(settings.ambient).to_array();
            params.lighting = [
                settings.key_intensity,
                settings.occlusion,
                settings.exposure.exp2(),
                settings.highlight_rolloff,
            ];
            params.occlusion_plane = [height, radius.recip(), 0.0, 0.0];

            if let Some(s) = shadow_settings.filter(|_| self.shadow.is_some()) {
                params.light_projection =
                    light_projection(center, radius, key, self.up).to_cols_array_2d();
                params.shadows = [1.0, s.depth_bias, s.slope_bias, s.filter_radius as f32];
            }

            if settings.floor {
                self.floor = Some(vertex_buffer(
                    gpu,
                    &floor_vertices(center, height, radius, self.up),
                ));
            }
        }
        gpu.queue
            .write_buffer(&self.uniform, 0, bytemuck::bytes_of(&params));
    }

    /// Only scene geometry casts into the cached map. The generated floor
    /// receives shadows but is excluded from the light-space pass.
    pub fn render_shadow(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        uniforms: &wgpu::BindGroup,
        vertices: &wgpu::Buffer,
        vertex_count: u32,
    ) {
        let Some(shadow) = self.shadow.as_mut().filter(|shadow| shadow.needs_render) else {
            return;
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("USD key-light depth"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &shadow.map.view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        pass.set_pipeline(&shadow.pipeline);
        pass.set_bind_group(0, uniforms, &[]);
        pass.set_vertex_buffer(0, vertices.slice(..));
        pass.draw(0..vertex_count, 0..1);
        shadow.needs_render = false;
    }
}

impl ShadowPass {
    fn new(gpu: &GpuContext, uniforms: &wgpu::BindGroupLayout) -> Self {
        let layout = gpu
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("USD shadow texture layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Depth,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                        count: None,
                    },
                ],
            });
        let pipeline_layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("USD key-light depth layout"),
                bind_group_layouts: &[Some(uniforms)],
                immediate_size: 0,
            });
        let shader = usd_shader(gpu, false);
        let pipeline = gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("USD key-light depth pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("shadow_vertex"),
                    compilation_options: Default::default(),
                    buffers: &[Some(VERTEX_LAYOUT)],
                },
                fragment: None,
                primitive: Default::default(),
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: SHADOW_FORMAT,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::Less),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            });
        let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("USD key-light depth comparison"),
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        let map = ShadowMap::new(gpu, &layout, &sampler, 1);
        Self {
            layout,
            pipeline,
            map,
            sampler,
            needs_render: false,
        }
    }
}

impl ShadowMap {
    fn new(
        gpu: &GpuContext,
        layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        resolution: u32,
    ) -> Self {
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("USD key-light depth map"),
            size: wgpu::Extent3d {
                width: resolution,
                height: resolution,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: SHADOW_FORMAT,
            usage: SHADOW_USAGE,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let binding = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("USD shadow texture"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        });
        Self {
            view,
            binding,
            resolution,
        }
    }
}

pub(super) fn from_studio_axes(p: Vec3, up: Vec3) -> Vec3 {
    if up == Vec3::Z {
        Vec3::new(p.x, -p.z, p.y)
    } else {
        p
    }
}

pub(super) fn stage_up(stage: &Stage) -> Result<Vec3> {
    Ok(match stage.stage_metadata("upAxis").map_err(error)? {
        Some(sdf::Value::Token(axis)) if axis.as_ref() == "Z" => Vec3::Z,
        _ => Vec3::Y,
    })
}

fn light_projection(center: Vec3, radius: f32, key: Vec3, up: Vec3) -> Mat4 {
    let light_up = if key.dot(up).abs() > 0.99 {
        Vec3::X
    } else {
        up
    };
    let view =
        glam::camera::rh::view::look_at_mat4(center + key * (2.0 * radius), center, light_up);
    let extent = radius * 1.05;
    glam::camera::rh::proj::directx::orthographic(
        -extent,
        extent,
        -extent,
        extent,
        radius * 0.5,
        radius * 3.5,
    ) * view
}

fn floor_vertices(center: Vec3, height: f32, radius: f32, up: Vec3) -> [Vertex; 6] {
    let center = center + up * (height - center.dot(up));
    let extent = radius * 3.0;
    let east = Vec3::X * extent;
    let north = from_studio_axes(Vec3::Z, up) * extent;
    let corners = [
        center - east - north,
        center - east + north,
        center + east + north,
        center + east - north,
    ];
    [0, 1, 2, 0, 2, 3].map(|i| Vertex {
        position: corners[i].to_array(),
        normal: up.to_array(),
        color: [0.28, 0.30, 0.33],
        uv: [0.0; 3],
        local: [0.0; 3],
    })
}
