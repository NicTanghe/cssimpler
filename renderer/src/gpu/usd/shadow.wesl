import package::studio_uniform::studio;

@group(2) @binding(0) var shadow_map: texture_depth_2d;
@group(2) @binding(1) var shadow_sampler: sampler_comparison;

public fn key_visibility(position: vec3<f32>, normal: vec3<f32>, n_dot_l: f32) -> f32 {
    let light = studio.light_projection * vec4(position, 1.0);
    let uv = light.xy * vec2(0.5, -0.5) + vec2(0.5);

    // Compare each tap at the depth of the receiver plane at that tap.
    // A single depth across the kernel leaves self-shadow bands on slopes.
    let dx = dpdx(vec3(uv, light.z));
    let dy = dpdy(vec3(uv, light.z));
    let area = dx.x * dy.y - dx.y * dy.x;
    var depth_gradient = vec2(0.0);
    if abs(area) > 1e-12 {
        depth_gradient = vec2(dx.z * dy.y - dx.y * dy.z, dx.x * dy.z - dx.z * dy.x) / area;
    }

    if studio.shadows.x < 0.5 || studio.fill_tint.w == 0.0 || n_dot_l <= 0.0 ||
        any(uv < vec2(0.0)) || any(uv >= vec2(1.0)) || light.z < 0.0 {
        return 1.0;
    }
    let texel = 1.0 / vec2<f32>(textureDimensions(shadow_map));
    let max_radius = studio.shadows.w;
    let subtexel_bias = min(0.003, 0.5 * dot(abs(depth_gradient), texel));
    let bias = studio.shadows.y + studio.shadows.z * (1.0 - n_dot_l) + subtexel_bias;
    let receiver = light.z - bias;
    var radius = max_radius;
    let area_key = studio.key_tint.w > 0.0 && max_radius > 0.0;
    var wide_gradient = depth_gradient;
    if area_key {
        // Screen derivatives describe a single triangle. Extrapolating that
        // plane across a wide filter makes penumbras change at mesh edges.
        // Use the interpolated normal for the broad footprint instead.
        wide_gradient = smooth_receiver_gradient(normal, n_dot_l);
        let separation = blocker_separation(uv, receiver, wide_gradient, texel * max_radius);
        // The depth projection spans 3 radii and the UV projection spans 2.1.
        // Project angular key size along that receiver-to-blocker distance.
        let projected = (3.0 / 2.1) * separation * studio.key_tint.w;
        radius = clamp(projected / max(texel.x, texel.y), min(1.25, max_radius), max_radius);
    }
    if radius <= 1.0 {
        return compare_shadow(uv, receiver);
    }

    // A disk breaks up the staircase produced by equally spaced square taps.
    // Keep the same sample pattern as the area-key radius changes.
    let count = select(16u, 48u, area_key || radius > 6.0);
    var lit = 0.0;
    for (var i = 0u; i < count; i++) {
        let delta = disk_samples[i + select(0u, 16u, count == 48u)] * (radius * texel);
        // Retain precise geometric receiver depth for taps near contact.
        let local_weight = 1.0 - smoothstep(1.0, 5.0, length(delta / texel));
        let gradient = mix(wide_gradient, depth_gradient, local_weight);
        let slope_offset = clamp(dot(gradient, delta), -0.02, 0.02);
        lit += compare_shadow(uv + delta, receiver + slope_offset);
    }
    return lit / f32(count);
}

fn smooth_receiver_gradient(normal: vec3<f32>, n_dot_l: f32) -> vec2<f32> {
    let projection = transpose(studio.light_projection);
    let x_axis = projection[0].xyz;
    let y_axis = projection[1].xyz;
    let z_axis = projection[2].xyz;
    // The key projection is orthographic. These rows are orthogonal axes;
    // dividing by their squared lengths transforms the plane covector.
    let n = vec3(
        dot(normal, x_axis) / dot(x_axis, x_axis),
        dot(normal, y_axis) / dot(y_axis, y_axis),
        dot(normal, z_axis) / dot(z_axis, z_axis),
    );
    // Taper the correction at grazing angles, where a plane is ill-conditioned.
    return vec2(2.0, -2.0) * n.xy / max(abs(n.z), length(n) * 0.05)
        * smoothstep(0.03, 0.20, n_dot_l);
}

fn compare_shadow(uv: vec2<f32>, depth: f32) -> f32 {
    if any(uv < vec2(0.0)) || any(uv >= vec2(1.0)) {
        return 1.0;
    }
    // Receivers beyond the far plane can be shadowed, while cleared texels are lit.
    return textureSampleCompareLevel(shadow_map, shadow_sampler, uv, clamp(depth, 0.0, 1.0));
}

fn blocker_separation(uv: vec2<f32>, receiver: f32, gradient: vec2<f32>, extent: vec2<f32>) -> f32 {
    // Search past the filter footprint so a wide penumbra can extend into
    // pixels whose centre and inner probes are still fully illuminated.
    let probes = array(
        vec2(0.0), vec2(0.55, 0.0), vec2(-0.55, 0.0), vec2(0.0, 0.55), vec2(0.0, -0.55),
        vec2(1.1, 0.0), vec2(-1.1, 0.0), vec2(0.0, 1.1), vec2(0.0, -1.1),
        vec2(0.78, 0.78), vec2(-0.78, 0.78), vec2(0.78, -0.78), vec2(-0.78, -0.78),
    );
    var sum = 0.0;
    var count = 0.0;
    for (var i = 0u; i < 13u; i++) {
        let delta = probes[i] * extent;
        let sample_uv = uv + delta;
        let surface_depth = receiver + clamp(dot(gradient, delta), -0.02, 0.02);
        let occluded = 1.0 - compare_shadow(sample_uv, surface_depth);
        let weight = smoothstep(0.5, 0.95, occluded);
        if weight > 0.0 {
            // Recover approximate blocker depth using comparison sampling,
            // which is portable to backends without depth texture loads.
            var near = clamp(surface_depth - 0.5, 0.0, 1.0);
            var far = clamp(surface_depth, 0.0, 1.0);
            for (var search = 0u; search < 7u; search++) {
                let middle = (near + far) * 0.5;
                if compare_shadow(sample_uv, middle) >= 0.5 {
                    near = middle;
                } else {
                    far = middle;
                }
            }
            sum += weight * max(0.0, surface_depth - (near + far) * 0.5);
            count += weight;
        }
    }
    return sum / max(1.0, count);
}

const disk_samples = array(
    vec2(0.17678, 0.00000), vec2(-0.22577, 0.20683), vec2(0.03456, -0.39377), vec2(0.28457, 0.37117),
    vec2(-0.52222, -0.09237), vec2(0.49470, -0.31468), vec2(-0.16547, 0.61553), vec2(-0.31556, -0.60759),
    vec2(0.68464, 0.25003), vec2(-0.71226, 0.29401), vec2(0.34335, -0.73373), vec2(0.25373, 0.80893),
    vec2(-0.76475, -0.44319), vec2(0.89713, -0.19723), vec2(-0.54751, 0.77877), vec2(-0.12649, -0.97609),
    vec2(0.10206, 0.00000), vec2(-0.13035, 0.11941), vec2(0.01995, -0.22734), vec2(0.16430, 0.21430),
    vec2(-0.30151, -0.05333), vec2(0.28561, -0.18168), vec2(-0.09553, 0.35537), vec2(-0.18219, -0.35079),
    vec2(0.39528, 0.14436), vec2(-0.41122, 0.16975), vec2(0.19824, -0.42362), vec2(0.14649, 0.46704),
    vec2(-0.44153, -0.25587), vec2(0.51796, -0.11387), vec2(-0.31610, 0.44962), vec2(-0.07303, -0.56355),
    vec2(0.44832, 0.37784), vec2(-0.60329, 0.02495), vec2(0.44006, -0.43791), vec2(-0.02944, 0.63670),
    vec2(-0.41871, -0.50176), vec2(0.66329, 0.08924), vec2(-0.56200, 0.39103), vec2(0.15357, -0.68264),
    vec2(0.35520, 0.61988), vec2(-0.69439, -0.22153), vec2(0.67451, -0.31164), vec2(-0.29221, 0.69823),
    vec2(-0.26079, -0.72508), vec2(0.69395, 0.36472), vec2(-0.77080, 0.20318), vec2(0.43813, -0.68139),
    vec2(0.13937, 0.81096), vec2(-0.66050, -0.51153), vec2(0.84490, -0.07000), vec2(-0.58400, 0.63129),
    vec2(0.00425, -0.87201), vec2(0.59387, 0.65465), vec2(-0.89177, -0.08265), vec2(0.72260, -0.54843),
    vec2(-0.16441, 0.90373), vec2(-0.49523, -0.78697), vec2(0.90750, 0.24871), vec2(-0.84696, 0.43464),
    vec2(0.33470, -0.90281), vec2(0.36709, 0.90175), vec2(-0.88942, -0.42151), vec2(0.95062, -0.29309),
);
