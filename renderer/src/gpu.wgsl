struct Viewport {
  size: vec2f,
  _padding: vec2f,
}

@group(0) @binding(0)
var<uniform> viewport: Viewport;

struct FillVertexOut {
  @builtin(position) position: vec4f,
  @location(0) local: vec2f,
  @location(1) size: vec2f,
  @location(2) radii: vec4f,
  @location(3) color: vec4f,
}

struct BorderVertexOut {
  @builtin(position) position: vec4f,
  @location(0) local: vec2f,
  @location(1) outer_size: vec2f,
  @location(2) inner_offset: vec2f,
  @location(3) inner_size: vec2f,
  @location(4) outer_radii: vec4f,
  @location(5) inner_radii: vec4f,
  @location(6) color: vec4f,
}

struct TextVertexOut {
  @builtin(position) position: vec4f,
  @location(0) uv: vec2f,
  @location(1) color: vec4f,
}

struct TextureVertexOut {
  @builtin(position) position: vec4f,
  @location(0) uv: vec2f,
}

struct ProjectedTextureVertexOut {
  @builtin(position) position: vec4f,
  @location(0) source_rect: vec4f,
  @location(1) inverse_row0: vec4f,
  @location(2) inverse_row1: vec4f,
  @location(3) inverse_row2: vec4f,
}

@group(1) @binding(0)
var text_texture: texture_2d<f32>;

@group(1) @binding(1)
var text_sampler: sampler;

fn quad_uv(index: u32) -> vec2f {
  switch index {
    case 0u: { return vec2f(0.0, 0.0); }
    case 1u: { return vec2f(1.0, 0.0); }
    case 2u: { return vec2f(0.0, 1.0); }
    case 3u: { return vec2f(0.0, 1.0); }
    case 4u: { return vec2f(1.0, 0.0); }
    default: { return vec2f(1.0, 1.0); }
  }
}

fn screen_to_clip(screen: vec2f) -> vec4f {
  let clip_x = (screen.x / viewport.size.x) * 2.0 - 1.0;
  let clip_y = 1.0 - (screen.y / viewport.size.y) * 2.0;
  return vec4f(clip_x, clip_y, 0.0, 1.0);
}

fn inside_rounded_rect(local: vec2f, size: vec2f, radii: vec4f) -> bool {
  if local.x < 0.0 || local.y < 0.0 || local.x > size.x || local.y > size.y {
    return false;
  }

  let top_left = radii.x;
  let top_right = radii.y;
  let bottom_right = radii.z;
  let bottom_left = radii.w;

  if top_left > 0.0 && local.x < top_left && local.y < top_left {
    let delta = local - vec2f(top_left, top_left);
    return dot(delta, delta) <= top_left * top_left;
  }
  if top_right > 0.0 && local.x > size.x - top_right && local.y < top_right {
    let delta = local - vec2f(size.x - top_right, top_right);
    return dot(delta, delta) <= top_right * top_right;
  }
  if bottom_right > 0.0 && local.x > size.x - bottom_right && local.y > size.y - bottom_right {
    let delta = local - vec2f(size.x - bottom_right, size.y - bottom_right);
    return dot(delta, delta) <= bottom_right * bottom_right;
  }
  if bottom_left > 0.0 && local.x < bottom_left && local.y > size.y - bottom_left {
    let delta = local - vec2f(bottom_left, size.y - bottom_left);
    return dot(delta, delta) <= bottom_left * bottom_left;
  }

  return true;
}

@vertex
fn fill_vs(
  @builtin(vertex_index) vertex_index: u32,
  @location(0) rect: vec4f,
  @location(1) radii: vec4f,
  @location(2) color: vec4f,
) -> FillVertexOut {
  let uv = quad_uv(vertex_index);
  let screen = rect.xy + uv * rect.zw;

  var out: FillVertexOut;
  out.position = screen_to_clip(screen);
  out.local = uv * rect.zw;
  out.size = rect.zw;
  out.radii = radii;
  out.color = color;
  return out;
}

@fragment
fn fill_fs(in: FillVertexOut) -> @location(0) vec4f {
  if !inside_rounded_rect(in.local, in.size, in.radii) {
    discard;
  }
  return in.color;
}

@vertex
fn border_vs(
  @builtin(vertex_index) vertex_index: u32,
  @location(0) outer_rect: vec4f,
  @location(1) inner_rect: vec4f,
  @location(2) outer_radii: vec4f,
  @location(3) inner_radii: vec4f,
  @location(4) color: vec4f,
) -> BorderVertexOut {
  let uv = quad_uv(vertex_index);
  let screen = outer_rect.xy + uv * outer_rect.zw;

  var out: BorderVertexOut;
  out.position = screen_to_clip(screen);
  out.local = uv * outer_rect.zw;
  out.outer_size = outer_rect.zw;
  out.inner_offset = inner_rect.xy - outer_rect.xy;
  out.inner_size = inner_rect.zw;
  out.outer_radii = outer_radii;
  out.inner_radii = inner_radii;
  out.color = color;
  return out;
}

@fragment
fn border_fs(in: BorderVertexOut) -> @location(0) vec4f {
  if !inside_rounded_rect(in.local, in.outer_size, in.outer_radii) {
    discard;
  }

  if in.inner_size.x > 0.0 && in.inner_size.y > 0.0 {
    let inner_local = in.local - in.inner_offset;
    if inside_rounded_rect(inner_local, in.inner_size, in.inner_radii) {
      discard;
    }
  }

  return in.color;
}

@vertex
fn text_vs(
  @builtin(vertex_index) vertex_index: u32,
  @location(0) rect: vec4f,
  @location(1) color: vec4f,
) -> TextVertexOut {
  let uv = quad_uv(vertex_index);
  let screen = rect.xy + uv * rect.zw;

  var out: TextVertexOut;
  out.position = screen_to_clip(screen);
  out.uv = uv;
  out.color = color;
  return out;
}

@fragment
fn text_fs(in: TextVertexOut) -> @location(0) vec4f {
  let alpha = textureSample(text_texture, text_sampler, in.uv).r;
  if alpha <= 0.0 {
    discard;
  }
  return vec4f(in.color.rgb, in.color.a * alpha);
}

@vertex
fn texture_vs(
  @builtin(vertex_index) vertex_index: u32,
  @location(0) rect: vec4f,
) -> TextureVertexOut {
  let uv = quad_uv(vertex_index);
  let screen = rect.xy + uv * rect.zw;

  var out: TextureVertexOut;
  out.position = screen_to_clip(screen);
  out.uv = uv;
  return out;
}

@fragment
fn texture_fs(in: TextureVertexOut) -> @location(0) vec4f {
  return textureSample(text_texture, text_sampler, in.uv);
}

@vertex
fn projected_texture_vs(
  @builtin(vertex_index) vertex_index: u32,
  @location(0) screen_rect: vec4f,
  @location(1) source_rect: vec4f,
  @location(2) inverse_row0: vec4f,
  @location(3) inverse_row1: vec4f,
  @location(4) inverse_row2: vec4f,
) -> ProjectedTextureVertexOut {
  let uv = quad_uv(vertex_index);
  let screen = screen_rect.xy + uv * screen_rect.zw;

  var out: ProjectedTextureVertexOut;
  out.position = screen_to_clip(screen);
  out.source_rect = source_rect;
  out.inverse_row0 = inverse_row0;
  out.inverse_row1 = inverse_row1;
  out.inverse_row2 = inverse_row2;
  return out;
}

@fragment
fn projected_texture_fs(in: ProjectedTextureVertexOut) -> @location(0) vec4f {
  let screen = in.position.xy;
  let denominator = dot(in.inverse_row2.xyz, vec3f(screen, 1.0));
  if abs(denominator) <= 0.00001 {
    discard;
  }

  let source_x = dot(in.inverse_row0.xyz, vec3f(screen, 1.0)) / denominator;
  let source_y = dot(in.inverse_row1.xyz, vec3f(screen, 1.0)) / denominator;
  let uv = vec2f(
    (source_x - in.source_rect.x) / in.source_rect.z,
    (source_y - in.source_rect.y) / in.source_rect.w,
  );
  if uv.x < 0.0 || uv.y < 0.0 || uv.x > 1.0 || uv.y > 1.0 {
    discard;
  }

  return textureSample(text_texture, text_sampler, uv);
}
