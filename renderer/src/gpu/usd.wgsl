struct Camera { view_projection: mat4x4<f32>, view: mat4x4<f32>, viewport: vec4<f32> }
@group(0) @binding(0) var<uniform> camera: Camera;

// Per-model wireframe state, indexed by the model id stored on each edge.
// `mode` is 0 for off, 1 for every edge, 2 for silhouette/boundary edges and
// 3 for edges drawn through the surface.
struct ModelWire {
    core: vec4<f32>,
    outline: vec4<f32>,
    width_px: f32,
    mode: u32,
    _pad0: u32,
    _pad1: u32,
}
@group(1) @binding(0) var<storage, read> models: array<ModelWire>;

// One unique polygon-cage edge. `adjacency.x` counts the adjacent faces that
// contributed a normal; fewer than two means an open boundary edge.
// `adjacency.y` is the model that owns the edge.
struct Edge {
    a: vec4<f32>,
    b: vec4<f32>,
    n0: vec4<f32>,
    n1: vec4<f32>,
    adjacency: vec4<u32>,
}
@group(1) @binding(1) var<storage, read> edges: array<Edge>;

struct Varying {
    @builtin(position) position: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) color: vec3<f32>,
}

// Native UNORM targets are composited in encoded space. Resolving encoded
// color and coverage together produces premultiplied edge pixels.
fn target_encode(color: vec3<f32>) -> vec3<f32> {
    if camera.viewport.w <= 0.5 {
        return color;
    }
    return select(12.92 * color, 1.055 * pow(max(color, vec3(0.0)), vec3(1.0 / 2.4)) - 0.055, color > vec3(0.0031308));
}

// Move a clip-space position onto the horizontal strip selected by the
// instance index. The mesh and wireframe passes share this mapping.
fn to_strip(position: vec4<f32>, strip: u32) -> vec4<f32> {
    let full_height = camera.viewport.y;
    let strip_height = camera.viewport.z;
    let origin = f32(strip) * strip_height;
    var out = position;
    out.y = out.y * full_height / strip_height
        + out.w * (1.0 - full_height / strip_height + 2.0 * origin / strip_height);
    return out;
}

@vertex fn vertex(@location(0) position: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) color: vec3<f32>, @builtin(instance_index) strip: u32) -> Varying {
    var out: Varying;
    out.position = to_strip(camera.view_projection * vec4(position, 1.0), strip);
    out.normal = (camera.view * vec4(normal, 0.0)).xyz;
    out.color = color;
    return out;
}

@fragment fn fragment(in: Varying, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let n = normalize(select(-in.normal, in.normal, front));
    let studio = 0.3 + 0.7 * max(dot(n, normalize(vec3(-0.4, 0.7, 0.8))), 0.0);
    let rim = pow(1.0 - abs(n.z), 3.0) * 0.2;
    let color = in.color * studio + vec3(rim);
    return vec4(target_encode(color), 1.0);
}

struct WireVarying {
    @builtin(position) position: vec4<f32>,
    @location(0) dist: f32,
    @location(1) @interpolate(flat) core: vec3<f32>,
    @location(2) @interpolate(flat) outline: vec3<f32>,
}

fn wire_hidden() -> WireVarying {
    var out: WireVarying;
    out.position = vec4<f32>(0.0, 0.0, -2.0, 1.0);
    out.dist = 1.0;
    out.core = vec3<f32>(0.0);
    out.outline = vec3<f32>(0.0);
    return out;
}

// Two triangles per edge. The quad is centred on the edge and expanded along
// the screen-space perpendicular, so half the band falls on each adjacent face
// and, at a silhouette, half falls outside the mesh.
//
// `overlay_pass` selects which models this draw emits: the occluded pipeline
// takes modes 1 and 2, the through-surface pipeline takes mode 3. Emitting the
// same edge range twice keeps one edge buffer valid for both pipelines.
fn wire_emit(index: u32, strip: u32, overlay_pass: bool) -> WireVarying {
    let edge = edges[index / 6u];
    let model = models[min(edge.adjacency.y, arrayLength(&models) - 1u)];
    let wanted = model.mode;
    if overlay_pass {
        if wanted != 3u {
            return wire_hidden();
        }
    } else if wanted != 1u && wanted != 2u {
        return wire_hidden();
    }

    let clip_a = camera.view_projection * vec4(edge.a.xyz, 1.0);
    let clip_b = camera.view_projection * vec4(edge.b.xyz, 1.0);

    if wanted == 2u {
        // Camera position is the negated translation of the inverse view.
        let view_rotation = mat3x3(camera.view[0].xyz, camera.view[1].xyz, camera.view[2].xyz);
        let camera_position = -(transpose(view_rotation) * camera.view[3].xyz);
        let to_camera = camera_position - (edge.a.xyz + edge.b.xyz) * 0.5;
        // Authored normals are unit length or zero, so a degenerate face reads
        // as back-facing and keeps its edges visible instead of producing NaN.
        let front_a = dot(edge.n0.xyz, to_camera) > 0.0;
        let front_b = dot(edge.n1.xyz, to_camera) > 0.0;
        if edge.adjacency.x >= 2u && front_a == front_b {
            return wire_hidden();
        }
    }

    let corner = index % 6u;
    let ends = array<u32, 6>(0u, 0u, 1u, 0u, 1u, 1u);
    let sides = array<f32, 6>(-1.0, 1.0, 1.0, -1.0, 1.0, -1.0);
    let end = ends[corner];
    let side = sides[corner];

    // Screen-space perpendicular of the edge, in aspect-corrected NDC so the
    // band is an even width in pixels at any edge angle.
    let aspect = vec2(camera.viewport.x / camera.viewport.y, 1.0);
    let ndc_a = clip_a.xy / max(clip_a.w, 1e-6);
    let ndc_b = clip_b.xy / max(clip_b.w, 1e-6);
    let delta = (ndc_b - ndc_a) * aspect;
    let length_sq = dot(delta, delta);
    let dir = select(vec2(1.0, 0.0), delta / sqrt(max(length_sq, 1e-12)), length_sq > 1e-12);
    let perpendicular = vec2(-dir.y, dir.x) / aspect;

    var clip = select(clip_a, clip_b, end == 1u);
    // width_px is the full band width, so `side` of +/-1 spans the half width.
    let offset = perpendicular * (vec2(model.width_px) / camera.viewport.xy) * clip.w * side;
    // Line and point primitives receive no fixed-function depth bias, so nudge
    // the shared edge toward the camera to break coplanar ties.
    clip = vec4(clip.x + offset.x, clip.y + offset.y, clip.z - clip.w * 0.00001, clip.w);

    var out: WireVarying;
    out.position = to_strip(clip, strip);
    out.dist = side;
    out.core = model.core.rgb;
    out.outline = model.outline.rgb;
    return out;
}

@vertex fn wire_vertex_occluded(@builtin(vertex_index) index: u32, @builtin(instance_index) strip: u32) -> WireVarying {
    return wire_emit(index, strip, false);
}

@vertex fn wire_vertex_overlay(@builtin(vertex_index) index: u32, @builtin(instance_index) strip: u32) -> WireVarying {
    return wire_emit(index, strip, true);
}

@fragment fn wire_fragment(in: WireVarying) -> @location(0) vec4<f32> {
    // `dist` is -1..1 across the band. Equal core and outline colours give a
    // solid band; different colours give an outline on both sides of the core.
    let edge = smoothstep(0.48, 0.52, abs(in.dist));
    let color = mix(in.core, in.outline, edge);
    return vec4(target_encode(color), 1.0);
}
