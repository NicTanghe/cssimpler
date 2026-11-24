use super::*;

const POSE_MESH: &str = "/Cloth";

fn points(fold: f32) -> Vec<gf::Vec3f> {
    vec![
        gf::vec3f(0., 0., 0.),
        gf::vec3f(1., 0., 0.),
        gf::vec3f(0., 1., 0.),
        gf::vec3f(1., 1., fold),
    ]
}

fn scene(flat: bool) -> UsdScene {
    let stage = Stage::builder()
        .schema_registry(openusd_schemas::schema_registry())
        .in_memory("deform-test.usda")
        .unwrap();
    for (path, vertices) in [(POSE_MESH, points(0.)), ("/Avatar", points(-0.5))] {
        let mesh = geom::Mesh::define(&stage, path).unwrap();
        mesh.create_points_attr().unwrap().set(vertices).unwrap();
        mesh.create_face_vertex_counts_attr()
            .unwrap()
            .set(vec![3_i32, 3])
            .unwrap();
        mesh.create_face_vertex_indices_attr()
            .unwrap()
            .set(vec![0_i32, 1, 2, 1, 3, 2])
            .unwrap();
    }
    if flat {
        geom::Mesh::get(&stage, POSE_MESH)
            .unwrap()
            .unwrap()
            .create_normals_attr()
            .unwrap()
            .set_interpolation(openusd::tf::Token::new("faceVarying"))
            .unwrap();
    }
    UsdScene::from_stage(stage)
        .unwrap()
        .with_deforming_meshes(POSE_MESH)
        .unwrap()
}

#[test]
fn position_only_edits_leave_topology_cached() {
    let scene = scene(false);
    let revision = scene.revision.get();
    let pose_revision = scene.pose_revision.get();
    assert!(
        scene
            .set_deformed_mesh_points("/Avatar", points(1.))
            .is_err()
    );
    scene
        .set_deformed_mesh_points(POSE_MESH, points(1.))
        .unwrap();
    assert_eq!(scene.revision.get(), revision);
    assert_eq!(scene.pose_revision.get(), pose_revision + 1);
    let cache = extract_scene_meshes(scene.stage(), None, scene.deform_root.as_ref()).unwrap();
    let expected = Vec3::new(-1., -1., 2.).normalize();
    assert!(
        cache
            .vertices
            .iter()
            .any(|v| Vec3::from_array(v.normal).abs_diff_eq(expected, 1e-6))
    );
    let attribute = scene.stage().attribute("/Cloth.faceVertexIndices").unwrap();
    attribute.set(vec![0_i32, 2, 1, 1, 2, 3]).unwrap();
    assert_ne!(scene.revision.get(), revision);
}

fn read_buffer<T: bytemuck::Pod>(gpu: &GpuContext, src: &wgpu::Buffer) -> Vec<T> {
    let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("deformation test readback"),
        size: src.size(),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(src, 0, &readback, 0, src.size());
    gpu.queue.submit([encoder.finish()]);
    let slice = readback.slice(..);
    let (send, receive) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        send.send(result).unwrap()
    });
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    receive.recv().unwrap().unwrap();
    slice
        .get_mapped_range()
        .unwrap()
        .chunks_exact(std::mem::size_of::<T>())
        .map(bytemuck::pod_read_unaligned)
        .collect()
}

#[test]
#[ignore = "requires a wgpu adapter with compute shaders; reads back test buffers"]
fn gpu_pose_updates_normals_and_wireframes_with_cached_geometry() {
    let instance = aa_tests::test_instance();
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    let gpu = GpuContext {
        adapter,
        device,
        queue,
    };
    let info = GpuViewportInfo {
        bounds: cssimpler_core::LayoutBox::new(0.0, 0.0, 128.0, 128.0),
        physical_size: [128, 128],
        scale_factor: 1.0,
        format: wgpu::TextureFormat::Rgba8Unorm,
        alpha_mode: wgpu::CompositeAlphaMode::PreMultiplied,
        background: None,
        texture_usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
    };
    let image = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("deformation test frame"),
        size: wgpu::Extent3d {
            width: 128,
            height: 128,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: info.format,
        usage: info.texture_usage,
        view_formats: &[],
    });
    for flat in [false, true] {
        let scene = scene(flat);
        let mut renderer = UsdRenderer::new(
            GpuViewportInit {
                gpu: &gpu,
                info,
                redraw: GpuRedrawHandle::default(),
            },
            scene.clone(),
            UsdCameraMode::Orbit,
        )
        .unwrap();
        assert!(renderer.deformation.is_some());
        renderer
            .set_studio_lighting(Some(UsdStudioLighting::default()))
            .unwrap();
        let original = aa_tests::read_frame(&gpu, info, &image, &mut renderer);
        let buffers = (
            renderer.vertices.clone(),
            renderer.indices.clone(),
            renderer.edges.clone(),
        );
        scene
            .set_deformed_mesh_points(POSE_MESH, points(1.))
            .unwrap();
        assert_ne!(
            original,
            aa_tests::read_frame(&gpu, info, &image, &mut renderer)
        );
        assert_eq!(
            (&renderer.vertices, &renderer.indices, &renderer.edges),
            (&buffers.0, &buffers.1, &buffers.2)
        );
        let expected =
            extract_scene_meshes(scene.stage(), None, scene.deform_root.as_ref()).unwrap();
        let vertices = read_buffer::<Vertex>(&gpu, &renderer.vertices);
        assert_eq!(vertices.len(), expected.vertices.len());
        for (actual, expected) in vertices.iter().zip(&expected.vertices) {
            assert_eq!(actual.position, expected.position);
            assert_eq!(actual.color, expected.color);
            assert_eq!(actual.uv, expected.uv);
            assert_eq!(actual.local, expected.local);
            assert!(
                Vec3::from_array(actual.normal)
                    .abs_diff_eq(Vec3::from_array(expected.normal), 1e-6)
            );
        }
        let edges = read_buffer::<EdgeRecord>(&gpu, &renderer.edges);
        assert_eq!(edges.len(), expected.edges.len());
        for (actual, expected) in edges.iter().zip(&expected.edges) {
            assert_eq!(actual.a, expected.a);
            assert_eq!(actual.b, expected.b);
            assert_eq!(actual.adjacency, expected.adjacency);
            assert!(
                glam::Vec4::from_array(actual.n0)
                    .abs_diff_eq(glam::Vec4::from_array(expected.n0), 1e-6)
            );
            assert!(
                glam::Vec4::from_array(actual.n1)
                    .abs_diff_eq(glam::Vec4::from_array(expected.n1), 1e-6)
            );
        }
    }
}
