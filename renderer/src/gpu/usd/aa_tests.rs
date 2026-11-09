use super::*;

#[test]
#[ignore = "requires a hardware GPU; reads back only test images"]
fn strip_resolve_matches_full_msaa_and_preserves_premultiplied_coverage() {
    check_strip_resolve(wgpu::TextureFormat::Rgba8Unorm);
}

#[test]
#[ignore = "requires a hardware GPU; reads back only test images"]
fn strip_resolve_matches_full_msaa_on_bgra_metal_surfaces() {
    check_strip_resolve(wgpu::TextureFormat::Bgra8Unorm);
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "GPU timing benchmark; requires a hardware GPU"]
fn metal_msaa_frame_cost() {
    let instance = wgpu::Instance::default();
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    if adapter.get_info().transient_saves_memory != Some(true) {
        eprintln!("Memoryless AA benchmark requires an adapter with tile-memory attachments");
        return;
    }
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    let gpu = GpuContext {
        adapter,
        device,
        queue,
    };
    let info = GpuViewportInfo {
        bounds: cssimpler_core::LayoutBox::new(0.0, 0.0, 800.0, 600.0),
        physical_size: [1600, 1200],
        scale_factor: 2.0,
        format: wgpu::TextureFormat::Bgra8Unorm,
        alpha_mode: wgpu::CompositeAlphaMode::PreMultiplied,
        background: None,
        texture_usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_DST,
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
    let image = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("AA benchmark output"),
        size: wgpu::Extent3d {
            width: 1600,
            height: 1200,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: info.format,
        usage: info.texture_usage,
        view_formats: &[],
    });
    let view = image.create_view(&Default::default());
    let mut measurements = [(0u128, 0u128); 2];
    // Alternate strategies to reduce warmup/order bias. No vsync or readback;
    // completion time includes CPU encoding, submission, and waiting for the GPU.
    for round in 0..4 {
        for (index, memoryless) in [false, true].into_iter().enumerate() {
            renderer.targets = aa::Targets::with_memoryless(&gpu, info, memoryless);
            if memoryless {
                assert!(
                    renderer
                        .targets
                        .depth
                        .usage()
                        .contains(wgpu::TextureUsages::TRANSIENT_ATTACHMENT)
                );
            }
            for frame_index in 0..40 {
                let start = std::time::Instant::now();
                let mut encoder = gpu.device.create_command_encoder(&Default::default());
                renderer
                    .render(
                        &gpu,
                        GpuViewportFrame {
                            info,
                            frame: FrameInfo {
                                frame_index,
                                delta: Default::default(),
                            },
                            target: &view,
                            texture: &image,
                            encoder: &mut encoder,
                        },
                    )
                    .unwrap();
                let command = encoder.finish();
                let encoded = start.elapsed();
                gpu.queue.submit([command]);
                gpu.device
                    .poll(wgpu::PollType::wait_indefinitely())
                    .unwrap();
                if round != 0 {
                    measurements[index].0 += encoded.as_micros();
                    measurements[index].1 += start.elapsed().as_micros();
                }
            }
        }
    }
    for (label, (encode, complete)) in ["strips", "memoryless"].into_iter().zip(measurements) {
        eprintln!(
            "Metal AA 1600x1200 {label}: mean_encode_us={} mean_complete_us={}",
            encode / 120,
            complete / 120
        );
    }
}

fn check_strip_resolve(format: wgpu::TextureFormat) {
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
        format,
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
    // Exercise the strip fallback even on Apple GPUs that use a direct resolve.
    renderer.targets = aa::Targets::with_memoryless(&gpu, info, false);
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
    let native_targets = aa::Targets::new(&gpu, info);
    if native_targets
        .depth
        .usage()
        .contains(wgpu::TextureUsages::TRANSIENT_ATTACHMENT)
    {
        assert_eq!(native_targets.height, info.physical_size[1]);
        assert!(native_targets.resolve.is_none());
        assert!(
            native_targets
                .color
                .as_ref()
                .unwrap()
                .usage()
                .contains(wgpu::TextureUsages::TRANSIENT_ATTACHMENT)
        );
        renderer.targets = native_targets;
        let native = draw(&mut renderer);
        assert!(
            native
                .iter()
                .zip(&reference)
                .all(|(a, b)| a.abs_diff(*b) <= 2),
            "memoryless direct resolve must match full-frame MSAA"
        );
    }
    let different: Vec<_> = strips
        .chunks_exact(4)
        .zip(reference.chunks_exact(4))
        .filter(|(a, b)| a.iter().zip(*b).any(|(a, b)| a.abs_diff(*b) > 2))
        .collect();
    // Metal can choose the other face at a shared triangle-edge depth tie
    // after strip projection. Permit one color-only edge pixel in this image;
    // coverage must still match, and interior pixels and strip seams must match.
    let allowed = usize::from(cfg!(target_os = "macos"));
    assert!(
        different.len() <= allowed,
        "strip/full-frame differences: {different:?}"
    );
    for (actual, expected) in different {
        assert_eq!(actual[3], expected[3], "edge coverage must match exactly");
        assert!(
            actual[3] > 0 && actual[3] < 255,
            "interior color must match"
        );
    }
}
