# cssimpler workspace

Current focus:

- `src/app.rs` owns the explicit application loop for Epic G: state, invalidation, scoped refresh, and DOM rerender policy.
- `core/` owns DOM, style, layout, and renderer-facing primitives.
- `renderer/` consumes backend-neutral extracted scene data, keeps the software renderer as the correctness path, and can present through an optional baseline GPU backend.
- `style/` holds stylesheet parsing, selector matching, and DOM-to-render-tree resolution.
- `macro/` is the bootstrap home for `ui!`.
- `examples/demo.rs` is the small demo app edge.
- `examples/collapsible_sidebar.rs` proves Windows/system font resolution against a live UI.
- `examples/powerline_typography.rs` registers a bundled Powerline TTF from `examples/assets/`.
- `examples/text_render_stress.rs` generates large pseudo-random documents to pressure text layout, wrapping, and scrolling.

The workspace now has the foundations through rendering, typography and font resolution, interaction, scoped invalidation, renderer-side dirty-region diffing, and a first shared CPU/GPU backend contract in place.
