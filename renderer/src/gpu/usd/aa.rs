//! Tile memory on Apple GPUs, direct resolve on Windows, bounded strips elsewhere.
use super::*;

#[derive(Clone, Copy)]
pub(super) enum Strategy {
    Strips,
    FullFrame,
    Memoryless,
}

pub(super) struct Targets {
    pub depth: wgpu::Texture,
    pub color: Option<wgpu::Texture>,
    pub resolve: Option<wgpu::Texture>,
    pub height: u32,
    pub samples: u32,
}

impl Targets {
    pub fn new(gpu: &GpuContext, info: GpuViewportInfo) -> Self {
        let adapter = gpu.adapter.get_info();
        let memoryless = cfg!(target_os = "macos")
            && adapter.backend == wgpu::Backend::Metal
            && adapter.transient_saves_memory == Some(true);
        let strategy = if memoryless {
            Strategy::Memoryless
        } else if cfg!(windows) {
            // Avoid repeating vertex work and render-pass/copy overhead per strip.
            Strategy::FullFrame
        } else {
            Strategy::Strips
        };
        Self::with_strategy(gpu, info, strategy)
    }

    pub(super) fn with_strategy(
        gpu: &GpuContext,
        info: GpuViewportInfo,
        strategy: Strategy,
    ) -> Self {
        let color_flags = gpu.adapter.get_texture_format_features(info.format).flags;
        let depth_flags = gpu
            .adapter
            .get_texture_format_features(wgpu::TextureFormat::Depth32Float)
            .flags;
        let strips = matches!(strategy, Strategy::Strips);
        let enabled = matches!(
            info.format,
            wgpu::TextureFormat::Rgba8Unorm
                | wgpu::TextureFormat::Rgba8UnormSrgb
                | wgpu::TextureFormat::Bgra8Unorm
                | wgpu::TextureFormat::Bgra8UnormSrgb
        ) && (!strips || info.physical_size[1] >= 16)
            && (!strips || info.texture_usage.contains(wgpu::TextureUsages::COPY_DST))
            && color_flags.contains(
                wgpu::TextureFormatFeatureFlags::MULTISAMPLE_X4
                    | wgpu::TextureFormatFeatureFlags::MULTISAMPLE_RESOLVE,
            )
            && depth_flags.contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_X4);
        let samples = if enabled { 4 } else { 1 };
        let memoryless = enabled && matches!(strategy, Strategy::Memoryless);
        let height = if strips {
            strip_height(info.physical_size[1], enabled)
        } else {
            info.physical_size[1]
        };
        let attachment_usage = wgpu::TextureUsages::RENDER_ATTACHMENT
            | if memoryless {
                wgpu::TextureUsages::TRANSIENT_ATTACHMENT
            } else {
                wgpu::TextureUsages::empty()
            };
        let texture = |label, format, sample_count, usage| {
            gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: info.physical_size[0],
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        Self {
            depth: texture(
                "USD depth attachment",
                wgpu::TextureFormat::Depth32Float,
                samples,
                attachment_usage,
            ),
            color: enabled.then(|| {
                texture(
                    "USD MSAA attachment",
                    info.format,
                    samples,
                    attachment_usage,
                )
            }),
            resolve: (enabled && strips).then(|| {
                texture(
                    "USD resolved strip",
                    info.format,
                    1,
                    wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                )
            }),
            height,
            samples,
        }
    }
}

fn strip_height(height: u32, enabled: bool) -> u32 {
    if enabled {
        ((height - 4) / 9).clamp(1, 64)
    } else {
        height
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_attachments_and_uniform_growth_fit_old_depth_budget() {
        // RGBA8 x4 + Depth32 x4 + RGBA8 resolve = 36 bytes per strip pixel.
        // The old full-window Depth32 attachment cost 4 bytes per pixel.
        for height in 16..=16384u64 {
            for width in [1, 800, 1920, 3840, 16384u64] {
                let bytes = width * u64::from(strip_height(height as u32, true)) * 36 + 16;
                assert!(bytes <= width * height * 4, "{width}x{height}: {bytes}");
            }
        }
    }
}
