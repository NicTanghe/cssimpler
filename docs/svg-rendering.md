# CPU 2D rendering and cached layers

SVG fills and strokes use tiny-skia's antialiased scanline rasterizer by default.
No Cargo feature is required. `gpu-viewport` remains independent: SVGs still
render on the CPU when native GPU viewports are enabled.

The SVG parser and public scene types are unchanged. The renderer converts the
existing contours to tiny-skia paths, preserving nonzero winding fills, open
strokes, and the previous renderer's round caps and joins. It strokes in source
coordinates before applying the viewBox/CSS transform, so nonuniform scaling
also scales the stroke outline. Perspective transforms project the outlines,
flattening curved caps and joins to device-space line segments.

## Buffers and memory

The existing framebuffer contains packed `u32` pixels and uses linear-light
blending, with a separate alpha target when needed. It cannot be passed directly
to `PixmapMut::from_bytes`, which requires premultiplied RGBA8 pixels.

Instead, tiny-skia writes coverage into one reusable 256 × 256, one-byte-per-pixel
mask per rendering thread (64 KiB). The mask is allocated lazily when an SVG is
painted and reused across tiles, paths, and frames. Its size does not grow with
window size, SVG bounds, zoom level, or number of paths. Parallel rendering can
allocate one such mask on each participating thread. Temporary path and scanline
storage also depends on geometry complexity.

Coverage is blended into the existing framebuffer using cssimpler's color and
alpha handling. Linear SVG gradients keep their existing interpolation. This
adds no persistent RGBA pixmaps, per-panel raster caches, or scene layers unless
the application explicitly requests a layer (or the existing transformed-subtree
promotion chooses one).

Both path bounds and the active dirty clip restrict rasterization. Worker row
bands restrict buffer writes, and rounded/transformed ancestor clips still
apply. Antialiased edge pixels can differ from the old four-sample rasterizer.

## Opt-in layers for a 2D editor

For a step-by-step setup with a live overlay, see [Using optional layer caching](./layer-caching.md).

Use `ElementNode::with_cached_layer(true)` on a static board container or SVG.
Low-level render-tree users have the same `RenderNode::with_cached_layer(true)`
builder. Both set `VisualStyle::cache_layer`. For example:

```rust
use cssimpler::core::Node;

let board = Node::element("svg")
    .with_attribute("viewBox", "0 0 100 100")
    .with_attribute("width", "400")
    .with_attribute("height", "400")
    .with_cached_layer(true)
    .with_child(
        Node::element("path")
            .with_attribute("d", "M 10 10 L 90 10 L 80 90 L 20 90 Z")
            .with_attribute("fill", "#c8dcf0")
            .with_attribute("stroke", "#1e3c5a")
            .into(),
    );
```

Keep hover handles, marquee selection, and drag previews in a sibling overlay
above the cached board. Updating that overlay can repaint the damaged region by
compositing the cached board and painting the overlay. Rebuilding content inside
the cached subtree invalidates its raster. The renderer keys layers from content,
style, layout, image generations, and descendants; applications do not need to
maintain a separate invalidation token.

A cached layer is a stacking context: its children paint together relative to
outside siblings. Keep overlays outside that context and use the normal sibling
stacking order or positioned `z-index` to put them above it.

Layers reuse the renderer's existing subtree-surface cache, LRU eviction, and
temporary buffer pool. No second layer cache is added. An unchanged layer can be
reused across translation and reduction in scale. Magnification draws vectors
directly for sharp edges; it does not enlarge a low-resolution cached raster.
Changes to layout or SVG viewBox/geometry rebuild the raster. This is a raster
cache, not a promise to retain parsed path geometry across application view builds.

Defaults remain **16 MiB total**, **4 MiB per layer**, and **64 entries**. The
existing cache stores premultiplied linear RGBA as four `f32` values: **16 bytes
per pixel**, not four. A 400 × 400 board is about 2.44 MiB; a 1920 × 1080 board is
about 31.64 MiB. Increase the budget explicitly if the application needs a larger
board:

```rust
use cssimpler::renderer::{set_layer_cache_limits, LayerCacheLimits};

// Application-wide; set once during startup. This permits one 1080p board.
set_layer_cache_limits(LayerCacheLimits {
    total_bytes: 48 * 1024 * 1024,
    layer_bytes: 32 * 1024 * 1024,
});
```

`layer_cache_stats()` reports retained entries, pixel bytes, and cumulative
builds. `clear_layer_cache()` releases retained entries; future paints may rebuild
them. Set both byte limits to zero to disable raster caching and release cached
entries. Lowering limits evicts entries immediately. Applications should configure
limits between frames; active paints may still hold references to evicted layers.

Building a layer also needs two temporary packed buffers (eight bytes per pixel),
reused from the existing bounded worker buffer pool. Cold builds are serialized
so row-band workers do not allocate duplicate surfaces for the same layer. These
temporary buffers, path data, and the small SVG coverage masks are separate from
the retained layer budget.

Caching is a hint. A layer outside the viewport, over budget, magnified, using
depth/perspective, or containing backdrop effects/native glass draws directly.
Do not cache an entire infinite workspace: cache a bounded visible board, then
measure whether raster reuse benefits the workload. Ordinary stationary elements
do not acquire cached layers by default.

## Static SVG performance comparison

The [opaque-fill benchmark](./reports/opaque-vector-fills.md) measures the current
solid-fill optimization for static SVG and native vector scenes.

The headless benchmark constructs ordinary SVG scenes once and forces full
repaints of that unchanged scene. It does not request cached layers. Both
revisions compile the **exact same benchmark source**:

```sh
cargo run --release -p cssimpler-renderer --example svg_raster_bench -- --case icons --frames 12
cargo run --release -p cssimpler-renderer --example svg_raster_bench -- --case panels --frames 12
```

Both cases use a 960 × 640 framebuffer:

- **Icons:** 192 SVG nodes, 480 paths: 24 px badges/checkmarks, folders, search
  icons with winding holes, and small charts. This exercises normal UI SVGs.
- **Panels:** 12 SVG nodes, 24 paths: medium filled/stroked panels, 64-segment
  curved outlines, 24-segment holes, and open internal lines.

Each fresh process measures a cold paint, then twelve timed full repaints. JSON
output includes median/p95 render time, resident RAM before painting, after warmup,
end-of-run resident RAM, and process peak resident RAM. Linux memory values come
from `/proc/self/status` (`VmRSS`/`VmHWM`); other platforms report `null` for RAM.
These are **whole-process resident bytes**, not allocator-only measurements or
an exact accounting of tiny-skia allocations.

Timing includes renderer traversal, scene extraction, buffer clearing, and SVG
painting. It excludes DOM construction, SVG parsing, layout, window presentation,
and 3D rendering. These results describe forced static repaints, not idle CPU
usage or an application's interaction latency.

### Measured results

Measured on 2026-11-16 on a shared Linux x86-64 cloud machine, Intel Xeon Platinum
8370C, with a two-CPU cgroup quota. Both executables used Rust 1.100.0, the same
Cargo release profile, and the repository's SSE4.1/SSE4.2 target flags. Original
renderer: commit `7c25976849d2b6ffb179f7c1f7400e74fd5ed22c`. **No layer caching was
requested in either executable.**

| Static scene | Renderer | Median paint | p95 paint | Peak process RSS |
| --- | --- | ---: | ---: | ---: |
| Icons | original | 22.23 ms | 46.01 ms | 5.26 MiB |
| Icons | tiny-skia | 4.35 ms | 6.39 ms | 5.71 MiB |
| Panels | original | 649.97 ms | 1160.11 ms | 4.94 MiB |
| Panels | tiny-skia | 13.33 ms | 17.37 ms | 5.63 MiB |

In these scenes, tiny-skia was **5.1× faster for icons** and **48.8× faster for
panels**. Median peak process RAM increased by **0.45 MiB** and **0.70 MiB**,
respectively. End-of-run RSS matched the peak-RSS medians shown above. These
measurements include code pages, worker threads, allocator retention, and path
scratch storage as well as the coverage masks; they are not extra cached layers.

The median cold-paint times were 20.16 → 5.78 ms for icons and
666.14 → 16.43 ms for panels. The shared host showed timing variation:
per-process steady medians ranged from 21.66–37.15 ms (original icons) and
4.27–6.50 ms (tiny-skia icons), so use the raw measurements when assessing the
spread rather than treating a single timing as a guarantee.

[Raw results and environment metadata](./reports/svg-rendering-results.json)
include every run, cold paints, RSS before/after painting, and the benchmark
source SHA-256. These are synthetic renderer measurements, not ClothCraft frame
rates. Antialiased edge values differ between rasterizers.

### Reproduction

The [comparison runner](./reports/svg-rendering-benchmark.py) alternates executable
order and runs each renderer/case in three fresh processes. It reports medians of
per-process measurements; p95 is the median of the three per-process p95 values.
Build both executables before comparing, and leave other builds stopped during
the measurement.

To reproduce against the original renderer, create a detached worktree at
`7c25976849d2b6ffb179f7c1f7400e74fd5ed22c`, copy the benchmark unchanged with the
runner's `prepare-baseline` command, and build it with the same release profile,
toolchain, and target flags. Then run:

```sh
python3 docs/reports/svg-rendering-benchmark.py prepare-baseline \
  renderer/examples/svg_raster_bench.rs \
  /path/to/baseline/renderer/examples/svg_raster_bench.rs
# Build each revision, saving its executable separately, then:
python3 docs/reports/svg-rendering-benchmark.py compare \
  --baseline /path/to/original-svg-bench \
  --current /path/to/tiny-skia-svg-bench \
  --baseline-commit 7c25976849d2b6ffb179f7c1f7400e74fd5ed22c \
  --runs 3 --frames 12 --output svg-rendering-results.json
```

Applications still benefit from retaining geometry and using scoped refreshes.
This rasterizer change does not change their UI rebuild or invalidation policy.

For ClothCraft-style applications, measure UI construction, style/layout, SVG
painting, and presentation separately with simulation paused. Retain panel
geometry, use fragment refreshes for the board/overlay, and process the latest
drag input once per displayed frame. A raster cache avoids repeated painting of
unchanged content; it cannot avoid an application's `view` function rebuilding
every SVG node before the renderer receives the scene.
