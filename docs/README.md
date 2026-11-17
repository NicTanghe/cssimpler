# cssimpler workspace

Current focus:

- `src/app.rs` owns the explicit application loop for Epic G: state, invalidation, scoped refresh, and DOM rerender policy.
- `core/` owns DOM, style, layout, and renderer-facing primitives.
- `renderer/` consumes `RenderNode`, skips redraw on unchanged frames, and incrementally repaints dirty regions when scenes change.
- `style/` holds stylesheet parsing, selector matching, and DOM-to-render-tree resolution.
- `macro/` is the bootstrap home for `ui!`.
- `examples/demo.rs` is the small demo app edge.
- `examples/collapsible_sidebar.rs` proves Windows/system font resolution against a live UI.
- `examples/powerline_typography.rs` registers a bundled Powerline TTF from `examples/assets/`.
- `examples/text_render_stress.rs` generates large pseudo-random documents to pressure text layout, wrapping, and scrolling.

The workspace now has the foundations through rendering, typography and font resolution, interaction, scoped invalidation, and renderer-side dirty-region diffing in place.

- [Optional layer caching: usage guide](./layer-caching.md) — enable caches, separate overlays, choose RAM budgets, inspect usage, and disable caching.
- [Frame-aligned drag updates](./frame-aligned-drag.md) — opt-in latest-state drag coalescing, bounded pending state, and frame scheduling.
- [Native vector paths and SVG export](./native-vector-paths.md) — typed curves, native UI elements, export, and a construction/rendering comparison.
- [Native vector pan and zoom](./vector-pan-zoom.md) — view-only pan/zoom, pointer mapping, viewport clipping, sharpness/caching interaction, and measurements.
- [Fluent vector path builder](./vector-builder.md) — chain typed commands and paint into the same validated `VectorPath`, with allocation notes and when to use direct construction.
- [CPU 2D rendering and cached layers](./svg-rendering.md) — tiny-skia SVG coverage, opt-in board caches, memory budgets, overlays, and a headless benchmark.
- [Native vector frame diagnostics](./vector-diagnostics.md) — last-frame work counts, opt-in detailed timers, layer-cache hits/misses, and how to read slow frames.

## Planned extensions

- [Native GPU and USD viewport specsheet](./native-gpu-viewport-specsheet.md) — optional native `wgpu` surfaces and USD scene representation; implementation in progress.
- [GPU viewport usage](./gpu-viewports.md) — experimental Windows preview, registration API, and current limitations.
