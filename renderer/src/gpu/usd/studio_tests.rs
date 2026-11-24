use super::*;

fn fixture() -> UsdScene {
    UsdScene::open(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../tests/fixtures/usd-studio-shadows.usda"
    ))
    .unwrap()
}

#[test]
fn studio_fixture_keeps_prim_ranges_and_flattened_uvs() {
    let scene = fixture();
    let cache = extract_meshes(&scene.stage, None).unwrap();
    let range_for = |path| {
        let index = cache.models.iter().position(|model| model == path).unwrap();
        let range = &cache.ranges[index];
        cache.indices[range.start as usize..range.end as usize]
            .iter()
            .map(|i| cache.vertices[*i as usize])
            .collect::<Vec<_>>()
    };
    let receiver = range_for("/Studio/Receiver");
    assert_eq!(
        receiver.iter().map(|v| v.uv).collect::<Vec<_>>(),
        [
            [0.0, 0.0, 1.0],
            [0.0, 1.0, 1.0],
            [1.0, 1.0, 1.0],
            [0.0, 0.0, 1.0],
            [1.0, 1.0, 1.0],
            [1.0, 0.0, 1.0],
        ]
    );
    let caster_index = cache
        .models
        .iter()
        .position(|p| p == "/Studio/Caster")
        .unwrap();
    assert_eq!(
        cache.shadow_ranges,
        vec![cache.ranges[caster_index].clone()]
    );
    assert_eq!(
        cache.shadow_bounds.unwrap(),
        [Vec3::new(-0.25, 0.8, -0.3), Vec3::new(0.35, 1.1, 0.3),]
    );
    let caster = range_for("/Studio/Caster");
    assert_eq!(caster.len(), 36);
    // Four receiver vertices plus four vertices for each hard-edged box face.
    assert_eq!(cache.vertices.len(), 4 + 6 * 4);
    assert!(caster.iter().all(|v| v.uv == [0.0; 3]));
    assert!(caster.iter().any(|v| v.local[1] < 0.0));
    assert!(caster.iter().any(|v| v.local[1] > 0.0));
}

#[test]
#[ignore = "requires a wgpu adapter with sampled depth textures; reads back test frames"]
fn studio_shadows_reach_mesh_and_floor() {
    let instance = aa_tests::test_instance();
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    let gpu = GpuContext {
        adapter,
        device,
        queue,
    };
    let info = GpuViewportInfo {
        bounds: cssimpler_core::LayoutBox::new(0.0, 0.0, 256.0, 256.0),
        physical_size: [256, 256],
        scale_factor: 1.0,
        format: wgpu::TextureFormat::Rgba8Unorm,
        alpha_mode: wgpu::CompositeAlphaMode::PreMultiplied,
        background: None,
        texture_usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
    };
    let scene = fixture();
    let redraw = GpuRedrawHandle::default();
    let mut orbit_renderer = UsdRenderer::new(
        GpuViewportInit {
            gpu: &gpu,
            info,
            redraw: GpuRedrawHandle::default(),
        },
        scene.clone(),
        UsdCameraMode::Orbit,
    )
    .unwrap();
    let camera_stage = orbit_renderer.camera_stage.clone();
    let camera_path = orbit_renderer.camera_path.clone();
    let mut orbit = orbit_renderer.orbit.take().unwrap();
    drop(orbit_renderer);
    let mut renderer = UsdRenderer::new_with_authored_camera(
        GpuViewportInit {
            gpu: &gpu,
            info,
            redraw: redraw.clone(),
        },
        scene,
        camera_stage.clone(),
        &camera_path.to_string(),
    )
    .unwrap();
    assert!(renderer.supports_shadow_mapping());
    let image = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("USD studio test frame"),
        size: wgpu::Extent3d {
            width: 256,
            height: 256,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: info.format,
        usage: info.texture_usage,
        view_formats: &[],
    });
    let draw = |renderer: &mut UsdRenderer| aa_tests::read_frame(&gpu, info, &image, renderer);
    let legacy = draw(&mut renderer);
    let studio = UsdStudioLighting {
        toward_key: [-0.45, 0.95, -0.62],
        shadows: Some(UsdShadowSettings {
            filter_radius: 5,
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut cloth = UsdMaterial::fabric([0.70, 0.09, 0.05]);
    renderer
        .set_model_material("/Studio/Receiver", cloth.clone())
        .unwrap();
    renderer
        .set_studio_lighting(Some(UsdStudioLighting {
            shadows: None,
            ..studio
        }))
        .unwrap();
    renderer
        .set_model_material(
            "/Studio/Caster",
            UsdMaterial::brushed_metal([0.55, 0.65, 0.95]),
        )
        .unwrap();
    let untextured = draw(&mut renderer);
    cloth.color_texture = Some(
        UsdColorTexture::new(
            [2, 2],
            vec![
                255, 255, 255, 255, 210, 180, 160, 255, 210, 180, 160, 255, 255, 255, 255, 255,
            ],
        )
        .unwrap(),
    );
    renderer
        .set_model_material("/Studio/Receiver", cloth)
        .unwrap();
    let without = draw(&mut renderer);
    assert!(
        untextured
            .chunks_exact(4)
            .zip(without.chunks_exact(4))
            .any(|(plain, textured)| {
                plain[3] == 255
                    && i32::from(plain[0]) - i32::from(plain[1]) > 45
                    && plain[0].saturating_sub(textured[0]) > 8
            }),
        "the receiver must sample the color texture"
    );

    renderer
        .set_studio_lighting(Some(UsdStudioLighting {
            floor: false,
            shadows: None,
            ..studio
        }))
        .unwrap();
    let no_floor = draw(&mut renderer);
    assert!(
        no_floor
            .chunks_exact(4)
            .any(|p| p[3] == 255 && p[2].saturating_sub(p[0]) > 24),
        "metal caster must use its blue tint"
    );

    renderer.set_studio_lighting(Some(studio)).unwrap();
    let with = draw(&mut renderer);
    assert_eq!(
        with,
        draw(&mut renderer),
        "cached depth map must render identically"
    );

    let close = studio.shadows.unwrap();
    renderer
        .set_shadow_settings(UsdShadowSettings {
            coverage: 1.0,
            light_radius: 0.12,
            ..close
        })
        .unwrap();
    let wide = draw(&mut renderer);
    let wide_receiver_shadow = without
        .chunks_exact(4)
        .zip(wide.chunks_exact(4))
        .filter(|(lit, shadow)| {
            lit[3] == 255
                && shadow[3] == 255
                && i32::from(lit[0]) - i32::from(lit[1]) > 45
                && lit[0].saturating_sub(shadow[0]) > 24
        })
        .count();
    assert!(
        wide_receiver_shadow > 50,
        "expected a shadow in the full fit"
    );
    renderer.set_shadow_settings(close).unwrap();
    assert_eq!(
        with,
        draw(&mut renderer),
        "restoring close fit must restore its shadow"
    );

    let mut receiver_shadow = 0;
    let mut floor_shadow = 0;
    for ((lit, shadow), floorless) in without
        .chunks_exact(4)
        .zip(with.chunks_exact(4))
        .zip(no_floor.chunks_exact(4))
    {
        if lit[3] == 255 && shadow[3] == 255 && lit[0].saturating_sub(shadow[0]) > 24 {
            if floorless[3] == 0 {
                floor_shadow += 1;
            } else if i32::from(lit[0]) - i32::from(lit[1]) > 45 {
                receiver_shadow += 1;
            }
        }
    }
    assert!(
        receiver_shadow > 50,
        "expected a shadow on the fabric receiver, found {receiver_shadow} pixels"
    );
    assert!(
        floor_shadow > 50,
        "expected a shadow on the floor, found {floor_shadow} pixels"
    );

    let vertices = renderer.vertices.clone();
    let indices = renderer.indices.clone();
    redraw.take_dirty();
    let yaw = orbit.yaw;
    orbit.yaw += 0.25;
    update_orbit_camera(&camera_stage, &camera_path, &orbit).unwrap();
    assert!(redraw.take_dirty(), "camera edits must wake the viewport");
    assert_ne!(with, draw(&mut renderer));
    assert_eq!(renderer.vertices, vertices);
    assert_eq!(renderer.indices, indices);
    orbit.yaw = yaw;
    update_orbit_camera(&camera_stage, &camera_path, &orbit).unwrap();

    renderer.clear_model_material("/Studio/Receiver");
    renderer.clear_model_material("/Studio/Caster");
    renderer.set_studio_lighting(None).unwrap();
    assert_eq!(
        legacy,
        draw(&mut renderer),
        "clearing overrides must restore the original viewport"
    );
}
