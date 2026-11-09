# Native GPU viewports (experimental)

Enable the optional `gpu-viewport` feature to embed a `wgpu` viewport in the
CPU-rendered UI. Default builds retain the existing CPU-only path and do not
include the GPU or USD dependencies.

The first native backend uses Windows DirectComposition visuals. These are
separate presentation surfaces above the parent UI, sharing one GPU device.
Transparent pixels reveal CSSimpler's background; the CPU UI is not uploaded to
the GPU. macOS, X11, and Wayland embedding are still pending and currently return
an explicit unsupported-platform error when a visible viewport is registered.

## Try the USD preview

For just a cube over CSSimpler's native glass background, with transparent GPU
pixels everywhere around the cube:

```powershell
cargo run --features gpu-viewport --example usd_glass_cube
```

Drag the cube to orbit, scroll to zoom, and drag the top 28 pixels to move the
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

Drag with the primary mouse button to orbit and use the wheel to zoom. The
sidebar and text input are CPU-rendered. `--smoke` runs 120 GPU frames, prints GPU
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

## USD and the initial rendering subset

The native Rust `openusd` stage is the authoritative representation of geometry,
cameras, materials, and lights. GPU vertex buffers and camera uniforms are derived
execution data. To show one stage in multiple widgets, open a `UsdScene` once and
clone its handle into their registrations. Stages stay on the event-loop thread.

The current renderer supports visible `UsdGeomMesh` polygon cages with convex
faces, world transforms, a constant `displayColor`, time-sampled points/topology,
perspective/orthographic authored cameras, and Y-up/Z-up orbit views. It generates
flat normals and uses built-in studio shading. It does not yet evaluate subdivision,
skinning, arbitrary material networks, textures, authored lighting, or production
PBR. Material and lighting data remain in the USD stage for later rendering support.
Concave faces and malformed topology return errors instead of incorrect geometry.

Orbit cameras live in a viewport-owned in-memory USD stage and do not modify the
source asset. `UsdRenderer::control()` provides view-local animation-time control
when constructing a renderer through the generic factory API. Stage edits wake
the viewport and invalidate its derived geometry. The initial evaluation time is
the stage's start time.

Separate native surfaces currently require fully visible, rectangular, childless
regions. A viewport hides when partially clipped, covered by later-painted UI,
rounded, or affected by unsupported transforms. Its CPU background remains visible
while hidden. Full CSS stacking and background effects sampling live 3D are pending.

See the [specsheet](./native-gpu-viewport-specsheet.md) for the full target, deferred
platform work, and future upstream Hydra-like backend migration.
