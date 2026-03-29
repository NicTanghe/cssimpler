# cssimpler workspace

Current focus:

- `core/` owns DOM, style, layout, and renderer-facing primitives.
- `renderer/` consumes `RenderNode` and draws a minimal native demo window.
- `style/` holds stylesheet parsing, selector matching, and DOM-to-render-tree resolution.
- `macro/` is the bootstrap home for `ui!`.
- `examples/demo.rs` is the small demo app edge.

This is the first implementation pass for Epic A from the spec sheet.
