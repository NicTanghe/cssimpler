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

## Pick a path under the pointer

Pass the same scene and layout used to draw the board. The returned index
refers to the topmost painted path in `scene.paths()`. An app can use it
to update hover on pointer movement or selection on click.

```rust
use cssimpler::{
    core::{LayoutBox, VectorPoint, VectorScene},
    renderer::hit_test_vector_scene,
};

fn path_under_pointer(
    scene: &VectorScene,
    layout: LayoutBox,
    pointer_x: f32,
    pointer_y: f32,
) -> Option<usize> {
    hit_test_vector_scene(scene, layout, VectorPoint::new(pointer_x, pointer_y), 3.0)
}
```

The pointer and layout use logical layout pixels. The query includes the
viewBox and the scene’s pan and zoom, clips to the supplied layout, and skips
fully transparent paint. If a CSS transform or ancestor clip affects the
board, apply its inverse transform to the pointer or check the ancestor clip
before calling. The last argument adds a radius in logical pixels around the
painted stroke. Use zero to pick using the authored width. Fills use nonzero
winding, including holes and the implicit closure of open subpaths.

Picking scans in reverse paint order and checks the shared geometry bounds.
It prepares paths only for nearby candidates and uses a one-pixel mask for
the winding test. The scene stores no picking index or cached outlines.

In a release benchmark of 10,000 disjoint filled paths, the median was
0.59 µs when the topmost path hit, 64.38 µs when the bottommost path hit,
and 64.87 µs when all bounds missed (Linux x86-64, Intel Xeon Platinum
8370C). Dense overlapping bounds can make more geometry tests necessary.

```sh
cargo run --release --locked --example vector_hit_test_bench
```

## Geometry ownership and edits

Each `VectorPath` owns a lightweight `VectorGeometry` handle. It keeps the
validated authored commands and their conservative bounds together. Cloning a
path or scene, rebuilding a native element, runtime round-trips, and building
render trees clone this handle, not the command buffer. Paint remains independent
on each path and each scene has its own viewBox. To reuse one drawing with a
different paint, pass `path.geometry().clone()` to
`VectorPath::from_geometry(geometry, paint)`; `VectorPath::new(commands, paint)`
continues to work. Construct replacement geometry with `VectorGeometry::new`
when editing commands, then build a replacement path or scene. Once no owners
remain, its command buffer is released; there is no geometry registry or new
pixel cache.

`VectorGeometry::identity()` is an automatic process-local, lifetime-stable
identity used for invalidation, not a persisted content hash. Clones keep the
identity; constructing new geometry gives it a new identity even if the commands
are identical. Retain the handle across rebuilds to enable cache reuse. Debug
output for native geometry contains this identity rather than formatting all
commands, so layer-cache key generation does not repeatedly serialize geometry.
Paint, layout and viewBox remain part of that key. Legacy SVG contour storage and
its content-based invalidation are unchanged.

Lower-level users constructing `SvgPathGeometry` with a struct literal should
replace a `Vec<PathCommand>` `commands` field with `VectorGeometry::new(commands)?`
(or `VectorGeometry::default()` for no native commands) and set `bounds` from
`commands.bounds()`. The field dereferences to a read-only command slice; mutate
by replacing the handle and updating bounds together. Prefer `VectorPath` and
`VectorScene` for validated native construction.

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

