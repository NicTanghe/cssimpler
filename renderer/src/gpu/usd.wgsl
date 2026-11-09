struct Camera { view_projection: mat4x4<f32>, view: mat4x4<f32>, viewport: vec4<f32> }
@group(0) @binding(0) var<uniform> camera: Camera;
struct Varying {
    @builtin(position) position: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) color: vec3<f32>,
}
@vertex fn vertex(@location(0) position: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) color: vec3<f32>, @builtin(instance_index) strip: u32) -> Varying {
    var out: Varying;
    out.position = camera.view_projection * vec4(position, 1.0);
    let full_height = camera.viewport.y;
    let strip_height = camera.viewport.z;
    let origin = f32(strip) * strip_height;
    out.position.y = out.position.y * full_height / strip_height
        + out.position.w * (1.0 - full_height / strip_height + 2.0 * origin / strip_height);
    out.normal = (camera.view * vec4(normal, 0.0)).xyz;
    out.color = color;
    return out;
}
@fragment fn fragment(in: Varying, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let n = normalize(select(-in.normal, in.normal, front));
    let studio = 0.3 + 0.7 * max(dot(n, normalize(vec3(-0.4, 0.7, 0.8))), 0.0);
    let rim = pow(1.0 - abs(n.z), 3.0) * 0.2;
    var color = in.color * studio + vec3(rim);
    if camera.viewport.w > 0.5 {
        // Transparent UNORM targets are composited in encoded space by X11/CA. Resolving
        // encoded color and coverage together produces premultiplied edge pixels.
        color = select(12.92 * color, 1.055 * pow(max(color, vec3(0.0)), vec3(1.0 / 2.4)) - 0.055, color > vec3(0.0031308));
    }
    return vec4(color, 1.0);
}
