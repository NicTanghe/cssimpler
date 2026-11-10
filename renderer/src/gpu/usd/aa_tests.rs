use super::*;

fn test_instance() -> wgpu::Instance {
    // Match Windows DirectComposition's backend instead of the default Vulkan
    // adapter selected by headless request_adapter on multi-backend systems.
    wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: if cfg!(windows) {
            wgpu::Backends::DX12
        } else {
            wgpu::Backends::all()
        },
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    })
}

#[test]
#[ignore = "requires a hardware GPU; reads back only test images"]
fn strip_resolve_matches_full_msaa_and_preserves_premultiplied_coverage() {
    check_strip_resolve(wgpu::TextureFormat::Rgba8Unorm);
}

#[test]
#[ignore = "requires a hardware GPU; reads back only test images"]
fn strip_resolve_matches_full_msaa_on_bgra_surfaces() {
    check_strip_resolve(wgpu::TextureFormat::Bgra8Unorm);
}

#[test]
#[ignore = "requires a hardware GPU; reads back only test images"]
fn strip_resolve_matches_full_msaa_on_srgb_surfaces() {
    check_strip_resolve(wgpu::TextureFormat::Rgba8UnormSrgb);
    check_strip_resolve(wgpu::TextureFormat::Bgra8UnormSrgb);
}

#[test]
#[ignore = "GPU timing benchmark; requires a hardware GPU"]
fn usd_msaa_frame_cost() {
    for size in [[800, 600], [1600, 1200]] {
        measure_msaa_frame_cost(size);
    }
}

fn measure_msaa_frame_cost(size: [u32; 2]) {
    let instance = test_instance();
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    eprintln!("AA benchmark adapter: {:?}", adapter.get_info());
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    let gpu = GpuContext {
        adapter,
        device,
        queue,
    };
    let info = GpuViewportInfo {
        bounds: cssimpler_core::LayoutBox::new(0.0, 0.0, 800.0, 600.0),
        physical_size: size,
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
            width: size[0],
            height: size[1],
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
    let mut strategies = vec![
        ("single-sample-uncached", aa::Strategy::FullFrame, true),
        ("strips", aa::Strategy::Strips, false),
        ("full-frame", aa::Strategy::FullFrame, false),
        ("full-frame-uncached", aa::Strategy::FullFrame, true),
    ];
    if cfg!(target_os = "macos") && gpu.adapter.get_info().transient_saves_memory == Some(true) {
        strategies.push(("memoryless", aa::Strategy::Memoryless, false));
    }
    let mut measurements = vec![(0u128, 0u128); strategies.len()];
    // Alternate strategies to reduce warmup/order bias. No vsync or readback;
    // completion time includes CPU encoding, submission, and waiting for the GPU.
    for round in 0..4 {
        let mut order: Vec<_> = (0..strategies.len()).collect();
        if round % 2 != 0 {
            order.reverse();
        }
        for index in order {
            let (_, strategy, uncached) = strategies[index];
            renderer.targets = aa::Targets::with_strategy(&gpu, info, strategy);
            if index == 0 {
                renderer.targets.depth = gpu.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("Single-sample benchmark depth"),
                    size: image.size(),
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Depth32Float,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                });
                renderer.targets.color = None;
                renderer.targets.samples = 1;
            }
            renderer.pipeline = mesh_pipeline(
                &gpu,
                &renderer.pipeline_layout,
                info,
                renderer.targets.samples,
            );
            for frame_index in 0..40 {
                if uncached {
                    // Reproduce the old per-frame USD camera evaluation/upload.
                    renderer.camera_key = None;
                }
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
    for ((label, _, _), (encode, complete)) in strategies.into_iter().zip(measurements) {
        eprintln!(
            "USD AA {}x{} {label}: mean_encode_us={} mean_complete_us={}",
            size[0],
            size[1],
            encode / 120,
            complete / 120
        );
    }
}

fn check_strip_resolve(format: wgpu::TextureFormat) {
    let instance = test_instance();
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
    renderer.targets = aa::Targets::with_strategy(&gpu, info, aa::Strategy::Strips);
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
    if !format.is_srgb() {
        assert!(
            strips
                .chunks_exact(4)
                .all(|p| p[..3].iter().all(|c| *c <= p[3])),
            "RGB must be premultiplied by coverage"
        );
    }

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
    if native_targets.samples == 4 && native_targets.resolve.is_none() {
        assert_eq!(native_targets.height, info.physical_size[1]);
        assert!(native_targets.resolve.is_none());
        renderer.targets = native_targets;
        let native = draw(&mut renderer);
        assert!(
            native
                .iter()
                .zip(&reference)
                .all(|(a, b)| a.abs_diff(*b) <= 2),
            "native direct resolve must match full-frame MSAA"
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

    // A cached uniform must still follow orbit input, authored camera edits,
    // animation time and target changes. Repeated frames retain the same key.
    let before = draw(&mut renderer);
    let key = renderer.camera_key;
    assert_eq!(before, draw(&mut renderer));
    assert!(key == renderer.camera_key);
    assert!(renderer.event(&EngineEvent::Scroll {
        delta: crate::ScrollDelta::Lines { x: 0.0, y: 1.0 },
        modifiers: Default::default(),
    }));
    assert_ne!(before, draw(&mut renderer));
    assert!(key != renderer.camera_key);

    let before = draw(&mut renderer);
    let camera = geom::Camera::get(&renderer.camera_stage, &renderer.camera_path)
        .unwrap()
        .unwrap();
    camera
        .create_focal_length_attr()
        .unwrap()
        .set(20.0f32)
        .unwrap();
    assert_ne!(before, draw(&mut renderer));

    renderer.control().set_time(Some(12.0)).unwrap();
    draw(&mut renderer);
    assert!(renderer.camera_key.unwrap().time == Some(scene::TimeCode::new(12.0)));

    let depth = renderer.targets.depth.clone();
    renderer
        .resize(
            &gpu,
            GpuViewportInfo {
                background: Some(cssimpler_core::Color::rgb(20, 30, 40)),
                ..info
            },
        )
        .unwrap();
    assert_eq!(
        depth, renderer.targets.depth,
        "backdrop changes must reuse attachments"
    );
    renderer
        .resize(
            &gpu,
            GpuViewportInfo {
                texture_usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                ..info
            },
        )
        .unwrap();
    if cfg!(windows) {
        assert_eq!(
            renderer.targets.samples, 4,
            "direct resolve needs no COPY_DST"
        );
        assert!(renderer.targets.resolve.is_none());
    }
}
