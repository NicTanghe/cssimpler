//! Wireframe coverage tests. They read back only test images and need a GPU.
use super::*;

fn test_instance() -> wgpu::Instance {
    wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: if cfg!(windows) {
            wgpu::Backends::DX12
        } else {
            wgpu::Backends::all()
        },
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    })
}

// 256 px keeps the readback row stride aligned to COPY_BYTES_PER_ROW_ALIGNMENT.
const SIZE: [u32; 2] = [256, 160];

/// Opaque black pixels are produced only by the stroke, because the meshes
/// shade to non-zero colours on a transparent background.
fn stroke_pixels(pixels: &[u8]) -> impl Iterator<Item = &[u8]> {
    pixels
        .chunks_exact(4)
        .filter(|pixel| pixel[0] == 0 && pixel[1] == 0 && pixel[2] == 0 && pixel[3] == 255)
}

#[test]
#[ignore = "requires a hardware GPU; reads back only test images"]
fn wireframe_is_per_model_and_straddles_the_silhouette() {
    let instance = test_instance();
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    let gpu = GpuContext {
        adapter,
        device,
        queue,
    };
    let info = GpuViewportInfo {
        bounds: cssimpler_core::LayoutBox::new(0.0, 0.0, SIZE[0] as f32, SIZE[1] as f32),
        physical_size: SIZE,
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
        "/../examples/assets/usd-two-models.usda"
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
    assert_eq!(
        renderer.models(),
        ["/World/Left".to_owned(), "/World/Right".to_owned()]
    );
    // A wider band guarantees fully covered pixels at any subpixel placement.
    renderer.set_wireframe_style(WireframeStyle {
        width_px: 4.0,
        ..WireframeStyle::default()
    });
    let image = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("wireframe test presentation image"),
        size: wgpu::Extent3d {
            width: SIZE[0],
            height: SIZE[1],
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
            label: Some("wireframe test readback"),
            size: (SIZE[0] * SIZE[1] * 4) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            image.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(SIZE[0] * 4),
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

    let shaded = draw(&mut renderer);
    assert_eq!(stroke_pixels(&shaded).count(), 0, "wireframe is off");

    renderer.set_wireframe(WireframeMode::Edges);
    let all_models = draw(&mut renderer);
    let all = stroke_pixels(&all_models).count();
    assert!(all > 0, "edges mode must draw the cage");

    // Every stroke pixel is opaque black; the band must land on both the shaded
    // surface and the background outside the silhouette.
    let mut inside = 0;
    let mut outside = 0;
    for (stroke, shaded) in all_models.chunks_exact(4).zip(shaded.chunks_exact(4)) {
        if stroke == [0, 0, 0, 255] {
            if shaded[3] == 0 {
                outside += 1;
            } else {
                inside += 1;
            }
        }
    }
    assert!(inside > 0, "expected the stroke on the shaded surface");
    assert!(outside > 0, "expected the stroke outside the silhouette");

    // Restricting the stroke to one model must remove the other model's edges.
    renderer.set_model_wireframe(
        "/World/Left",
        Wireframe {
            mode: WireframeMode::Off,
            ..Wireframe::default()
        },
    );
    let left_off = stroke_pixels(&draw(&mut renderer)).count();
    assert!(
        left_off > 0 && left_off < all,
        "per-model override must keep the right model and drop the left: {left_off} vs {all}"
    );
    renderer.set_model_wireframe(
        "/World/Right",
        Wireframe {
            mode: WireframeMode::Off,
            ..Wireframe::default()
        },
    );
    assert_eq!(
        stroke_pixels(&draw(&mut renderer)).count(),
        0,
        "both models off must draw nothing"
    );

    // Silhouette mode draws a strict subset of the cage edges.
    renderer.clear_model_wireframe("/World/Left");
    renderer.clear_model_wireframe("/World/Right");
    renderer.set_wireframe(WireframeMode::Silhouette);
    let silhouette = stroke_pixels(&draw(&mut renderer)).count();
    assert!(
        silhouette > 0 && silhouette < all,
        "silhouette must be a non-empty subset of every edge: {silhouette} vs {all}"
    );
}
