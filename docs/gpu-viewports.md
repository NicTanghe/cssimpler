# Native GPU viewports (experimental)

Enable the optional `gpu-viewport` feature to embed a `wgpu` viewport in the
CPU-rendered UI. Default builds retain the existing CPU-only path and do not
include the GPU or USD dependencies.

Native backends use Windows DirectComposition visuals or Linux X11 child windows.
These are separate presentation surfaces above the parent UI, sharing one GPU device.
Transparent pixels reveal CSSimpler's background; the CPU UI is not uploaded to
the GPU. X11 transparency uses an ARGB visual and automatic X Composite
redirection; X Shape 1.1 keeps input on the parent window. Transparent X11 views
require X Composite 0.4 and an ARGB visual. macOS and native Wayland embedding
are still pending and return an explicit unsupported-platform error when a
visible viewport is registered. On Wayland desktops with XWayland, use:

```sh
env -u WAYLAND_DISPLAY -u WAYLAND_SOCKET cargo run --features gpu-viewport --example usd_glass_cube
```

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

## Linux anti-aliasing and memory

The USD renderer uses 4× MSAA on Linux when the surface supports GPU copies and
the RGBA8 color/depth formats support 4× sampling. It renders into one reusable
horizontal strip, resolves coverage there, then copies the strip directly into
the presentation texture on the GPU. CPU UI pixels are never uploaded for AA;
production rendering does not read back GPU images.

For glass, the renderer clears to transparent black and resolves premultiplied
color and coverage for native X11 composition. Transparent X11 targets use an
encoded UNORM format so the resolved RGB stays premultiplied in the compositor's
color space. Non-glass views clear to the solid background found beneath the
viewport in paint order, including its own background color and the window clear
color as fallback. This assumes a solid backdrop; gradients and images are not
sampled or copied into the GPU renderer.

The strip uses 36 bytes per pixel: 4× RGBA8 color, 4× Depth32, and one resolved
RGBA8 image. Its height is capped at 64 rows and sized so these attachments plus
the extra uniform data fit within the previous full-window Depth32 allocation.
Views below 16 pixels high and unsupported surfaces retain single-sample rendering.
Strips repeat vertex processing, trading extra GPU work for bounded memory; no
full-window MSAA or history textures are allocated. Background-only changes do
not reallocate render targets.

Validation commands (the image comparison test requires a GPU):

```sh
cargo test -p cssimpler-renderer --features gpu-viewport --lib gpu::
cargo test -p cssimpler-renderer --features gpu-viewport --lib strip_resolve_matches -- --ignored
```

The GPU test compares strip output with full-frame 4× MSAA, checks fractional
edge coverage and premultiplied RGB, and covers a partial final strip. Readback
and full-frame MSAA allocations are confined to that test.

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
