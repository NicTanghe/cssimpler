# Using optional layer caching

Use a cached layer for a mostly unchanged SVG or container that is expensive to
repaint beneath changing controls or selection highlights. Caching is optional;
tiny-skia SVG rendering works without it and needs no Cargo feature.

## 1. Enable caching on the static content

Call `with_cached_layer(true)` on an `ElementNode` while building your UI.
For example, this function builds a cached board and a separate live overlay:

```rust
use cssimpler::core::Node;

fn build_board(handle_x: f32) -> Node {
    let board = Node::element("svg")
        .with_class("board")
        .with_attribute("viewBox", "0 0 100 100")
        .with_cached_layer(true)
        .with_child(
            Node::element("path")
                .with_attribute("d", "M 10 10 L 90 10 L 80 90 L 20 90 Z")
                .with_attribute("fill", "#c8dcf0")
                .with_attribute("stroke", "#1e3c5a")
                .into(),
        );

    // This SVG is a sibling, so moving the handle leaves the board cache valid.
    let overlay = Node::element("svg")
        .with_class("overlay")
        .with_attribute("viewBox", "0 0 100 100")
        .with_child(
            Node::element("circle")
                .with_attribute("cx", handle_x.to_string())
                .with_attribute("cy", "50")
                .with_attribute("r", "2")
                .with_attribute("fill", "#ef4444")
                .into(),
        );

    Node::element("div")
        .with_class("pattern-editor")
        .with_children([board.into(), overlay.into()])
        .into()
}
```

Include these rules in the stylesheet passed to your application:

```css
.pattern-editor { position: relative; width: 400px; height: 400px; }
.board, .overlay {
    position: absolute;
    left: 0;
    top: 0;
    width: 400px;
    height: 400px;
}
.overlay { z-index: 1; }
```

Keep pointer handlers on the editor or the appropriate interactive element.
Update application state and request the appropriate scoped refresh when the
handle moves; enabling a cache does not schedule refreshes itself.

For low-level rendering, call `RenderNode::with_cached_layer(true)` after
applying your `VisualStyle`. Both builders set `VisualStyle::cache_layer`.
This is a Rust API setting, not a CSS property or a `cache-layer` markup attribute.

## 2. Keep changing content outside the layer

Put hover highlights, drag previews, and marquee selection in sibling overlays.
Changing content, style, layout, or geometry inside a cached subtree causes its
raster to be rebuilt automatically. No manual invalidation token is required.
An unchanged subtree can reuse its raster even if your application reconstructs
equivalent DOM nodes, although that DOM construction still costs CPU time.

A cached layer establishes a stacking context: its descendants paint together
relative to outside siblings. Use normal sibling ordering or positioned
`z-index` to place your overlay above it. A raster cache does not eliminate
application-side SVG parsing, geometry generation, or layout work.

## 3. Choose a memory budget

The default shared cache allows **16 MiB total**, **4 MiB per layer**, and
**64 entries**. It reuses the existing renderer surface cache. Layers store
premultiplied linear RGBA at **16 bytes per pixel**:

| Layer size | Retained pixel storage |
| --- | ---: |
| 400 × 400 | 2.44 MiB |
| 1920 × 1080 | 31.64 MiB |

The example fits the default per-layer budget. To permit a larger board, set
application-wide limits once during startup:

```rust
use cssimpler::renderer::{set_layer_cache_limits, LayerCacheLimits};

set_layer_cache_limits(LayerCacheLimits {
    total_bytes: 48 * 1024 * 1024,
    layer_bytes: 32 * 1024 * 1024,
});
```

Size the budget for device pixels, including display scaling. A cold build
also uses two temporary packed buffers (eight bytes per pixel), reused from
the existing worker buffer pool. Those buffers, geometry, and SVG coverage masks
are separate from retained layer bytes. Cache a bounded visible board rather
than an entire infinite workspace.

## 4. Inspect, release, or disable the cache

```rust
use cssimpler::renderer::{
    clear_layer_cache, layer_cache_stats, set_layer_cache_limits, LayerCacheLimits,
};

let stats = layer_cache_stats();
println!("{} layers, {} bytes, {} builds",
    stats.entries, stats.bytes, stats.builds);

// Release retained entries; later paints may rebuild eligible layers.
clear_layer_cache();

// Disable all raster caching and release retained entries.
set_layer_cache_limits(LayerCacheLimits {
    total_bytes: 0,
    layer_bytes: 0,
});
```

For a stable board, the build count should stop increasing as only an overlay
changes. Counts are cumulative and application-wide. Set budgets between frames;
active paints can temporarily retain references to evicted surfaces.

`with_cached_layer(false)` removes that element's explicit request. It does not
clear existing entries or disable the renderer's automatic promotion of eligible
transformed subtrees. Use zero global limits to disable all raster caching.

## When a request falls back to direct rendering

Caching is a hint. Layers over budget, extending outside the viewport, magnified,
using depth/perspective, or containing backdrop-dependent effects/native glass
draw directly. Translation and reductions in scale can reuse a raster;
magnification uses vector rendering for sharp edges. Layout, viewBox, or
geometry changes invalidate the raster. Frequently changing layers may therefore
cost more than direct painting.

The [SVG rendering guide](./svg-rendering.md) explains the implementation and
contains the [static speed/RAM report](./svg-rendering.md#measured-results).
That report measures uncached SVG rendering; it does not benchmark layer caching.
