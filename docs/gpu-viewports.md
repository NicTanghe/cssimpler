# Native GPU viewports (experimental)

Enable the optional `gpu-viewport` feature to embed a `wgpu` viewport in the
CPU-rendered UI. Default builds retain the existing CPU-only path and do not
include the GPU or USD dependencies.

Native backends use Windows DirectComposition visuals, macOS Metal sublayers,
Linux X11 child windows or Linux Wayland subsurfaces. On Linux the host follows
winit's backend for the parent window.
These are separate presentation surfaces above the parent UI, sharing one GPU device.
Transparent pixels reveal CSSimpler's background; the CPU UI is not uploaded to
the GPU. X11 transparency uses an ARGB visual and automatic X Composite
redirection; X Shape 1.1 keeps input on the parent window. Transparent X11 views
require X Composite 0.4 and an ARGB visual.

On Wayland each viewport is a desynchronized `wl_subsurface` of the parent
surface, created on winit's connection with a separate event queue. An empty
input region keeps input on the parent, and the compositor blends premultiplied
buffer alpha itself. Positions apply with the parent's next commit, which follows
the GPU sync in the same frame. `wp_viewporter` maps the physical swapchain to
logical units, so scaled outputs (including fractional scales) require it; every
mainstream compositor provides it. Hiding destroys the subsurface role, which
unmaps the view without touching wgpu's buffers. To force the X11 path through
XWayland instead, use:

```sh
env -u WAYLAND_DISPLAY -u WAYLAND_SOCKET cargo run --features gpu-viewport --example usd_glass_cube
```

## Try the USD preview

For just a cube over CSSimpler's native glass background, with transparent GPU
pixels everywhere around the cube:

```powershell
cargo run --features gpu-viewport --example usd_glass_cube
```

Hold the primary button and drag the cube to orbit; scroll while holding it to
zoom. Release stops both camera controls, including trackpad scroll momentum.
Drag the top 28 pixels to move the
borderless window. The example uses on-demand rendering by default. Add
`-- --smoke` for a 120-frame native composition check.

From the repository root:

```powershell
cargo run --features gpu-viewport --example usd_viewport
cargo run --features gpu-viewport --example usd_viewport -- --transparent
cargo run --features gpu-viewport --example usd_viewport -- path/to/scene.usda
cargo run --features gpu-viewport --example usd_viewport -- --transparent --two-views
cargo run --features gpu-viewport --example usd_viewport -- --camera=/World/Camera
```

Hold the primary mouse button to orbit or zoom with the wheel/trackpad. The
sidebar and text input are CPU-rendered. Both USD examples render on demand by
default, so an idle scene does not keep submitting GPU work. `usd_viewport`
accepts `--continuous` to render every frame. `--smoke` runs 120 GPU frames, prints GPU
and CPU frame diagnostics, and exits through the normal application lifecycle.

## Register a USD viewport

Reserve a childless element with an ID in `ui!`, then register its renderer:

```rust,ignore
use cssimpler::renderer::gpu::usd::UsdCameraMode;

// The view contains <div id="scene" class="viewport"></div>.
let app = app.with_usd_viewport("scene", "asset.usda", UsdCameraMode::Orbit);
```

Use `UsdCameraMode::Authored("/World/Camera".into())` for a camera in the file.
For transparent output or continuous redraw, use the generic registration with
the built-in USD factory:

```rust,ignore
use cssimpler::renderer::{RedrawSchedule, gpu::{GpuViewportOptions, usd}};

let app = app.with_gpu_viewport(
    "scene",
    GpuViewportOptions { transparent: true, redraw: RedrawSchedule::EveryFrame },
    usd::factory("asset.usda", usd::UsdCameraMode::Orbit),
);
```

Both registration methods are available on `App` and `FragmentApp`. A custom
`SceneProvider` can transfer its registrations through `take_gpu_viewports`.
`GpuViewportRegistry::register` returns a thread-safe redraw handle. Renderer
factories also receive that handle and the shared GPU context. On-demand redraw
is the default; GPU-only frames do not invoke the CPU scene preparation path.

## Wireframe

`UsdRenderer::set_wireframe` sets the default mode for every mesh, and
`set_model_wireframe` overrides it for one mesh prim:

| `WireframeMode` | Edges drawn | Occlusion |
| --- | --- | --- |
| `Off` | none | — |
| `Edges` | every polygon-cage edge | hidden behind the surface |
| `Silhouette` | silhouette and open-boundary edges | hidden behind the surface |
| `Overlay` | every polygon-cage edge | drawn through the surface |

Edges come from the authored polygon cage rather than the fan triangulation, so
quad meshes do not show a diagonal per face. Each edge is stroked as a
screen-space band centred on it: the band covers both adjacent faces, and at a
silhouette it extends onto the background, so it is visible on both sides of
the mesh. Line and point primitives ignore fixed-function depth bias, so the
pass nudges the stroke toward the camera in clip space instead.

`WireframeStyle` sets the band width in physical pixels and its colours. The
band is split in half: `core` fills the middle and `outline` fills the outer
quarter either side. Equal colours therefore give a solid band, and different
colours give an outline on both sides of a lighter core. The default is a solid
2px black band.

```rust,ignore
use cssimpler::renderer::gpu::usd::{
    UsdRenderer, Wireframe, WireframeMode, WireframeStyle,
};

// Viewport default: silhouette strokes, 4px, light core with black edges.
renderer.set_wireframe_default(Wireframe {
    mode: WireframeMode::Silhouette,
    style: WireframeStyle {
        core: [0.10, 0.10, 0.10],
        outline: [0.0, 0.0, 0.0],
        width_px: 4.0,
    },
});

// Override one mesh, addressed by its USD prim path. Unprefixed models keep
// the viewport default.
renderer.set_model_wireframe(
    "/World/Preview",
    Wireframe {
        mode: WireframeMode::Off,
        ..Default::default()
    },
);
```

Modes, widths and colours are resolved per edge from a model array indexed by
the owning mesh, so one draw call covers the whole cage and a mesh can be
stroked differently from its neighbours. `UsdRenderer::models` lists the mesh
prim paths in the order used as the model id. Changing any wireframe setting
wakes an on-demand viewport, so a click can drive it without a continuous
redraw schedule. Enabling wireframe does not re-upload mesh vertices: unique
edges are collected during mesh extraction and drawn from a storage buffer, and
the stroke pipeline binds no vertex buffer at all. The `usd_viewport` example
shows per-model controls plus viewport-wide defaults; `usd_glass_cube` shows the
same controls in a toolbar.

## USD and the initial rendering subset

The native Rust `openusd` stage is the authoritative representation of geometry,
cameras, materials, and lights. GPU vertex buffers and camera uniforms are derived
execution data. To show one stage in multiple widgets, open a `UsdScene` once and
clone its handle into their registrations. Stages stay on the event-loop thread.

The current renderer supports visible `UsdGeomMesh` polygon cages with convex
faces, world transforms, a constant `displayColor`, time-sampled points/topology,
perspective/orthographic authored cameras, and Y-up/Z-up orbit views. It uses
normals from the mesh's `normals` attribute when a value is available at the
selected evaluation time; otherwise it keeps the existing flat-face fallback.
Authored normals are transformed with the inverse transpose and normalized before
being uploaded as shading normals. It defaults to camera-relative shading using
USD display colors. The built-in backend also offers view-local studio lighting
and material overrides. Subdivision, skinning, arbitrary UsdShade networks,
UsdLux evaluation, and production PBR remain outside this backend's subset.
Authored material and lighting data stay in the USD stage.
Concave faces and malformed topology return errors instead of incorrect geometry.

### Studio lighting and mesh materials

Construct an `UsdRenderer` inside a `with_gpu_viewport` factory and configure it
before returning it. Assignments match exact USD mesh prim paths, including
meshes that appear later. They affect only this view's rendering. A collider
can continue to use its own geometry.

```rust,ignore
use cssimpler::renderer::gpu::usd::{
    UsdCameraMode, UsdMaterial, UsdRenderer, UsdShadowSettings, UsdStudioLighting,
};

let mut renderer = UsdRenderer::new(init, scene, UsdCameraMode::Orbit)?;
renderer.set_studio_lighting(Some(UsdStudioLighting {
    ambient: 0.30,
    occlusion: 0.25,
    exposure: 0.25,
    highlight_rolloff: 0.35,
    shadows: Some(UsdShadowSettings { filter_radius: 2, ..Default::default() }),
    ..Default::default()
}))?;
renderer.set_model_material("/Form/DressForm", UsdMaterial::fabric([0.55, 0.43, 0.29]))?;
renderer.set_model_material("/Form/AluminumCap", UsdMaterial::brushed_metal([0.66, 0.72, 0.78]))?;
renderer.set_model_material("/Form/Brass", UsdMaterial::brushed_metal([0.60, 0.42, 0.16]))?;
renderer.set_model_material("/Form/Stand", UsdMaterial::dark_stand())?;
Ok(Box::new(renderer))
```

- A directional key adds shadowing within and between opaque meshes. The depth
  pass fits the evaluated caster bounds, includes both sides of cloth, and
  reuses the map during orbit navigation and material changes. The optional
  neutral floor sits below the scene. Set `floor: false` to use an authored
  receiver. A mesh with `custom bool cssimpler:castsShadow = false` receives
  shadows while staying outside the caster pass and its fitted bounds.
- Bias parameters use normalized light depth. Increase `depth_bias` or
  `slope_bias` in small steps if a receiver shows self-shadow artifacts.
  `filter_radius` sets the disk filter radius in map texels. A nonzero
  `light_radius` enables contact hardening and uses `filter_radius` as the
  maximum softness. Resolution is capped at the GPU's 2D texture limit. If the
  device cannot sample a depth attachment, the studio lights and floor still
  render. `supports_shadow_mapping()` reports this capability and
  `shadows: None` explicitly disables the depth pass.
- Ambient occlusion here approximates ground-facing darkening near the base of
  the scene. Exposure is in stops and highlight rolloff compresses bright
  channels together to preserve their ratios. The directional key and
  unshadowed fill have independent linear RGB tints. Both directions use +Y
  as up in the API, mapped to the stage's up axis.
- Materials use linear RGB tints or the existing USD display color. Fabric has
  grazing sheen and a derivative-filtered weave with surface relief. Brushed
  metal has a directional grain and a soft studio reflection gradient. Dark
  stands use a low-reflectance plain tint.
- For a texture, assign a `UsdColorTexture::new([width, height], rgba8)?` to
  `UsdMaterial::color_texture`. The renderer expects decoded, top-row-first
  sRGB pixels and samples RGB with repeating bilinear filtering. Meshes remain
  opaque. Indexed and unindexed `primvars:st` are read at the selected time and
  support constant, uniform, vertex, varying, and face-varying interpolation.
  `UsdUvMapping::AuthoredOrCylindrical` falls back to a cylinder around the
  mesh's local up axis, with one wrap horizontally and local units vertically.
  `uv_scale` controls repeat counts. Use `Cylindrical` to force that mapping or
  `Authored` to skip texture and weave detail when usable UVs are absent.

`clear_model_material` restores a mesh's default surface, and
`set_studio_lighting(None)` restores camera-relative lighting and removes the
generated floor. These view controls keep the USD stage and prim identities
as the input boundary for a future viewport backend. They leave authored
UsdShade and UsdLux data available for backends that evaluate them.

Run `cargo run --features gpu-viewport --example usd_viewport -- --studio`
to view the fabric receiver, metal caster and floor in the two-mesh fixture.

### Authored mesh normals

Normal indexing follows USD interpolation metadata, including across polygon
fan triangulation:

| `interpolation` | Required values | Value used for each emitted corner |
| --- | --- | --- |
| `constant` | One | The same normal everywhere |
| `uniform` | One per face | The source face's normal |
| `vertex` | One per point | Indexed by the source point index |
| `varying` | One per point | Indexed by the source point index for this polygon-cage renderer |
| `faceVarying` | One per face corner | Indexed by the original flat face-corner offset |

When interpolation metadata is absent, normals use `vertex`. For example, this
attribute fragment supplies normals for a four-point mesh:

```usda
normal3f[] normals = [(0, 0, 1), (0, 0, 1), (0, 0, 1), (0, 0, 1)] (
    interpolation = "vertex"
)
```

Authored normals can differ from geometric face normals: smooth vertex normals
are retained, and face-varying normals preserve distinct normals at shared points.
The existing shader interpolates normals over triangles and renormalizes them for
studio lighting. Nonuniform scale, shear, and rotation use the world transform's
inverse transpose; translation does not affect their direction. Non-unit input
vectors are normalized, including very large or small finite magnitudes.

The renderer reads normals at the same selected time as points and topology.
Changing authored normals through the shared USD stage invalidates derived mesh
vertices through the existing stage revision tracking; changing evaluation time
also reevaluates normals. USD resolves the time samples before normalization.

A missing value at the selected time uses generated flat face normals. An authored
value with a wrong count, unsupported interpolation, zero/nonfinite direction, or
singular normal transform returns an error with the mesh path. It does not
silently replace malformed authored normals with flat shading. This support is
for `Mesh.normals`; indexed `primvars:normals` and normal-map textures are not
evaluated by this path.

### Cameras and viewport composition

Orbit cameras live in a viewport-owned in-memory USD stage and do not modify the
source asset. `UsdRenderer::control()` provides view-local animation-time control
when constructing a renderer through the generic factory API. Stage edits wake
the viewport and invalidate its derived geometry. The initial evaluation time is
the stage's start time.

Separate native surfaces currently require fully visible, rectangular, childless
regions. A viewport hides when partially clipped, covered by later-painted UI,
rounded, or affected by unsupported transforms. Its CPU background remains visible
while hidden. Full CSS stacking and background effects sampling live 3D are pending.

## macOS composition

macOS uses a viewport-sized `CAMetalLayer` attached to the window's current
content view. It sits above the CPU content and native-glass tint layers, without
adding a view that could intercept pointer or keyboard events. Native glass is
installed before GPU hosts are synchronized. The original winit content view
remains in place, with an input-transparent `NSVisualEffectView` underneath the
CPU/GPU layers. Unchanged layer geometry is not reapplied. Hidden or removed
viewports hide their layer; dropping the host
removes it after the GPU surface is destroyed.

Placement converts physical pixels to AppKit points using the current scale
factor, accounts for flipped layer coordinates, and updates the Metal drawable
size on resize. Layer changes disable implicit animations. The CPU presenter
and its backing layer remain in place.

Transparent Metal surfaces use encoded BGRA8 and premultiplied RGB/coverage for
Core Animation. wgpu's Metal `PostMultiplied` surface mode marks the layer
non-opaque; it does not convert pixels, so renderer callbacks receive
`GpuViewportInfo::alpha_mode = PreMultiplied` to describe the required encoding.

## Anti-aliasing, performance and memory

The USD renderer uses 4× MSAA on Windows, Linux and macOS when the RGBA8/BGRA8
color and depth formats support 4× sampling. On Apple GPUs where Metal reports that
transient attachments save memory, it renders once using memoryless MSAA color
and depth attachments and resolves directly into the presentation texture.
These attachments live in tile memory; no persistent full-frame MSAA buffers or
intermediate resolve image are allocated.

Windows renders once into reusable full-frame MSAA color and depth attachments,
then resolves directly into the presentation texture. This avoids repeating
geometry processing, render-pass setup and GPU copies for every strip, and does
not require surface `COPY_DST` support. It trades memory for speed: the two
attachments use 32 bytes per pixel (about 58.6 MiB at 1600×1200), compared with
4 bytes per pixel for the former single-sample depth attachment. There is no
intermediate resolve image or history texture.

Other macOS adapters and Linux use one reusable horizontal strip, resolve
coverage there, then copy the strip directly into the presentation texture on
the GPU. This fallback requires surface GPU-copy support. CPU UI pixels are
never uploaded for AA; production rendering does not read back GPU images.

For glass, the renderer clears to transparent black and resolves premultiplied
color and coverage for native composition. Transparent DirectComposition, X11
and Metal targets use an encoded UNORM format so the resolved RGB stays premultiplied in the compositor's
color space. Non-glass views clear to the solid background found beneath the
viewport in paint order, including its own background color and the window clear
color as fallback. This assumes a solid backdrop; gradients and images are not
sampled or copied into the GPU renderer.

The fallback strip uses 36 bytes per pixel: 4× RGBA8 color, 4× Depth32, and one resolved
RGBA8 image. Its height is capped at 64 rows and sized so these attachments plus
the extra uniform data fit within the previous full-window Depth32 allocation.
Strip-based views below 16 pixels high and unsupported surfaces retain
single-sample rendering. Strips repeat vertex processing, trading extra GPU
work for bounded memory. Background-only changes do not reallocate render targets.
Camera uniforms are evaluated and uploaded only when the camera stage, animation
time, physical dimensions or target projection changes. Unchanged Windows visual
placement skips redundant DirectComposition updates; swapchain configuration
changes are still committed explicitly.

Validation commands (the image comparison test requires a GPU):

```sh
cargo test -p cssimpler-renderer --features gpu-viewport --lib gpu::
cargo test -p cssimpler-renderer --features gpu-viewport --lib strip_resolve_matches -- --ignored
```

The GPU tests compare RGBA8/BGRA8 strip and native direct-resolve output with
full-frame 4× MSAA, check fractional edge coverage and premultiplied RGB, and
cover a partial final strip. The Metal strip fallback permits one color-only
shared-edge pixel difference with identical alpha; the memoryless path matches
the reference within two channel levels throughout. They also exercise camera
cache invalidation and target reuse. Readback and reference images are confined
to tests. On Windows these tests use DX12 to match native DirectComposition.

To compare single-sample rendering, strip AA, full-frame AA and (on supported
Apple GPUs) memoryless AA without vsync:

```sh
cargo test -p cssimpler-renderer --release --features gpu-viewport --lib usd_msaa_frame_cost -- --ignored --nocapture --test-threads=1
```

On Windows with a GTX 1660 Ti / DX12 (driver 27.21.14.5751), release-build
measurements of the bundled cube gave the following medians across five process
runs. Each run discards one warmup round and averages 120 measured frames per
strategy, reversing strategy order between rounds. Times are microseconds.

| Size | Strategy | CPU encode | Encode, submit and GPU completion |
| --- | --- | ---: | ---: |
| 800×600 | Single-sample, camera evaluated every frame | 99 | 234 |
| 800×600 | Strip 4× MSAA, cached camera | 610 | 913 |
| 800×600 | Direct 4× MSAA, cached camera | 64 | 183 |
| 1600×1200 | Single-sample, camera evaluated every frame | 100 | 259 |
| 1600×1200 | Strip 4× MSAA, cached camera | 1131 | 1732 |
| 1600×1200 | Direct 4× MSAA, cached camera | 64 | 275 |

At 1600×1200, direct AA completion times ranged from 269 to 282 µs across the
five runs, versus 1697 to 1851 µs for strips. For direct AA, disabling camera
caching raised median encoding time to 121 µs and total completion time to
370 µs. The single-sample case simulates the former per-frame camera work;
these are fixed-camera render-work comparisons, excluding native composition,
window dragging, vsync and input latency. Moving cameras still incur evaluation
and upload work, and AA adds GPU cost compared with single-sample rendering.

In an earlier Metal development-build run at 1600×1200, mean CPU encoding time decreased from
813 to 88 µs and encoding/submission/GPU completion time from 2568 to 460 µs.
These are render-work measurements; native glass, presentation and input latency
are outside that benchmark. Native Metal surfaces use two drawables to limit
queued camera frames. GPU-consumed orbit/zoom events do not schedule CPU UI
redraws. Drag capture retains its original viewport until all held buttons are
released, is cancelled when hidden,
and follows queued press/release events on every platform. Live OS button polling
is deliberately avoided: it can cancel valid queued drag movement after the
physical release, particularly when native glass adds presentation latency.

The macOS host also observes native AppKit drag/release events for its own
window. Native vibrancy can receive those events without winit delivering the
corresponding cursor motion or mouse-up. The host coalesces native movement for
the captured viewport and forwards its final held position before release.
Only the captured viewport is awakened, and native events remain available to
AppKit. This bridge uses real event edges rather than polling global buttons.

In windowed mode, macOS Ctrl+Command pointer gestures yield to system window
management before UI hit testing or GPU capture. Completing that chord during a
drag cancels capture and discards buffered native movement; releasing the
modifier does not resume the drag without a fresh click. Window moves and resizes
alone do not cancel pointer capture. A primary press with that chord starts
AppKit's window-drag loop when the user's `NSWindowShouldDragOnGesture`
preference is enabled, including for frameless windows. Keyboard shortcuts remain
available, and fullscreen keeps modifier pointer gestures in the app.
`set_system_drag_override` can explicitly yield pointer input in either mode.

For native input and frame-timing diagnostics:

```sh
cargo run --release --features gpu-viewport --example usd_glass_cube -- --trace-input
```

The trace distinguishes AppKit input, winit delivery, viewport capture, and
GPU acquisition/encoding/presentation timing. It is disabled in normal runs.

Exercise the glass example's on-demand redraw path (rather than continuous
smoke rendering) with:

```sh
cargo run --features gpu-viewport --example usd_glass_cube -- --smoke-on-demand
```

This requests each subsequent frame through the redraw handle and exits after
120 frames, reporting elapsed time. It checks redraw scheduling independently
of manual pointer input.

`--smoke-input` additionally feeds 1,000 pointer samples per frame while orbiting
and presenting native glass, then checks that hover and scrolling are rejected
after releasing the button. For an optimized native run:

```sh
cargo run --release --features gpu-viewport --example usd_glass_cube -- --smoke-input
```

Input samples accumulate camera changes; USD camera attributes are committed
once per rendered frame. Redraw wakeups coalesce while a frame is pending, and
GPU-consumed button events avoid an unnecessary CPU glass frame.
During `--smoke-input`, the cube deliberately rotates under scripted input and
ignores physical input callbacks so manual window interaction cannot interrupt
the test's simulated press/release sequence. Normal runs use physical input.

A release-build run on the development Mac completed this native glass stress
test in 1984 ms for 120 frames. The samples enter renderer callbacks directly;
this verifies input processing and presentation capacity, not physical trackpad
event delivery or input-to-display latency.

On the development X11/NVIDIA RTX 3070 system, paired release-build measurements
at matching 1912×2152 window sizes gave:

| Example | Process RAM before → after (KiB) | RAM growth | Reported VRAM before → after |
| --- | ---: | ---: | ---: |
| Glass cube | 209,184 → 216,760 | 3.6% | 203 → 203 MiB |
| Regular viewport | 212,244 → 222,152 | 4.7% | 203 → 203 MiB |

These are whole-process `VmHWM`/`VmRSS` and NVIDIA `pmon` samples after four
seconds, within the requested 10% growth limit for these workloads. Driver
allocation and scene complexity can affect other workloads. Native 120-frame
smoke tests also passed for the glass cube and two transparent viewports.

See the [specsheet](./native-gpu-viewport-specsheet.md) for the full target, deferred
platform work, and future upstream Hydra-like backend migration.
