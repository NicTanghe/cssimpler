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
    shadow_fit: [f32; 4],
    key_tint: [f32; 4],
    fill: [f32; 4],
    fill_tint: [f32; 4],
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
    caster_bounds: [Vec3; 2],
    scene_bounds: [Vec3; 2],
    caster_ranges: Vec<Range<u32>>,
    up: Vec3,
    dirty: bool,
    depth_dirty: bool,
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
            caster_bounds: cache
                .shadow_bounds
                .unwrap_or([cache.minimum, cache.maximum]),
            scene_bounds: [cache.minimum, cache.maximum],
            caster_ranges: cache.shadow_ranges.clone(),
            up: cache.up,
            dirty: true,
            depth_dirty: true,
        }
    }

    pub fn shadow_layout(&self) -> Option<&wgpu::BindGroupLayout> {
        self.shadow.as_ref().map(|shadow| &shadow.layout)
    }

    pub fn shadow_binding(&self) -> Option<&wgpu::BindGroup> {
        self.shadow.as_ref().map(|shadow| &shadow.map.binding)
    }

    pub fn set_settings(&mut self, settings: Option<UsdStudioLighting>) -> bool {
        if self.settings == settings {
            return false;
        }
        let depth_key = |settings: Option<UsdStudioLighting>| {
            settings.and_then(|light| {
                light
                    .shadows
                    .map(|shadow| (light.toward_key, shadow.resolution, shadow.coverage))
            })
        };
        self.depth_dirty |= depth_key(self.settings) != depth_key(settings);
        self.settings = settings;
        self.dirty = true;
        true
    }

    pub fn update_mesh(&mut self, cache: &MeshCache) {
        self.scene_bounds = [cache.minimum, cache.maximum];
        self.caster_bounds = cache.shadow_bounds.unwrap_or(self.scene_bounds);
        self.caster_ranges.clone_from(&cache.shadow_ranges);
        self.up = cache.up;
        self.dirty = true;
        self.depth_dirty = true;
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
            shadow_fit: [1.0, 1.0, 0.0, 1.0],
            key_tint: [1.0, 1.0, 1.0, 0.0],
            fill: [0.0; 4],
            fill_tint: [1.0, 1.0, 1.0, 0.0],
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
            shadow.needs_render =
                shadow_settings.is_some() && (shadow.needs_render || self.depth_dirty);
        }
        self.depth_dirty = false;
        self.floor = None;
        if let Some(settings) = self.settings {
            let key = from_studio_axes(Vec3::from_array(settings.toward_key), self.up).normalize();
            let [minimum, maximum] = self.caster_bounds;
            let radius = ((maximum - minimum) * 0.5).length().max(0.01);
            let height = self.scene_bounds[0].dot(self.up) - radius * 0.015;
            params.key = key.extend(1.0).to_array();
            params.key_tint[..3].copy_from_slice(&settings.key_tint);
            params.fill = from_studio_axes(Vec3::from_array(settings.toward_fill), self.up)
                .normalize()
                .extend(settings.fill_intensity)
                .to_array();
            params.fill_tint[..3].copy_from_slice(&settings.fill_tint);
            params.up = self.up.extend(settings.ambient).to_array();
            params.lighting = [
                settings.key_intensity,
                settings.occlusion,
                settings.exposure.exp2(),
                settings.highlight_rolloff,
            ];
            params.occlusion_plane = [height, radius.recip(), 0.0, 0.0];

            if let Some(s) = shadow_settings.filter(|_| self.shadow.is_some()) {
                let fit = shadow_projection(
                    self.caster_bounds,
                    self.scene_bounds,
                    key,
                    self.up,
                    s.coverage,
                );
                params.light_projection = fit.matrix.to_cols_array_2d();
                params.shadow_fit = fit.sampling;
                params.shadows = [
                    1.0,
                    s.depth_bias * fit.bias_scale,
                    s.slope_bias * fit.bias_scale,
                    s.filter_radius as f32,
                ];
                params.key_tint[3] = s.light_radius.tan();
                params.fill_tint[3] = s.strength;
            }

            if settings.floor {
                let [minimum, maximum] = self.scene_bounds;
                let center = minimum + (maximum - minimum) * 0.5;
                let radius = ((maximum - minimum) * 0.5).length().max(0.01);
                self.floor = Some(vertex_buffer(
                    gpu,
                    &floor_vertices(center, height, radius, self.up),
                ));
            }
        }
        gpu.queue
            .write_buffer(&self.uniform, 0, bytemuck::bytes_of(&params));
    }

    /// Receiver-only scene meshes and the generated floor stay out of this pass.
    pub fn render_shadow(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        uniforms: &wgpu::BindGroup,
        vertices: &wgpu::Buffer,
        indices: &wgpu::Buffer,
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
        pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
        for range in &self.caster_ranges {
            pass.draw_indexed(range.clone(), 0, 0..1);
        }
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
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
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

struct ShadowProjection {
    matrix: Mat4,
    /// UV span per unit of normalized depth, then the caster depth interval.
    sampling: [f32; 4],
    bias_scale: f32,
}

fn shadow_projection(
    casters: [Vec3; 2],
    scene: [Vec3; 2],
    key: Vec3,
    up: Vec3,
    coverage: f32,
) -> ShadowProjection {
    let light_up = if key.dot(up).abs() > 0.99 {
        Vec3::X
    } else {
        up
    };
    let center = casters[0] + (casters[1] - casters[0]) * 0.5;
    let view = glam::camera::rh::view::look_to_mat4(center, -key, light_up);
    let [caster_min, caster_max] = light_bounds(view, casters);
    let [scene_min, scene_max] = light_bounds(view, scene);
    // The cove can be many times larger than the subject. Give small
    // adjustments near the caster-only end more room on the control.
    let weight = coverage.powi(3);
    let mut minimum = caster_min.lerp(scene_min, weight);
    let mut maximum = caster_max.lerp(scene_max, weight);
    let padding = ((maximum - minimum) * 0.025).max(Vec3::splat(0.01));
    minimum -= padding;
    maximum += padding;
    let span = maximum - minimum;
    let caster_depth = caster_max.z - caster_min.z;
    let tight_depth_span = caster_depth + 2.0 * (caster_depth * 0.025).max(0.01);
    ShadowProjection {
        matrix: glam::camera::rh::proj::directx::orthographic(
            minimum.x, maximum.x, minimum.y, maximum.y, -maximum.z, -minimum.z,
        ) * view,
        sampling: [
            span.z / span.x,
            span.z / span.y,
            (maximum.z - caster_max.z) / span.z,
            (maximum.z - caster_min.z) / span.z,
        ],
        bias_scale: tight_depth_span / span.z,
    }
}

fn light_bounds(view: Mat4, [min, max]: [Vec3; 2]) -> [Vec3; 2] {
    let mut minimum = Vec3::splat(f32::INFINITY);
    let mut maximum = Vec3::splat(f32::NEG_INFINITY);
    for x in [min.x, max.x] {
        for y in [min.y, max.y] {
            for z in [min.z, max.z] {
                let point = view.transform_point3(Vec3::new(x, y, z));
                minimum = minimum.min(point);
                maximum = maximum.max(point);
            }
        }
    }
    [minimum, maximum]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coverage_expands_from_subject_to_cove_without_clipping_casters() {
        let casters = [Vec3::new(-0.55, 0.0, -0.25), Vec3::new(0.55, 1.8, 0.4)];
        let scene = [Vec3::new(-30.0, 0.0, -30.0), Vec3::new(30.0, 40.0, 30.0)];
        let key = Vec3::from_array(UsdStudioLighting::default().toward_key).normalize();
        let fit = |coverage| shadow_projection(casters, scene, key, Vec3::Y, coverage);
        let tight = fit(0.0);
        let middle = fit(0.5);
        let full = fit(1.0);
        let contains = |projection, bounds| {
            let [min, max] = light_bounds(projection, bounds);
            min.cmpge(Vec3::new(-1.0, -1.0, 0.0)).all() && max.cmple(Vec3::ONE).all()
        };

        assert!(contains(tight.matrix, casters));
        assert!(contains(middle.matrix, casters));
        assert!(contains(full.matrix, scene));
        assert!(!contains(tight.matrix, scene));
        assert!(!contains(middle.matrix, scene));

        let ground_area = |matrix: Mat4| {
            let axes = matrix.transpose();
            let area_in_light_space =
                4.0 / (axes.x_axis.truncate().length() * axes.y_axis.truncate().length());
            area_in_light_space / key.y
        };
        let tight_area = ground_area(tight.matrix);
        assert!((3.0..10.0).contains(&tight_area), "{tight_area} m²");
        assert!(tight_area < ground_area(middle.matrix));
        assert!(ground_area(middle.matrix) < ground_area(full.matrix));
        assert!((tight.bias_scale - 1.0).abs() < 1e-6);
        assert!(full.bias_scale < middle.bias_scale && middle.bias_scale < tight.bias_scale);
        for projection in [tight, middle, full] {
            let [min, max] = light_bounds(projection.matrix, casters);
            assert!((projection.sampling[2] - min.z).abs() < 1e-6);
            assert!((projection.sampling[3] - max.z).abs() < 1e-6);
        }
    }

    #[test]
    fn shadow_projection_handles_degenerate_bounds_and_vertical_z_up_light() {
        let bounds = [Vec3::new(0.0, 0.0, 2.0); 2];
        let fit = shadow_projection(bounds, bounds, Vec3::Z, Vec3::Z, 1.0);
        assert!(fit.matrix.is_finite());
        assert!(
            fit.matrix
                .transform_point3(bounds[0])
                .abs_diff_eq(Vec3::Z * 0.5, 1e-5)
        );
    }
}
