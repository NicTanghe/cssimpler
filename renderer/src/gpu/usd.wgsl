struct Camera { view_projection: mat4x4<f32>, view: mat4x4<f32> }
@group(0) @binding(0) var<uniform> camera: Camera;
struct Varying {
    @builtin(position) position: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) color: vec3<f32>,
}
@vertex fn vertex(@location(0) position: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) color: vec3<f32>) -> Varying {
    var out: Varying;
    out.position = camera.view_projection * vec4(position, 1.0);
    out.normal = (camera.view * vec4(normal, 0.0)).xyz;
    out.color = color;
    return out;
}
@fragment fn fragment(in: Varying, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let n = normalize(select(-in.normal, in.normal, front));
    let studio = 0.3 + 0.7 * max(dot(n, normalize(vec3(-0.4, 0.7, 0.8))), 0.0);
    let rim = pow(1.0 - abs(n.z), 3.0) * 0.2;
    return vec4(in.color * studio + vec3(rim), 1.0);
}
