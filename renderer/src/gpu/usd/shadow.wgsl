@group(2) @binding(0) var shadow_map: texture_depth_2d;

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
    let radius = i32(studio.shadows.w);
    var lit = 0.0;
    for (var y = -radius; y <= radius; y++) {
        for (var x = -radius; x <= radius; x++) {
            let texel = center + vec2(x, y);
            if any(texel < vec2(0)) || any(texel >= size) {
                lit += 1.0;
            } else {
                let depth = textureLoad(shadow_map, texel, 0);
                // The shadow projection encloses the casters. A floor point
                // behind its far plane can still be occluded by those casters.
                lit += select(0.0, 1.0, depth >= 1.0 || light.z - bias <= depth);
            }
        }
    }
    let width = 2 * radius + 1;
    return lit / f32(width * width);
}
