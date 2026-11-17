# Native vector paths and SVG export

Use `Node::vector(scene)` to draw typed path commands inside a normal cssimpler
UI, without generating SVG markup or parsing path strings. No Cargo feature is
required. Layout, handlers, clipping, transforms, dirty redraws, and optional
layer caching use the same renderer pipeline as existing SVG elements.

## Construct a scene

```rust
use cssimpler::core::{
    Color, Node, PathCommand::*, VectorPaint, VectorPath,
    VectorPoint as P, VectorScene, VectorViewBox,
};

let path = VectorPath::new(vec![
    MoveTo(P::new(20.0, 180.0)),
    CubicTo {
        control1: P::new(10.0, 10.0),
        control2: P::new(200.0, 10.0),
        end: P::new(260.0, 100.0),
    },
    QuadTo { control: P::new(280.0, 190.0), end: P::new(180.0, 180.0) },
    Close,
], VectorPaint {
    fill: Some(Color::rgb(200, 220, 240)),
    stroke: Some(Color::rgb(30, 60, 90)),
    stroke_width: 2.0,
})?;
let scene = VectorScene::new(VectorViewBox::new(0.0, 0.0, 300.0, 200.0), vec![path])?;

// Export before consuming the scene in a UI leaf.
let svg = scene.to_svg();
let element: Node = Node::vector(scene).with_id("drawing").into();
```

Use the element in your application's view function. CSS can size and position
`vector` or `#drawing`, for example `#drawing { width: 600px; height: 400px; }`.
By default, intrinsic width/height come from the viewBox. The mapping stretches
independently along each axis, matching existing SVG rendering. The typed viewBox
owns the coordinate mapping; SVG markup attributes do not replace that value.

Commands support `MoveTo`, `LineTo`, `QuadTo`, `CubicTo`, and `Close`. Each contour
starts with `MoveTo`; fills use nonzero winding and implicitly close open
contours. Strokes retain open endpoints and use round caps/joins. Paints currently
support solid colors with alpha, an optional fill/stroke, and a nonnegative width.
Constructors reject nonfinite coordinates, invalid viewBoxes, and commands before
an initial move. Empty paths are allowed and paint nothing.

Coordinates are `f32`. Keep editable geometry, semantic identities, physical
units, and any higher precision required by your application in its own model.
Generate these display commands from that model; do not replace authored curves
with flattened rendering segments. Bounds use the control-point hull, which is
conservative and can be larger than the exact curve bounds.

The vector element is a leaf: place labels or live selection handles in sibling
elements. `with_child` on a native vector leaf is rejected during resolution.
Path paints are explicit scene data; CSS styles the outer element, not individual
commands. Rebuild the scene when path geometry or paints change and request a
paint refresh as usual. This does not add an animation scheduler.

For lower-level callers, `RenderNode::vector(layout, scene)` bypasses DOM and
style resolution. `VectorScene::into_render_scene()` bridges to the existing
`SvgScene` representation without serialization. The SVG type names in that
bridge describe shared renderer storage, not a parsing step.

## Export SVG

`VectorPath::to_svg_path_data()` writes the original M/L/Q/C/Z commands.
`VectorScene::to_svg()` produces a standalone SVG containing the viewBox,
dimensions, path data, colors, alpha, and stroke settings. Curves keep their
control points; numbers use Rust's round-tripping `f32` representation rather
than a fixed decimal rounding rule.

SVG sizes use numeric user units (default SVG units are pixels). Exporting a
physical drawing in millimetres requires your application to choose and convert
the coordinate system accordingly. This exports the scene, not the entire UI:
CSS transforms, surrounding layout, labels, filters, and other sibling content
are not baked into the file. No new general SVG XML import API is introduced.
Existing SVG parsing remains available and its flattening behavior is unchanged.

Try the example interactively or export it without opening a window:

```sh
cargo run --release --example native_vector_paths
mkdir -p work
cargo run --release --example native_vector_paths -- --export work/curves.svg
```

## Caching dynamic native drawings

Caching stores pixels for an unchanged native subtree. It does not freeze your
commands or remove the need to request a refresh. Every geometry, paint, or layout
change inside the layer invalidates that raster automatically.

Use this arrangement for editors and interactive drawings:

| Content | Native drawing strategy |
| --- | --- |
| Mostly unchanged background paths | `Node::vector(background).with_cached_layer(true)` |
| Hover handles or a moving selection outline | Separate uncached `Node::vector(overlay)` sibling |
| Path whose controls change on every drag frame | Separate uncached native vector leaf |
| Occasional geometry edits | Rebuild its scene and let the cached layer rebuild on the next paint |

For example, your view function can compose two native scenes:

```rust
use cssimpler::core::{Node, VectorScene};

fn drawing_view(background: &VectorScene, overlay: &VectorScene) -> Node {
    Node::element("div")
        .with_class("drawing")
        .with_children([
            Node::vector(background.clone())
                .with_class("background")
                .with_cached_layer(true)
                .into(),
            Node::vector(overlay.clone())
                .with_class("overlay")
                .into(),
        ])
        .into()
}
```

Include these rules in the application's stylesheet. Both scenes should use the
same viewBox for aligned coordinates:

```css
.drawing { position: relative; width: 400px; height: 300px; }
.background, .overlay {
    position: absolute;
    left: 0;
    top: 0;
    width: 400px;
    height: 300px;
}
.overlay { z-index: 1; }
```

Cache only the background leaf. Caching the parent would include the overlay;
each handle movement would then invalidate the whole cached drawing. A cached
leaf establishes a stacking context; the positioned overlay stays above it.

### Request a paint when commands change

Keep the native scenes in application state. Build replacement `VectorPath`s
with updated controls, validate a replacement `VectorScene`, and replace the
appropriate scene. A minimal update/view arrangement is:

```rust
use cssimpler::{
    app::Invalidation,
    core::{Node, VectorScene},
    renderer::FrameInfo,
};

struct DrawingState {
    background: VectorScene,
    overlay: VectorScene,
    paint_pending: bool,
}

fn update(state: &mut DrawingState, _: FrameInfo) -> Invalidation {
    // Input handlers/queued actions replace a scene and set paint_pending.
    if std::mem::take(&mut state.paint_pending) {
        Invalidation::Paint
    } else {
        Invalidation::Clean
    }
}

fn view(state: &DrawingState) -> Node {
    drawing_view(&state.background, &state.overlay)
}
```

This uses the application's normal update/view loop. `Invalidation::Paint` still
runs the view function; it does not bypass DOM construction. Use scoped fragment
refreshes when other UI sections should also avoid rebuilding. `Clean` is only
appropriate when there is no pending visual change.

During a drag, keep stable paths in the cached background and put the active path
in the uncached scene. When the drag ends, commit the edited path into the
background and remove the temporary preview. If your entire drawing changes each
frame, leave its explicit cache request off. Enabling caching again after an edit
allows the next eligible paint to build the updated raster.

Do not call `clear_layer_cache()` every frame: it releases all retained layers,
including unrelated UI layers. Old content versions can remain within the bounded
LRU cache until eviction; the cache key prevents them being reused for new geometry.

### Budget, inspect, and release

```rust
use cssimpler::renderer::{
    LayerCacheLimits, set_layer_cache_limits, layer_cache_stats, clear_layer_cache,
};

// Set once at startup, or between frames when application requirements change.
set_layer_cache_limits(LayerCacheLimits {
    total_bytes: 16 * 1024 * 1024,
    layer_bytes: 4 * 1024 * 1024,
});
let stats = layer_cache_stats();
println!("{} cached bytes, {} cumulative builds", stats.bytes, stats.builds);

// Release retained entries; an eligible later paint can build them again.
clear_layer_cache();
```

A stable background should stop adding builds while only its overlay changes.
Statistics are application-wide, so other layers can also increase the counter.
Default storage is 16 bytes per pixel: the 400 × 300 background is about 1.83 MiB
before display scaling. Temporary buffers and path data are separate. Rendering
falls back to direct drawing if the cache is over budget or the layer is ineligible.

`with_cached_layer(false)` removes a leaf's explicit cache request; it does not
clear retained entries or disable automatic promotion of other transformed
subtrees. Set both byte limits to zero to disable raster caching globally.
Magnification draws native curves directly to preserve sharpness; translation
and reduction can reuse a raster. See [layer caching limits and fallback](./layer-caching.md)
for the full memory policy. No caching performance measurements are claimed here.

## Comparing construction, preparation, painting, and RAM

```sh
cargo run --release --example native_vector_bench -- --mode native --frames 20
cargo run --release --example native_vector_bench -- --mode svg --frames 20
```

Both modes start from the same twelve authored paths with cubic and quadratic
curves and solid fills/strokes in a 960 × 640 scene. The native mode uses typed
commands. The SVG mode writes path strings and constructs ordinary SVG DOM nodes.

- **Construction** measures building the DOM from already-authored paths,
  including string serialization in SVG mode.
- **Preparation** measures style resolution, path preparation, and layout with
  `build_render_tree` on that DOM. This includes path parsing/flattening in SVG mode.
- **Painting** measures full repaints of an already-prepared static render tree,
  after one warmup, including scene extraction and framebuffer clearing.
- **RAM** is Linux whole-process resident memory (`VmRSS`/`VmHWM`), including
  retained allocator storage from all stages. Other platforms emit `null`.

This measures display plumbing, not application geometry editing or presentation.
Native commands retain curves until tiny-skia rasterizes them; existing SVG input
is flattened during parsing, so antialiased edge values can differ. A difference
in paint time is not evidence that SVG parsing happens during painting.

### Measured static comparison

Measured on 2026-11-16 on the shared Linux x86-64 cloud host with a two-CPU
quota, Rust 1.100.0, release optimization, default features, and the repository's
SSE4.1/SSE4.2 flags. Three fresh processes per mode, alternating order; each stage
uses twenty iterations. Values are medians of per-process medians. No cached
layers are requested in either mode.

| Input route | DOM construction | Preparation | Full paint | Peak process RSS |
| --- | ---: | ---: | ---: | ---: |
| svg | 17.1 µs | 33.0 µs | 17.39 ms | 6.78 MiB |
| native | 0.6 µs | 3.1 µs | 16.00 ms | 6.07 MiB |

Typed paths substantially reduce construction and preparation in this small
example, but those stages already take microseconds. Painting still dominates.
Paint medians varied from 11.31–16.05 ms for native input and 11.76–17.51 ms for
SVG input across processes, so the modest paint difference does not establish a
reliable general speedup. The native route's main benefit here is preserving
curves and avoiding string/parsing work. Peak process RSS was about 0.71 MiB lower
for native input; this includes allocator retention, not just path storage.

[Raw results](./reports/native-vector-results.json) include all stage timings,
RSS snapshots, and source SHA-256. Reproduce after compiling, with no other builds
running:

```sh
cargo build --release --example native_vector_bench
python3 docs/reports/native-vector-benchmark.py target/release/examples/native_vector_bench
```

## Compatibility

Prefer constructors for public structs. Direct `ElementNode`/`RuntimeElementData`
struct literals now need `vector_scene: None`; direct `SvgPathGeometry` literals
need `commands: Vec::new()` for existing flattened geometry. Nonempty commands
have precedence over contours. Existing constructors and SVG markup keep working.

### Frame diagnostics

`latest_frame_timing_stats()` returns a fixed-size snapshot of the last completed
window frame. Enable detailed vector timing with `set_vector_timing_enabled(true)`
before drawing; it is off by default. For a runnable example, use
`cargo run --example native_vector_paths -- --diagnostics` and close the window.
The reported `vector` values cover both native vector and SVG paths, since they
share a renderer. An idle last frame has zero work, so inspect a frame that
actually repainted when investigating a slow drawing.

`paint_us` is elapsed frame painting time; `present_us` is CPU-side presentation
elapsed time, not GPU execution. Vector `path_build_work_us` covers conversion
from renderer geometry into a tiny-skia path, `stroke_work_us` covers stroke
outline construction, `projection_work_us` covers screen projection, and
`coverage_work_us` covers coverage and color blending. They are sums of elapsed
measurements across jobs, including scheduler interruptions, not CPU-clock
measurements or frame wall times. They can exceed `paint_us` when jobs overlap.
They exclude application geometry creation and scene preparation. Measurements
are accumulated at nanosecond resolution and reported in whole microseconds.

`paths_painted` counts path paint attempts with paint and bounds surviving
coarse clipping, per paint job or worker band; it is not a count of unique paths
or guaranteed visible pixels. Full redraw counts all such attempts; incremental
redraw counts only visited damage bands, possibly more than once per path.
Clipped, unpainted, and invalid-bound paths are excluded. Cached raster layers
can avoid path painting, while layer construction may paint a subtree more than
once. Idle frames report zero counts and timings. Changing the timing switch
applies at the next painted frame; disabled detailed durations are zero.

`layer_cache_hits` and `layer_cache_misses` count first promoted-layer lookups
within that frame, including misses that cannot produce an eligible layer. A
concurrent build may satisfy a miss on its second lookup. `layer_cache_stats()`
reports currently retained raster entries and bytes, and cumulative successful
builds since process start; clearing the cache does not reset builds. Repeated
misses and rising builds on otherwise unchanged artwork suggest investigating
invalidation or cache limits. These values describe raster layers, not a
prepared-geometry cache, and collecting them never allocates raster layers.

As with the existing latest-frame API, the published snapshot is process-wide.
Frame render jobs inherit the collection scope and are joined before publication;
unrelated concurrent headless renders do not contribute. The window event loop
serializes window frames. Snapshots contain no per-path names or history;
counters saturate instead of wrapping.

A local unoptimized test of 3,000 small 24×24 filled paths measured 2.115 s
with detailed timers disabled and 2.224 s enabled. A second run measured
0.907 s disabled and 0.793 s enabled; an isolated-target run measured 0.758 s
and 0.765 s respectively. The variation prevents a reliable overhead estimate
on this shared machine. This narrow debug
workload is not a general performance claim; the opt-in switch keeps per-path
clock reads out of normal painting. Reproduce with
`cargo test -p cssimpler-renderer vector_timing_overhead --lib -- --ignored --test-threads=1 --nocapture`.
