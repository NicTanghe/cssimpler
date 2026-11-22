struct Camera {
    view_projection: mat4x4<f32>,
    view: mat4x4<f32>,
    viewport: vec4<f32>,
    projection_params: vec4<f32>,
}
@group(0) @binding(0) var<uniform> camera: Camera;

struct Studio {
    light_projection: mat4x4<f32>,
    key: vec4<f32>,
    up: vec4<f32>,
    lighting: vec4<f32>,
    occlusion_plane: vec4<f32>,
    shadows: vec4<f32>,
    key_tint: vec4<f32>,
    fill: vec4<f32>,
    fill_tint: vec4<f32>,
}
@group(0) @binding(1) var<uniform> studio: Studio;

struct Material {
    color: vec4<f32>,
    features: vec4<u32>,
    response: vec4<f32>,
    uv_scale: vec4<f32>,
}
@group(1) @binding(2) var<uniform> material: Material;
@group(1) @binding(3) var albedo: texture_2d<f32>;
@group(1) @binding(4) var albedo_sampler: sampler;

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
    @location(2) world_position: vec3<f32>,
    @location(3) uv: vec3<f32>,
    @location(4) local: vec3<f32>,
    @location(5) world_normal: vec3<f32>,
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

@vertex fn vertex(
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
    @location(3) uv: vec3<f32>,
    @location(4) local: vec3<f32>,
    @builtin(instance_index) strip: u32,
) -> Varying {
    var out: Varying;
    out.position = to_strip(camera.view_projection * vec4(position, 1.0), strip);
    out.normal = (camera.view * vec4(normal, 0.0)).xyz;
    out.world_normal = normal;
    out.color = color;
    out.world_position = position;
    out.uv = uv;
    out.local = local;
    return out;
}

@vertex fn shadow_vertex(@location(0) position: vec3<f32>) -> @builtin(position) vec4<f32> {
    return studio.light_projection * vec4(position, 1.0);
}

struct SurfaceUv {
    value: vec2<f32>,
    dx: vec2<f32>,
    dy: vec2<f32>,
    valid: bool,
}

fn surface_uv(in: Varying) -> SurfaceUv {
    let authored = in.uv.z > 0.5;
    let cylindrical = material.features.y == 1u || (material.features.y == 2u && !authored);
    // Keep derivatives continuous across the cylindrical seam. Vertical
    // coordinates use local mesh units, so motion does not slide the pattern.
    let radial = in.local.xz;
    let angle = atan2(radial.y, select(1.0, radial.x, dot(radial, radial) > 0.0));
    let angular_gradient = vec2(-radial.y, radial.x) / (6.2831853 * max(dot(radial, radial), 1e-12));
    let local_dx = dpdx(in.local);
    let local_dy = dpdy(in.local);
    let uv_dx = dpdx(in.uv.xy);
    let uv_dy = dpdy(in.uv.xy);
    let uv = select(in.uv.xy, vec2(angle / 6.2831853 + 0.5, in.local.y), cylindrical);
    let dx = select(uv_dx, vec2(dot(angular_gradient, local_dx.xz), local_dx.y), cylindrical);
    let dy = select(uv_dy, vec2(dot(angular_gradient, local_dy.xz), local_dy.y), cylindrical);
    return SurfaceUv(uv * material.uv_scale.xy, dx * material.uv_scale.xy, dy * material.uv_scale.xy, authored || cylindrical);
}

struct SurfaceFrame {
    tangent: vec3<f32>,
    bitangent: vec3<f32>,
}

fn unit_or_zero(v: vec3<f32>) -> vec3<f32> {
    return v * inverseSqrt(max(dot(v, v), 1e-24));
}

fn surface_frame(uv: SurfaceUv, position: vec3<f32>, normal: vec3<f32>) -> SurfaceFrame {
    let dx = dpdx(position);
    let dy = dpdy(position);
    let det = uv.dx.x * uv.dy.y - uv.dx.y * uv.dy.x;
    let t = (dx * uv.dy.y - dy * uv.dx.y) * sign(det);
    let b = (dy * uv.dx.x - dx * uv.dy.x) * sign(det);
    return SurfaceFrame(unit_or_zero(t - normal * dot(normal, t)),
                        unit_or_zero(b - normal * dot(normal, b)));
}

fn shade_fabric(base: vec3<f32>, normal: vec3<f32>, key: vec3<f32>, uv: SurfaceUv,
                frame: SurfaceFrame, facing: f32, key_power: vec3<f32>, ambient: vec3<f32>, roughness: f32) -> vec3<f32> {
    let frequency = material.response.z;
    let footprint = (abs(uv.dx) + abs(uv.dy)) * frequency;
    let detail = select(vec2(0.0), 1.0 - smoothstep(vec2(0.18), vec2(0.5), footprint), uv.valid);
    let phase = 6.2831853 * fract(uv.value * frequency);
    let thread = cos(phase) * detail;
    let weave = 0.04 * (thread.x + thread.y) + 0.04 * thread.x * thread.y;
    let slope = sin(phase) * detail;
    let woven_normal = normalize(normal + 0.13 * (frame.tangent * slope.x + frame.bitangent * slope.y));
    let direct = max(dot(woven_normal, key), 0.0) * key_power;
    let grazing = pow(1.0 - facing, mix(5.0, 2.0, roughness));
    let sheen = material.response.y * grazing * (0.35 * ambient + 0.35 * direct);
    return base * (ambient + direct) * (1.0 + weave) + mix(base, vec3(1.0), 0.5) * sheen;
}

fn shade_metal(base: vec3<f32>, normal: vec3<f32>, eye: vec3<f32>, up: vec3<f32>,
               key: vec3<f32>, uv: SurfaceUv, frame: SurfaceFrame, direct: vec3<f32>,
               ambient: vec3<f32>, roughness: f32) -> vec3<f32> {
    let reflected = reflect(-eye, normal);
    let height = 0.5 + 0.5 * dot(reflected, up);
    let brush_axis = dot(reflected - key, frame.tangent);
    let alignment = clamp(dot(reflected, key) - 0.25 * brush_axis * brush_axis, 0.0, 1.0);
    let softbox = pow(alignment, mix(18.0, 2.0, roughness));
    let footprint = (abs(uv.dx.x) + abs(uv.dy.x)) * 90.0;
    let grain = select(0.0, cos(6.2831853 * fract(uv.value.x * 90.0)) *
        (1.0 - smoothstep(0.15, 0.5, footprint)), uv.valid);
    let gradient = mix(0.60, 2.25, smoothstep(0.1, 0.9, height));
    return base * (ambient * gradient * (1.0 + 0.065 * grain) + 0.65 * softbox * direct);
}

@fragment fn fragment(in: Varying, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let studio_enabled = studio.key.w > 0.5;
    let view_normal = normalize(select(-in.normal, in.normal, front));
    let legacy_key = normalize(vec3(-0.4, 0.7, 0.8));
    let plain_legacy = !studio_enabled && material.features.x == 0u;
    let legacy = 0.3 + 0.7 * max(dot(view_normal, legacy_key), 0.0);
    let rim = pow(1.0 - abs(view_normal.z), 3.0) * 0.2;
    var base = select(in.color, material.color.rgb, material.color.w > 0.5);
    if plain_legacy && material.features.z == 0u {
        // Keep the original camera-relative look for unconfigured viewports.
        return vec4(target_encode(base * legacy + vec3(rim)), 1.0);
    }

    let rotation = mat3x3(camera.view[0].xyz, camera.view[1].xyz, camera.view[2].xyz);
    let normal = unit_or_zero(select(-in.world_normal, in.world_normal, front));
    let key = select(transpose(rotation) * legacy_key, studio.key.xyz, studio_enabled);
    let up = studio.up.xyz;
    let n_dot_l = max(dot(normal, key), 0.0);
    let ambient = select(0.3, studio.up.w, studio_enabled);
    let irradiance = select(0.7, studio.lighting.x, studio_enabled);
    let height = max(dot(in.world_position, up) - studio.occlusion_plane.x, 0.0);
    let ground = exp(-4.0 * height * studio.occlusion_plane.y);
    let sky = 0.5 + 0.5 * dot(normal, up);
    let ambient_occlusion = 1.0 - studio.lighting.y * ground * (1.0 - 0.75 * sky);
    let fill_power = max(dot(normal, studio.fill.xyz), 0.0) * studio.fill.w;
    let fill = (vec3(ambient) + studio.fill_tint.rgb * fill_power) * ambient_occlusion;
    let visibility = key_visibility(in.world_position, normal, n_dot_l);
    let key_power = studio.key_tint.rgb * (irradiance * mix(1.0, visibility, studio.fill_tint.w));
    let direct = n_dot_l * key_power;

    let uv = surface_uv(in);
    if material.features.z != 0u && uv.valid {
        let flip_v = vec2(1.0, -1.0);
        base *= textureSampleGrad(albedo, albedo_sampler, uv.value * flip_v, uv.dx * flip_v, uv.dy * flip_v).rgb;
    }

    let view_position = (camera.view * vec4(in.world_position, 1.0)).xyz;
    let view_ray = select(-view_position, vec3(0.0, 0.0, 1.0), camera.projection_params.x > 0.5);
    let eye = unit_or_zero(transpose(rotation) * view_ray);
    let roughness = material.response.x;
    var color = base * (fill + direct);
    if material.features.x != 0u {
        let frame = surface_frame(uv, in.world_position, normal);
        if material.features.x == 1u {
            color = shade_fabric(base, normal, key, uv, frame, clamp(abs(dot(normal, eye)), 0.0, 1.0),
                                 key_power, fill, roughness);
        } else {
            color = shade_metal(base, normal, eye, up, key, uv, frame, direct, fill, roughness);
        }
    } else if plain_legacy {
        color = base * legacy + vec3(rim);
    }

    color *= studio.lighting.z;
    let peak = max(color.r, max(color.g, color.b));
    color /= 1.0 + studio.lighting.w * max(peak - 0.65, 0.0);
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
