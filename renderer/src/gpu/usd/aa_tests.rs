use super::*;

#[test]
#[ignore = "requires a hardware GPU; reads back only test images"]
fn strip_resolve_matches_full_msaa_and_preserves_premultiplied_coverage() {
    let instance = wgpu::Instance::default();
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    let gpu = GpuContext {
        adapter,
        device,
        queue,
    };
    let info = GpuViewportInfo {
        bounds: cssimpler_core::LayoutBox::new(0.0, 0.0, 128.0, 117.0),
        physical_size: [128, 117],
        scale_factor: 1.0,
        format: wgpu::TextureFormat::Rgba8Unorm,
        alpha_mode: wgpu::CompositeAlphaMode::PreMultiplied,
        background: None,
        texture_usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_DST
            | wgpu::TextureUsages::COPY_SRC,
    };
    let scene = UsdScene::open(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../examples/assets/usd-preview.usda"
    ))
    .unwrap();
    let mut renderer = UsdRenderer::new(
        GpuViewportInit {
            gpu: &gpu,
            info,
            redraw: GpuRedrawHandle::default(),
        },
        scene,
        UsdCameraMode::Orbit,
    )
    .unwrap();
    assert_eq!(renderer.targets.samples, 4);
    let image = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("AA test presentation image"),
        size: wgpu::Extent3d {
            width: 128,
            height: 117,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: info.format,
        usage: info.texture_usage,
        view_formats: &[],
    });
    let draw = |renderer: &mut UsdRenderer| {
        let view = image.create_view(&Default::default());
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        renderer
            .render(
                &gpu,
                GpuViewportFrame {
                    info,
                    frame: FrameInfo {
                        frame_index: 0,
                        delta: Default::default(),
                    },
                    target: &view,
                    texture: &image,
                    encoder: &mut encoder,
                },
            )
            .unwrap();
        let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("AA test readback"),
            size: 128 * 117 * 4,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            image.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(512),
                    rows_per_image: None,
                },
            },
            image.size(),
        );
        gpu.queue.submit([encoder.finish()]);
        let (send, receive) = std::sync::mpsc::channel();
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                send.send(result).unwrap();
            });
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        receive.recv().unwrap().unwrap();
        let pixels = buffer.slice(..).get_mapped_range().unwrap().to_vec();
        buffer.unmap();
        pixels
    };
    let strips = draw(&mut renderer);
    assert!(
        strips.chunks_exact(4).any(|p| p[3] > 0 && p[3] < 255),
        "expected fractional edge coverage"
    );
    assert!(
        strips.chunks_exact(4).any(|p| p[3] == 255),
        "expected an opaque cube interior"
    );
    assert_eq!(&strips[..4], &[0, 0, 0, 0]);
    assert!(
        strips
            .chunks_exact(4)
            .all(|p| p[..3].iter().all(|c| *c <= p[3])),
        "RGB must be premultiplied by coverage"
    );

    // Test-only full-frame reference catches strip seams and last-strip clipping.
    let texture = |format, samples| {
        gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("AA full-frame test reference"),
            size: image.size(),
            mip_level_count: 1,
            sample_count: samples,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
    };
    renderer.targets.depth = texture(wgpu::TextureFormat::Depth32Float, 4);
    renderer.targets.color = Some(texture(info.format, 4));
    renderer.targets.height = 117;
    renderer.targets.resolve = Some(gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("AA full-frame test resolve"),
        size: image.size(),
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: info.format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    }));
    let reference = draw(&mut renderer);
    let different = strips
        .iter()
        .zip(&reference)
        .filter(|(a, b)| a.abs_diff(**b) > 2)
        .count();
    assert_eq!(different, 0, "strip rendering must match full-frame MSAA");
}
