# Native GPU viewports (experimental)

Enable the optional `gpu-viewport` feature to embed a `wgpu` viewport in the
CPU-rendered UI. Default builds retain the existing CPU-only path and do not
include the GPU or USD dependencies.

Native backends use Windows DirectComposition visuals, macOS Metal sublayers,
or Linux X11 child windows.
These are separate presentation surfaces above the parent UI, sharing one GPU device.
Transparent pixels reveal CSSimpler's background; the CPU UI is not uploaded to
the GPU. X11 transparency uses an ARGB visual and automatic X Composite
redirection; X Shape 1.1 keeps input on the parent window. Transparent X11 views
require X Composite 0.4 and an ARGB visual. Native Wayland embedding
is still pending and returns an explicit unsupported-platform error when a
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
