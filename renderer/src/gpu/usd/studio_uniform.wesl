struct Studio {
    light_projection: mat4x4<f32>,
    key: vec4<f32>,
    up: vec4<f32>,
    lighting: vec4<f32>,
    occlusion_plane: vec4<f32>,
    shadows: vec4<f32>,
    shadow_fit: vec4<f32>,
    key_tint: vec4<f32>,
    fill: vec4<f32>,
    fill_tint: vec4<f32>,
}
@group(0) @binding(1) public var<uniform> studio: Studio;
