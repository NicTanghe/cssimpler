@group(2) @binding(0) var shadow_map: texture_depth_2d;
@group(2) @binding(1) var shadow_sampler: sampler_comparison;

fn key_visibility(position: vec3<f32>, n_dot_l: f32) -> f32 {
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

    if studio.shadows.x < 0.5 || n_dot_l <= 0.0 ||
        any(uv < vec2(0.0)) || any(uv >= vec2(1.0)) || light.z < 0.0 {
        return 1.0;
    }
    let texel = 1.0 / vec2<f32>(textureDimensions(shadow_map));
    let radius = studio.shadows.w;
    // Linear depth comparisons smooth the sub-texel transition. Cap the
    // number of taps independently of blur width to bound rendering cost.
    let taps = min(i32(radius), 2);
    let step = texel * radius / f32(max(taps, 1));
    let subtexel_bias = min(0.003, 0.5 * dot(abs(depth_gradient), texel));
    let bias = studio.shadows.y + studio.shadows.z * (1.0 - n_dot_l) + subtexel_bias;
    var lit = 0.0;
    for (var y = -taps; y <= taps; y++) {
        for (var x = -taps; x <= taps; x++) {
            let delta = vec2(f32(x), f32(y)) * step;
            let sample_uv = uv + delta;
            if any(sample_uv < vec2(0.0)) || any(sample_uv >= vec2(1.0)) {
                lit += 1.0;
            } else {
                let slope_offset = clamp(dot(depth_gradient, delta), -0.02, 0.02);
                // Receivers beyond the caster bounds still see the map, while
                // untouched texels (cleared to 1.0) remain lit.
                let reference = clamp(light.z + slope_offset - bias, 0.0, 1.0);
                lit += textureSampleCompareLevel(shadow_map, shadow_sampler, sample_uv, reference);
            }
        }
    }
    let width = 2 * taps + 1;
    return lit / f32(width * width);
}
