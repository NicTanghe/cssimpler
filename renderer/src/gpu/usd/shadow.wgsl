@group(2) @binding(0) var shadow_map: texture_depth_2d;
@group(2) @binding(1) var shadow_sampler: sampler_comparison;

fn key_visibility(position: vec3<f32>, n_dot_l: f32) -> f32 {
    if studio.shadows.x < 0.5 || n_dot_l <= 0.0 {
        return 1.0;
    }
    let light = studio.light_projection * vec4(position, 1.0);
    let uv = light.xy * vec2(0.5, -0.5) + vec2(0.5);
    if any(uv < vec2(0.0)) || any(uv >= vec2(1.0)) || light.z < 0.0 {
        return 1.0;
    }
    let size = vec2<i32>(textureDimensions(shadow_map));
    let center = vec2<i32>(uv * vec2<f32>(size));
    let bias = studio.shadows.y + studio.shadows.z * (1.0 - n_dot_l);
    // Keep receivers beyond the caster projection eligible for shadows, while
    // treating untouched depth texels (cleared to 1.0) as lit.
    let reference = clamp(light.z - bias, 0.0, 1.0);
    let radius = i32(studio.shadows.w);
    var lit = 0.0;
    for (var y = -radius; y <= radius; y++) {
        for (var x = -radius; x <= radius; x++) {
            let texel = center + vec2(x, y);
            if any(texel < vec2(0)) || any(texel >= size) {
                lit += 1.0;
            } else {
                let sample_uv = (vec2<f32>(texel) + vec2(0.5)) / vec2<f32>(size);
                lit += textureSampleCompareLevel(shadow_map, shadow_sampler, sample_uv, reference);
            }
        }
    }
    let width = 2 * radius + 1;
    return lit / f32(width * width);
}
