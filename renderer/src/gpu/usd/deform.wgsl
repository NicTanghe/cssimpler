struct Edge {
    a: vec4<f32>,
    b: vec4<f32>,
    n0: vec4<f32>,
    n1: vec4<f32>,
    adjacency: vec4<u32>,
}

@group(0) @binding(0) var<storage, read> points: array<vec4<f32>>;
// Header: vertex job count, edge job count, triangle offset, incidence offset.
// Jobs have five words, followed by triangle point indices and face incidence.
@group(0) @binding(1) var<storage, read> topology: array<u32>;
@group(0) @binding(2) var<storage, read_write> vertices: array<f32>;
@group(0) @binding(3) var<storage, read_write> edges: array<Edge>;

fn face_normal(face: u32) -> vec3<f32> {
    let triangle = topology[2] + 3u * face;
    let a = points[topology[triangle]].xyz;
    let b = points[topology[triangle + 1u]].xyz;
    let c = points[topology[triangle + 2u]].xyz;
    return cross(b - a, c - a);
}

fn unit_normal(n: vec3<f32>, fallback: vec3<f32>) -> vec3<f32> {
    let magnitude = max(abs(n.x), max(abs(n.y), abs(n.z)));
    if magnitude > 0.0 && magnitude <= 3.402823e38 {
        return normalize(n / magnitude);
    }
    return fallback;
}

fn write_vec3(offset: u32, value: vec3<f32>) {
    vertices[offset] = value.x;
    vertices[offset + 1u] = value.y;
    vertices[offset + 2u] = value.z;
}

@compute @workgroup_size(64)
fn deform(@builtin(global_invocation_id) id: vec3<u32>) {
    let job = id.x;
    let vertex_count = topology[0];
    if job >= vertex_count + topology[1] {
        return;
    }
    let words = 4u + 5u * job;
    let destination = topology[words];
    let a = topology[words + 1u];
    let b = topology[words + 2u];
    let first = topology[words + 3u];
    let last = topology[words + 4u];
    if job < vertex_count {
        // Vertex job: destination, point, center, face or incidence start, count.
        var normal = vec3(0.0);
        if last == 0u {
            normal = face_normal(first);
        } else {
            for (var i = 0u; i < last; i++) {
                normal += face_normal(topology[topology[3] + first + i]);
            }
        }
        // Vertex uses five packed float3 attributes: position, normal, color, UV, local.
        let offset = destination * 15u;
        let position = points[a].xyz;
        write_vec3(offset, position);
        write_vec3(offset + 3u, unit_normal(normal, vec3(0.0, 1.0, 0.0)));
        write_vec3(offset + 12u, position - points[b].xyz);
    } else {
        // Edge job: destination, endpoints, first and second adjacent faces.
        edges[destination].a = points[a];
        edges[destination].b = points[b];
        edges[destination].n0 = vec4(unit_normal(face_normal(first), vec3(0.0)), 0.0);
        edges[destination].n1 = vec4(unit_normal(face_normal(last), vec3(0.0)), 0.0);
    }
}
