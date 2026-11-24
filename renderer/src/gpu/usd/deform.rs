//! Cached triangle and edge mappings for position-only USD updates.
use super::*;
use std::mem::size_of;

const JOB_WORDS: usize = 5;
const WORKGROUP_SIZE: u32 = 64;

#[derive(Clone, Copy)]
pub(super) struct Bounds {
    pub scene: [Vec3; 2],
    pub casters: Option<[Vec3; 2]>,
}

impl Default for Bounds {
    fn default() -> Self {
        Self {
            scene: [Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)],
            casters: None,
        }
    }
}

impl Bounds {
    pub fn include(&mut self, point: Vec3, casts_shadow: bool) {
        self.scene[0] = self.scene[0].min(point);
        self.scene[1] = self.scene[1].max(point);
        if casts_shadow {
            let limits = self.casters.get_or_insert([point; 2]);
            limits[0] = limits[0].min(point);
            limits[1] = limits[1].max(point);
        }
    }
}

struct Mesh {
    path: sdf::Path,
    count: usize,
    casts_shadow: bool,
}

pub(super) struct MeshJobs {
    base_point: u32,
    center: u32,
    base_face: u32,
    base_edge: usize,
    incidence: Option<Vec<Range<u32>>>,
}

#[derive(Default)]
pub(super) struct Plan {
    meshes: Vec<Mesh>,
    point_count: usize,
    faces: Vec<[u32; 3]>,
    vertex_jobs: Vec<[u32; JOB_WORDS]>,
    edge_jobs: Vec<[u32; JOB_WORDS]>,
    incidence: Vec<u32>,
    pub static_bounds: Bounds,
    pub unsupported: bool,
}

impl Plan {
    pub fn add_mesh(
        &mut self,
        path: sdf::Path,
        point_count: usize,
        indices: &[i32],
        flat: bool,
        casts_shadow: bool,
    ) -> MeshJobs {
        let base_point = self.point_count as u32;
        let center = base_point + point_count as u32;
        let base_face = self.faces.len() as u32;
        self.point_count += point_count + 1;
        self.meshes.push(Mesh {
            path,
            count: point_count,
            casts_shadow,
        });
        let mut incident_faces = if flat {
            Vec::new()
        } else {
            vec![Vec::new(); point_count]
        };
        for (face, triangle) in indices.chunks_exact(3).enumerate() {
            let [a, b, c] = [triangle[0] as u32, triangle[1] as u32, triangle[2] as u32];
            self.faces
                .push([a + base_point, b + base_point, c + base_point]);
            if !flat {
                for point in [a, b, c] {
                    incident_faces[point as usize].push(base_face + face as u32);
                }
            }
        }
        let incidence = (!flat).then(|| {
            incident_faces
                .into_iter()
                .map(|faces| {
                    let first = self.incidence.len() as u32;
                    self.incidence.extend(faces);
                    first..self.incidence.len() as u32
                })
                .collect()
        });
        MeshJobs {
            base_point,
            center,
            base_face,
            base_edge: self.edge_jobs.len(),
            incidence,
        }
    }

    pub fn add_vertex(&mut self, mesh: &MeshJobs, vertex: u32, point: usize, face: usize) {
        let (first, count) = mesh
            .incidence
            .as_ref()
            .map_or((mesh.base_face + face as u32, 0), |incidence| {
                (incidence[point].start, incidence[point].len() as u32)
            });
        self.vertex_jobs.push([
            vertex,
            mesh.base_point + point as u32,
            mesh.center,
            first,
            count,
        ]);
    }

    pub fn add_edge(&mut self, mesh: &MeshJobs, edge: usize, a: i32, b: i32, face: usize) {
        let face = mesh.base_face + face as u32;
        self.edge_jobs.push([
            edge as u32,
            mesh.base_point + a as u32,
            mesh.base_point + b as u32,
            face,
            face,
        ]);
    }

    pub fn second_edge_face(&mut self, mesh: &MeshJobs, mesh_edge: usize, face: usize) {
        self.edge_jobs[mesh.base_edge + mesh_edge][4] = mesh.base_face + face as u32;
    }
}

/// Preserve smooth, area-weighted shading on adapters without usable compute.
/// The same fallback also seeds the buffers after genuine stage edits.
pub(super) fn normals(points: &[Vec3], indices: &[i32], flat: bool) -> Result<Vec<Vec3>> {
    let mut output = if flat {
        Vec::with_capacity(indices.len())
    } else {
        vec![DVec3::ZERO; points.len()]
    };
    for face in indices.chunks_exact(3) {
        let [a, b, c] = [face[0] as usize, face[1] as usize, face[2] as usize];
        let (Some(a_pos), Some(b_pos), Some(c_pos)) = (points.get(a), points.get(b), points.get(c))
        else {
            return Err(error("deforming mesh has an invalid face point index"));
        };
        let cross =
            (b_pos.as_dvec3() - a_pos.as_dvec3()).cross(c_pos.as_dvec3() - a_pos.as_dvec3());
        if flat {
            output.extend([cross; 3]);
        } else {
            for point in [a, b, c] {
                output[point] += cross;
            }
        }
    }
    Ok(output
        .into_iter()
        .map(|normal| {
            if normal.length_squared() > 0.0 {
                normal.normalize().as_vec3()
            } else {
                Vec3::Y
            }
        })
        .collect())
}

pub(super) struct GpuPose {
    meshes: Vec<Mesh>,
    static_bounds: Bounds,
    scratch: Vec<[f32; 4]>,
    points: wgpu::Buffer,
    pipeline: wgpu::ComputePipeline,
    binding: wgpu::BindGroup,
    groups: u32,
}

impl GpuPose {
    pub fn new(
        gpu: &GpuContext,
        plan: Plan,
        vertices: &wgpu::Buffer,
        edges: &wgpu::Buffer,
    ) -> Option<Self> {
        let jobs = plan.vertex_jobs.len() + plan.edge_jobs.len();
        let groups = u32::try_from(jobs.div_ceil(WORKGROUP_SIZE as usize)).ok()?;
        let limits = gpu.device.limits();
        let face_base = 4 + jobs * JOB_WORDS;
        let incidence_base = face_base + plan.faces.len() * 3;
        let work_bytes = (incidence_base + plan.incidence.len()) as u64 * size_of::<u32>() as u64;
        let point_bytes = plan.point_count as u64 * size_of::<[f32; 4]>() as u64;
        let binding_limit = limits.max_storage_buffer_binding_size as u64;
        if groups == 0
            || plan.unsupported
            || groups > limits.max_compute_workgroups_per_dimension
            || WORKGROUP_SIZE > limits.max_compute_workgroup_size_x
            || WORKGROUP_SIZE > limits.max_compute_invocations_per_workgroup
            || limits.max_storage_buffers_per_shader_stage < 4
            || !gpu
                .adapter
                .get_downlevel_capabilities()
                .flags
                .contains(wgpu::DownlevelFlags::COMPUTE_SHADERS)
            || [work_bytes, point_bytes, vertices.size(), edges.size()]
                .into_iter()
                .any(|size| size > binding_limit)
        {
            return None;
        }
        let mut work = Vec::with_capacity((work_bytes / size_of::<u32>() as u64) as usize);
        work.extend([
            plan.vertex_jobs.len() as u32,
            plan.edge_jobs.len() as u32,
            face_base as u32,
            incidence_base as u32,
        ]);
        work.extend(plan.vertex_jobs.into_iter().flatten());
        work.extend(plan.edge_jobs.into_iter().flatten());
        work.extend(plan.faces.into_iter().flatten());
        work.extend(plan.incidence);
        let work = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("USD deformation topology"),
                contents: bytemuck::cast_slice(&work),
                usage: wgpu::BufferUsages::STORAGE,
            });
        let points = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("USD deformation points"),
            size: point_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let shader = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("USD deformation"),
                source: wgpu::ShaderSource::Wgsl(include_str!("deform.wgsl").into()),
            });
        let pipeline = gpu
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("USD deformation"),
                layout: None,
                module: &shader,
                entry_point: Some("deform"),
                compilation_options: Default::default(),
                cache: None,
            });
        let binding = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("USD deformation"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[&points, &work, vertices, edges]
                .into_iter()
                .enumerate()
                .map(|(binding, buffer)| wgpu::BindGroupEntry {
                    binding: binding as u32,
                    resource: buffer.as_entire_binding(),
                })
                .collect::<Vec<_>>(),
        });
        Some(Self {
            meshes: plan.meshes,
            static_bounds: plan.static_bounds,
            scratch: Vec::with_capacity(plan.point_count),
            points,
            pipeline,
            binding,
            groups,
        })
    }

    pub fn update(
        &mut self,
        gpu: &GpuContext,
        encoder: &mut wgpu::CommandEncoder,
        stage: &Stage,
        time: Option<scene::TimeCode>,
    ) -> Result<Bounds> {
        self.scratch.clear();
        let mut bounds = self.static_bounds;
        for mesh in &self.meshes {
            let usd_mesh = geom::Mesh::get(stage, &mesh.path)
                .map_err(error)?
                .ok_or_else(|| error(format!("{}: missing deforming mesh", mesh.path)))?;
            let points = usd_mesh
                .points_attr()
                .get_at::<Vec<gf::Vec3f>>(time)
                .map_err(error)?
                .ok_or_else(|| error(format!("{}: missing deforming mesh points", mesh.path)))?;
            if points.len() != mesh.count {
                return Err(error(format!(
                    "{}: a pose must preserve the point count",
                    mesh.path
                )));
            }
            let mut local = Bounds::default();
            for p in points {
                let point = Vec3::new(p.x, p.y, p.z);
                if !point.is_finite() {
                    return Err(error(format!("{}: non-finite pose point", mesh.path)));
                }
                self.scratch.push(point.extend(1.0).to_array());
                local.include(point, false);
            }
            let [min, max] = local.scene;
            self.scratch
                .push((min * 0.5 + max * 0.5).extend(0.0).to_array());
            bounds.include(min, mesh.casts_shadow);
            bounds.include(max, mesh.casts_shadow);
        }
        gpu.queue
            .write_buffer(&self.points, 0, bytemuck::cast_slice(&self.scratch));
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("USD mesh pose and normals"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.binding, &[]);
        pass.dispatch_workgroups(self.groups, 1, 1);
        Ok(bounds)
    }
}
