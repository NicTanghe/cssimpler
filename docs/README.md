# cssimpler workspace

Current focus:

- `core/` owns DOM, style, layout, and renderer-facing primitives.
- `renderer/` consumes `RenderNode` and draws a minimal native demo window.
- `style/` holds stylesheet and selector primitives.
- `macro/` is the bootstrap home for `ui!`.
- `app/` owns state, update, and render orchestration.

This is the first implementation pass for Epic A from the spec sheet.
