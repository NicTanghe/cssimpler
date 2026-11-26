//! View-local material overrides and studio lighting for the built-in USD backend.
use super::*;
use std::sync::Arc;

/// Surface response used by a viewport material override.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u32)]
pub enum UsdSurface {
    #[default]
    Plain,
    Fabric,
    BrushedMetal,
}

/// Source of surface coordinates, before applying the material's UV scale.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u32)]
pub enum UsdUvMapping {
    /// Read USD `primvars:st`. Skip the texture if the mesh has no usable UVs.
    Authored,
    /// Map around the mesh's local up axis, centered on its local bounds.
    Cylindrical,
    /// Use authored UVs when available and cylindrical mapping otherwise.
    #[default]
    AuthoredOrCylindrical,
}

/// Immutable, tightly packed RGBA8 base-color pixels in sRGB, top row first.
/// Alpha is ignored by the opaque USD mesh pipeline.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsdColorTexture {
    size: [u32; 2],
    rgba: Arc<[u8]>,
}

impl UsdColorTexture {
    pub fn new(size: [u32; 2], rgba: impl Into<Arc<[u8]>>) -> Result<Self> {
        let rgba = rgba.into();
        if size.contains(&0)
            || size[0].checked_mul(4).is_none()
            || (size[0] as usize)
                .checked_mul(size[1] as usize)
                .and_then(|pixels| pixels.checked_mul(4))
                != Some(rgba.len())
        {
            return Err(error("USD texture dimensions do not match RGBA8 data"));
        }
        Ok(Self { size, rgba })
    }

    pub fn size(&self) -> [u32; 2] {
        self.size
    }
}

/// A render-only material override. Colors are in linear RGB.
#[derive(Clone, Debug, PartialEq)]
pub struct UsdMaterial {
    pub surface: UsdSurface,
    /// Leave unset to use the mesh's USD displayColor.
    pub base_color: Option<[f32; 3]>,
    pub roughness: f32,
    pub sheen: f32,
    /// Weave cycles per mapped UV unit.
    pub weave_frequency: f32,
    pub mapping: UsdUvMapping,
    pub uv_scale: [f32; 2],
    pub color_texture: Option<UsdColorTexture>,
}

impl Default for UsdMaterial {
    fn default() -> Self {
        Self {
            surface: UsdSurface::Plain,
            base_color: None,
            roughness: 0.55,
            sheen: 0.25,
            weave_frequency: 64.0,
            mapping: UsdUvMapping::default(),
            uv_scale: [1.0; 2],
            color_texture: None,
        }
    }
}

impl UsdMaterial {
    pub fn fabric(color: [f32; 3]) -> Self {
        Self {
            surface: UsdSurface::Fabric,
            base_color: Some(color),
            ..Self::default()
        }
    }

    /// Supply a neutral tint for aluminium or a warm tint for brass.
    pub fn brushed_metal(color: [f32; 3]) -> Self {
        Self {
            surface: UsdSurface::BrushedMetal,
            base_color: Some(color),
            ..Self::default()
        }
    }

    pub fn dark_stand() -> Self {
        Self {
            base_color: Some([0.035, 0.040, 0.045]),
            ..Self::default()
        }
    }

    pub(super) fn validate(&self, max_dimension: u32) -> Result<()> {
        if self
            .base_color
            .iter()
            .flatten()
            .any(|v| !v.is_finite() || *v < 0.0)
            || ![self.roughness, self.sheen]
                .iter()
                .all(|v| (0.0..=1.0).contains(v))
            || !self.weave_frequency.is_finite()
            || self.weave_frequency < 0.0
            || self.uv_scale.iter().any(|v| !v.is_finite() || *v == 0.0)
        {
            return Err(error("invalid USD viewport material parameters"));
        }
        if self
            .color_texture
            .as_ref()
            .is_some_and(|t| t.size.iter().any(|&s| s > max_dimension))
        {
            return Err(error(
                "USD material texture exceeds the GPU's 2D texture limit",
            ));
        }
        Ok(())
    }
}

/// Directional key-light depth map settings. Biases are relative to the
/// caster-only depth range, and the filter radius is in map texels (0 to 64).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UsdShadowSettings {
    pub resolution: u32,
    /// Grow the light-space fit from the casters at 0 to all visible USD
    /// meshes at 1. The range gives finer control close to the casters.
    pub coverage: f32,
    /// Extra width and height in light space, in stage units. Split evenly
    /// across opposite edges of the fitted rectangle.
    pub coverage_expansion: f32,
    pub depth_bias: f32,
    pub slope_bias: f32,
    pub filter_radius: u32,
    /// Angular radius of the key in radians. Zero uses a fixed filter radius.
    /// With an area key, the filter radius caps the penumbra in map texels.
    pub light_radius: f32,
    /// Fraction of key light blocked by a fully occluded sample (0 to 1).
    pub strength: f32,
}

impl Default for UsdShadowSettings {
    fn default() -> Self {
        Self {
            resolution: 1024,
            coverage: 0.0,
            coverage_expansion: 0.0,
            depth_bias: 0.0004,
            slope_bias: 0.001,
            filter_radius: 1,
            light_radius: 0.0,
            strength: 1.0,
        }
    }
}

/// View-local studio display settings. The directional key is expressed in
/// studio coordinates: +Y is up, and +Z becomes -Y on a Z-up stage.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UsdStudioLighting {
    pub toward_key: [f32; 3],
    pub key_intensity: f32,
    /// Linear RGB multiplier for the shadow-casting key.
    pub key_tint: [f32; 3],
    /// Direction toward the unshadowed fill in the same studio coordinates.
    pub toward_fill: [f32; 3],
    pub fill_intensity: f32,
    /// Linear RGB multiplier for the fill.
    pub fill_tint: [f32; 3],
    pub ambient: f32,
    /// Strength of the approximate ground-facing ambient occlusion (0 to 1).
    pub occlusion: f32,
    /// Exposure compensation in stops.
    pub exposure: f32,
    /// Soft highlight compression (0 to 1).
    pub highlight_rolloff: f32,
    /// Add a neutral shadow receiver below the scene.
    pub floor: bool,
    /// `None` keeps the studio light and disables its depth pass.
    pub shadows: Option<UsdShadowSettings>,
}

impl Default for UsdStudioLighting {
    fn default() -> Self {
        Self {
            toward_key: [-0.55, 0.85, 0.45],
            key_intensity: 0.8,
            key_tint: [1.0; 3],
            toward_fill: [0.65, 0.55, 0.65],
            fill_intensity: 0.0,
            fill_tint: [1.0; 3],
            ambient: 0.3,
            occlusion: 0.25,
            exposure: 0.0,
            highlight_rolloff: 0.35,
            floor: true,
            shadows: Some(UsdShadowSettings::default()),
        }
    }
}

impl UsdStudioLighting {
    pub(super) fn validate(self) -> Result<()> {
        let valid_direction = |direction| {
            let length = Vec3::from_array(direction).length_squared();
            length.is_finite() && length > f32::MIN_POSITIVE
        };
        if !valid_direction(self.toward_key)
            || !valid_direction(self.toward_fill)
            || !self.fill_intensity.is_finite()
            || self.fill_intensity < 0.0
            || self
                .key_tint
                .iter()
                .chain(&self.fill_tint)
                .any(|v| !v.is_finite() || *v < 0.0)
            || !self.key_intensity.is_finite()
            || self.key_intensity < 0.0
            || !self.ambient.is_finite()
            || self.ambient < 0.0
            || !(0.0..=1.0).contains(&self.occlusion)
            || !(-16.0..=16.0).contains(&self.exposure)
            || !(0.0..=1.0).contains(&self.highlight_rolloff)
            || self.shadows.is_some_and(|s| {
                s.resolution == 0
                    || !(0.0..=1.0).contains(&s.coverage)
                    || !s.coverage_expansion.is_finite()
                    || s.coverage_expansion < 0.0
                    || !(0.0..=1.0).contains(&s.depth_bias)
                    || !(0.0..=1.0).contains(&s.slope_bias)
                    || s.filter_radius > 64
                    || !(0.0..=0.5).contains(&s.light_radius)
                    || !(0.0..=1.0).contains(&s.strength)
            })
        {
            return Err(error("invalid USD studio lighting or shadow settings"));
        }
        Ok(())
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct MaterialUniform {
    color: [f32; 4],
    features: [u32; 4],
    response: [f32; 4],
    uv_scale: [f32; 4],
}

pub(super) struct MaterialOverride {
    pub material: UsdMaterial,
    pub binding: Option<wgpu::BindGroup>,
}

pub(super) struct Materials {
    pub layout: wgpu::BindGroupLayout,
    pub default_binding: wgpu::BindGroup,
    white: wgpu::TextureView,
    sampler: wgpu::Sampler,
    pub max_dimension: u32,
}

impl Materials {
    pub fn new(gpu: &GpuContext) -> Self {
        let layout = gpu
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("USD surface material"),
                entries: &[
                    uniform_entry(2, wgpu::ShaderStages::FRAGMENT),
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 4,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });
        let white = color_view(gpu, [1, 1], &[255; 4]);
        let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("USD repeating surface sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let default_binding =
            material_binding(gpu, &layout, &white, &sampler, &UsdMaterial::default());
        Self {
            layout,
            default_binding,
            white,
            sampler,
            max_dimension: gpu.device.limits().max_texture_dimension_2d,
        }
    }

    pub fn binding(&self, gpu: &GpuContext, material: &UsdMaterial) -> wgpu::BindGroup {
        let view = material
            .color_texture
            .as_ref()
            .map(|t| color_view(gpu, t.size, &t.rgba));
        material_binding(
            gpu,
            &self.layout,
            view.as_ref().unwrap_or(&self.white),
            &self.sampler,
            material,
        )
    }
}

fn material_binding(
    gpu: &GpuContext,
    layout: &wgpu::BindGroupLayout,
    view: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
    material: &UsdMaterial,
) -> wgpu::BindGroup {
    let color = material.base_color.unwrap_or([0.0; 3]);
    let data = MaterialUniform {
        color: [
            color[0],
            color[1],
            color[2],
            f32::from(material.base_color.is_some()),
        ],
        features: [
            material.surface as u32,
            material.mapping as u32,
            u32::from(material.color_texture.is_some()),
            0,
        ],
        response: [
            material.roughness,
            material.sheen,
            material.weave_frequency,
            0.0,
        ],
        uv_scale: [material.uv_scale[0], material.uv_scale[1], 0.0, 0.0],
    };
    let buffer = gpu
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("USD surface parameters"),
            contents: bytemuck::bytes_of(&data),
            usage: wgpu::BufferUsages::UNIFORM,
        });
    gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("USD surface material"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 2,
                resource: buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(view),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

fn color_view(gpu: &GpuContext, [width, height]: [u32; 2], rgba: &[u8]) -> wgpu::TextureView {
    let size = wgpu::Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
    };
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("USD sRGB base color"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    gpu.queue.write_texture(
        texture.as_image_copy(),
        rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4 * width),
            rows_per_image: Some(height),
        },
        size,
    );
    texture.create_view(&Default::default())
}

pub(super) fn uniform_entry(
    binding: u32,
    visibility: wgpu::ShaderStages,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}
