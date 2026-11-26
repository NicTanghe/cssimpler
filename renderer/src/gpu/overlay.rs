//! Composite the CPU-painted layer over the native swapchain without reading
//! the viewport texture back. The same pixels determine pointer ownership.

use super::{GpuContext, GpuViewportInfo};
use crate::PointerPosition;
use cssimpler_core::{Color, RenderNode};

pub(super) struct Overlay {
    scene: Vec<RenderNode>,
    origin: [i32; 2],
    size: [u32; 2],
    scale: f64,
    alpha: Vec<u8>,
    pipeline: wgpu::RenderPipeline,
    texture: wgpu::Texture,
    binding: wgpu::BindGroup,
}

impl Overlay {
    pub fn sync(
        current: &mut Option<Self>,
        gpu: &GpuContext,
        info: GpuViewportInfo,
        origin: [i32; 2],
        scene: Vec<RenderNode>,
    ) -> bool {
        if scene.is_empty() {
            return current.take().is_some();
        }
        let size = info.physical_size;
        let scale = info.scale_factor;
        if let Some(layer) = current.as_ref()
            && layer.origin == origin
            && layer.size == size
            && layer.scale == scale
            && crate::prepare_scene_diff(&layer.scene, &scene)
                .dirty_regions
                .is_empty()
        {
            return false;
        }
        let layer = match current {
            Some(layer) if layer.size == size => layer,
            _ => current.insert(Self::new(gpu, info)),
        };
        let pixels = rasterize(&scene, origin, size, scale);
        layer.alpha.clear();
        layer
            .alpha
            .extend(pixels.chunks_exact(4).map(|rgba| rgba[3]));
        gpu.queue.write_texture(
            layer.texture.as_image_copy(),
            &pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * size[0]),
                rows_per_image: Some(size[1]),
            },
            layer.texture.size(),
        );
        layer.origin = origin;
        layer.scale = scale;
        layer.scene = scene;
        true
    }

    pub fn blocks(&self, point: PointerPosition) -> bool {
        alpha_at(&self.alpha, self.origin, self.size, self.scale, point) != 0
    }

    fn new(gpu: &GpuContext, info: GpuViewportInfo) -> Self {
        let device = &gpu.device;
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("CPU viewport overlay"),
            source: wgpu::ShaderSource::Wgsl(include_str!("overlay.wgsl").into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("CPU viewport overlay"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fragment"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: info.format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let size = info.physical_size;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("CPU viewport overlay"),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: if info.format.is_srgb() {
                wgpu::TextureFormat::Rgba8UnormSrgb
            } else {
                wgpu::TextureFormat::Rgba8Unorm
            },
            usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let binding = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("CPU viewport overlay"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(
                    &texture.create_view(&Default::default()),
                ),
            }],
        });
        Self {
            scene: Vec::new(),
            origin: [0, 0],
            size,
            scale: info.scale_factor,
            alpha: Vec::new(),
            pipeline,
            texture,
            binding,
        }
    }

    pub fn draw(&self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("CPU viewport overlay"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.binding, &[]);
        pass.draw(0..3, 0..1);
    }
}

fn rasterize(scene: &[RenderNode], origin: [i32; 2], size: [u32; 2], scale: f64) -> Vec<u8> {
    let local = scene
        .iter()
        .map(|node| {
            crate::translated_render_subtree(
                node,
                (-f64::from(origin[0]) / scale) as f32,
                (-f64::from(origin[1]) / scale) as f32,
            )
        })
        .collect::<Vec<_>>();
    let [width, height] = size.map(|extent| extent as usize);
    let mut rgba = vec![0; width * height * 4];
    crate::render_to_rgba8_at_scale(
        &local,
        &mut rgba,
        width,
        height,
        Color::rgba(0, 0, 0, 0),
        scale as f32,
    );
    rgba
}

fn alpha_at(
    alpha: &[u8],
    origin: [i32; 2],
    size: [u32; 2],
    scale: f64,
    point: PointerPosition,
) -> u8 {
    let x = (f64::from(point.x) * scale).floor() - f64::from(origin[0]);
    let y = (f64::from(point.y) * scale).floor() - f64::from(origin[1]);
    if x >= 0.0 && x < f64::from(size[0]) && y >= 0.0 && y < f64::from(size[1]) {
        alpha[y as usize * size[0] as usize + x as usize]
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cssimpler_core::{CornerRadius, LayoutBox, ZIndex};

    #[test]
    fn overlay_alpha_owns_input_only_on_painted_pixels_at_display_scale() {
        let origin = [20, 30];
        let size = [40, 40];
        let scale = 2.0;
        let mut panel = RenderNode::container(LayoutBox::new(15.0, 20.0, 10.0, 10.0));
        panel.style.background = Some(Color::rgba(50, 100, 150, 128));
        panel.style.corner_radius = CornerRadius::all(4.0);
        let pixels = rasterize(&[panel], origin, size, scale);
        let alpha = pixels
            .chunks_exact(4)
            .map(|rgba| rgba[3])
            .collect::<Vec<_>>();
        let at = |x, y| alpha_at(&alpha, origin, size, scale, PointerPosition { x, y });
        assert_eq!(at(9.9, 22.0), 0);
        assert_eq!(at(12.0, 22.0), 0);
        assert_eq!(at(15.0, 20.0), 0);
        assert_eq!(at(20.0, 25.0), 128);
        assert_eq!(at(27.0, 22.0), 0);
    }

    #[test]
    fn modal_dimmer_and_panel_paint_above_the_viewport_in_css_order() {
        let window = LayoutBox::new(0.0, 0.0, 100.0, 100.0);
        let bounds = LayoutBox::new(10.0, 10.0, 60.0, 60.0);
        let mut app = RenderNode::container(window);
        app.style.background = Some(Color::rgb(10, 60, 20));
        let mut viewport = RenderNode::container(bounds).with_element_id("scene");
        viewport.style.background = Some(Color::rgb(90, 30, 10));
        let mut modal = RenderNode::container(window);
        modal.style.positioned = true;
        modal.style.z_index = ZIndex::Integer(1000);
        let mut dimmer = RenderNode::container(window);
        dimmer.style.background = Some(Color::rgba(0, 0, 0, 64));
        let mut panel = RenderNode::container(LayoutBox::new(25.0, 25.0, 40.0, 50.0));
        panel.style.background = Some(Color::rgb(230, 240, 250));
        panel.style.positioned = true;
        panel.style.z_index = ZIndex::Integer(1);
        modal.children.extend([dimmer, panel]);
        app.children.extend([viewport, modal]);

        let location = super::super::geometry::locate(&[app], "scene", window, Color::BLACK)
            .unwrap()
            .unwrap();
        assert_eq!(location.bounds, bounds);
        let pixels = rasterize(&location.overlay, [10, 10], [60, 60], 1.0);
        assert_eq!(&pixels[..4], &[0, 0, 0, 64]);
        let center = 4 * (30 * 60 + 30);
        assert_eq!(&pixels[center..center + 4], &[230, 240, 250, 255]);
    }
}
