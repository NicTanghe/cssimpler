# Native vector pan and zoom (issue #19)

A native drawing can pan and zoom by changing a small view transform while its
authored path commands stay untouched. The view is carried by the scene, is
composed with the existing CSS/layout/device transform at paint time, and is
ignored by SVG export. Changing it never rebuilds a path, serializes SVG, or
copies a command array.

```rust
use cssimpler::core::{LayoutBox, Node, VectorPoint, VectorScene, VectorView};

fn view_element(drawing: &VectorScene, view: VectorView) -> Node {
    let mut drawing = drawing.clone(); // clones path handles, not commands
    drawing.set_view(Some(view));      // small Copy view state only
    Node::vector(drawing).into()
}

fn zoom_around_pointer(
    drawing: &VectorScene,
    layout: LayoutBox,
    pointer: VectorPoint,
) -> Option<VectorView> {
    VectorView::identity()
        .zoomed_about_viewport_point(drawing.view_box(), layout, pointer, 1.5)
        .ok()
}

let drawing = document_scene()?;
let layout = LayoutBox::new(0.0, 0.0, 800.0, 600.0);
let pointer = VectorPoint::new(420.0, 310.0);
if let Some(view) = zoom_around_pointer(&drawing, layout, pointer) {
    let document = view
        .viewport_to_document(drawing.view_box(), layout, pointer)
        .unwrap();
    println!("pointer is over drawing point {document:?}");
    let node = view_element(&drawing, view);
}
```

Run the example (the default mode is headless, `--interactive` opens a window):

```sh
cargo run --example vector_pan_zoom
cargo run --example vector_pan_zoom -- --interactive
cargo run --example vector_pan_zoom -- --export work/pan-zoom.svg
cargo run --release --example vector_pan_zoom -- --bench
```

## Design decision: focused helper, not a camera abstraction

The issue asked whether a focused helper/documented pattern is sufficient before
adding another public camera abstraction. Two options were considered.

1. **Reuse the existing CSS `transform` style as the live view.** This needs no
   new type, but an application-supplied `Style` is the *base* style that
   matching stylesheet rules overwrite. A direction check against this revision
   confirms it: an inline base transform of `scale(3)` plus
   `#draw { transform: translate(20px, 20px) }` resolves to the stylesheet's
   `translate` and silently loses the scale. A pan/zoom stored there would
   replace authored CSS transforms on the vector element instead of composing
   with them. It also cannot express the document-space inverse mapping or
   define viewport clipping without application-specific conventions.
2. **A separate, minimal view value on the scene.** This composed cleanly with
   the existing CSS matrix, layout/viewBox map, clipping pipeline, and device
   scaling. The value is only a zoom and an offset (12 bytes; 16 with `Option`),
   with no geometry ownership.

The second option is the focused helper: `VectorView`. It is deliberately not a
camera object:

- it stores no scene, geometry, history, or per-zoom copies;
- it has no registry, scheduler, hit-test tree, or retained rasters;
- it supports pan and uniform zoom only, which is the scope of the issue;
- it is opt-in per scene and the renderer already had one affine matrix
  composition point for CSS transforms.

`SvgScene` gained one field for this (`view: Option<VectorView>`); `VectorScene`
gained additive `view`/`set_view`/`with_view` methods. Parsed SVG
scenes keep `view: None` and behave as before.

## Semantics and composition

The view maps authored document coordinates into the scene's viewBox coordinate
space:

```text
view = document * zoom + offset
```

The renderer forms the final matrix as:

```text
CSS/layout/world matrix * viewBox map * view
```

so the view composes with existing CSS transforms, layout sizing, and device
scaling instead of replacing them. A scene without a view (`None`) is the
legacy path and is unchanged.

`VectorView` provides:

| Method | Purpose |
| --- | --- |
| `identity()`, `new(zoom, offset)`, `from_zoom(zoom)` | construct/validate |
| `document_to_view`, `view_to_document` | viewBox-space math |
| `document_to_viewport`, `viewport_to_document` | pointer mapping through a layout |
| `zoomed_about`, `zoomed_about_viewport_point` | cursor-anchored zoom |
| `panned_by_viewport_delta`, `translated_by_view` | panning |

`VectorScene::view()` reads the current value, `set_view` replaces or clears it
in place (O(1)), and `with_view` is the builder form. The application-level
pattern is to keep authored `VectorScene` values in state and apply the view to a
clone in the view function; no geometry is rebuilt.

## Viewport clipping

When a scene has `Some(view)`, the element's layout box is treated as its
viewport and drawing is clipped to that box. Clipping reuses the existing
`ClipState` pipeline (`push_layout_clip`), so it composes with ancestor clips,
CSS transforms, and dirty-region rendering. `None` keeps the legacy behavior
where content outside the layout box may paint; this preserves existing scenes
and legacy SVG scenes.

The example keeps a CSS `overflow: hidden` wrapper as well, demonstrating that
the standard clipping composition still works; the renderer-level viewport clip
is what prevents a panned drawing from leaking when no wrapper is present.

## Invalid transforms, singular inputs, and zoom limits

- `VectorView::new`/`from_zoom` reject non-finite or non-positive zooms and
  non-finite offsets. They never produce a singular transform.
- A finite positive zoom outside the practical limits is clamped to
  `MIN_ZOOM = 1/64` (0.015625) and `MAX_ZOOM = 64`, so repeated zooming stays
  bounded and the cursor anchor remains fixed at a limit.
- `zoomed_about*` rejects non-finite or non-positive factors. Pan deltas and
  translations must be finite.
- Viewport mapping returns `None` for a degenerate layout or viewBox (zero,
  negative, or non-finite size/position); it never divides by a singular view.
  The renderer already skips non-positive layout/viewBox dimensions.
- Because `VectorView` has private validated fields, rendering code cannot
  receive a singular view through the public API; invalid input fails at the
  helper boundary.

## Sharpness, caching, and magnification

The view participates in the matrix passed to tiny-skia path projection and
coverage rasterization, so magnified geometry is re-rasterized from the original
curves. No bitmap is enlarged: cross-checks in `tests/issue_19_pan_zoom.rs`
place magnified pixels at the mathematically expected positions and verify that
changing the view invalidates a cached layer instead of stretching the old raster.

Interaction with the optional pixel-layer cache:

- A cached layer's key includes the render node `Debug` output, and the scene
  `view` is part of that output. Changing the view therefore rebuilds the raster
  at the new magnification; repainting an unchanged view reuses it.
- The existing magnification fallback (`AffineTransform::magnifies`) still
  bypasses promoted surfaces for enlarged CSS transforms.
- Cache entries are bounded by the existing layer-cache byte budget and LRU
  policy. They are rasters, not geometry copies, and setting either limit to
  zero disables them. The measurements below disable pixel-layer caching.

## Invalidation and export stability

- `VectorGeometry::identity()` and the command buffer are unchanged by
  view-only updates; `VectorScene::clone` bumps handles.
- View-only changes change the scene value, so layer-cache keys change and the
  dirty region for the vector leaf remains its layout box (the viewport). The
  dirty-region/incremental paint test matches a full paint exactly.
- Geometry edits still construct replacement `VectorGeometry`, receive a new
  identity, and invalidate the appropriate derived work.
- `VectorScene::to_svg` and `to_svg_path_data` always write authored,
  untransformed geometry and control points. The view is never baked into an
  export.

## Measurements

Harness: `examples/vector_pan_zoom.rs --bench`. It uses the same approach as the
issue #17 harness: release build, a wrapping counting allocator for allocation
calls/requested bytes, whole-process RSS (`/proc/self/status` or `ps`, so peak
RSS is `null` on macOS), and an allocation-free ALU calibration loop to
normalize noisy wall-clock timings.

- Machine: Apple M1 Pro, 16 GiB RAM, macOS `Darwin 27.0.0 arm64`,
  `rustc`/`cargo` nightly as pinned by the toolchain file.
- Scenario: 12 curved/filled/stroked paths in a 960×640 scene, three path
  colours; pixel-layer caching explicitly disabled (both cache limits set to
  zero) and every node leaves `cache_layer` false.
- Five fresh processes, one benchmark run each; medians of the per-run
  per-operation medians are reported, with min..max across the five run medians. Load average was 9.26→9.64 during the
  five-run window (other agents were active), so absolute wall times carry the
  usual validity caveat; allocation counts/bytes and RSS are deterministic.
- Per-operation medians across the 5 runs:

| Phase | Median | Min..max | Allocs/op | Requested bytes/op |
| --- | ---: | ---: | ---: | ---: |
| view-only `set_view` (5000 ops) | ~0.00002 ms | 0.00000..0.00004 ms | 0 | 0 |
| scene rebuild + new node (400 ops) | 0.00025 ms | 0.00025..0.00025 ms | 4 | 1,662 |
| preparation: rebuild + style/layout (200 ops) | 0.00137 ms | 0.00133..0.00142 ms | 16 | 27,886 |
| uncached full paint (8 ops) | 3.26 ms | 3.06..3.63 ms | 2,440 | 1,476,045 (~1.41 MiB) |

The view-only update is effectively free in this environment: no allocation,
no command access. Rebuilding the node and preparing style/layout stay in the
microsecond range; painting dominates as in the earlier static comparison, and
the view does not add a paint phase. Normalizing paint by the calibration loop
gave 0.78–1.09 (median 0.85), consistent with the load noise.

Retained memory during repeated operations (RSS, KiB):

| Window | Median delta | Min..max |
| --- | ---: | ---: |
| view-only updates 1,000→6,000 | +128 | 128..128 |
| view-only updates 6,000→11,000 | 0 | 0..0 |
| full pipeline 100→1,000 frames | +240 | 160..272 |
| full pipeline 1,000→2,000 frames | 0 | 0..0 |

The one-time 128 KiB step includes the harness timing vector and first-touch
allocator pages from the `ps` RSS reader; it does not repeat. The pipeline
plateau behaves the same way: transient paint allocations are reused, and RSS is
identical between the 1,000- and 2,000-frame checkpoints. Nothing grows with the
number of pan/zoom operations, and the view state itself is 12 bytes
(16 as `Option<VectorView>`); `VectorScene` is 56 bytes regardless of command
count.

Raw samples for the recorded run are in
[`reports/issue-19-pan-zoom-runs.jsonl`](./reports/issue-19-pan-zoom-runs.jsonl).
Reproduce after building, with no other builds running:

```sh
cargo build --release -j 2 --example vector_pan_zoom
for i in 1 2 3 4 5; do target/release/examples/vector_pan_zoom --bench; done
```

## Limitations

- The view is translation + uniform zoom only. Rotation, skew, and arbitrary
  affine cameras are out of scope; an application that needs them should use the
  existing CSS transform matrix and accept its composition/precedence rules.
- Pointer→drawing mapping needs the element's layout box. The headless example
  uses a fixed 800×600 layout; in an application whose layout changes, query the
  box (for example `App::find_element_bounds_by_id`) and pass it to the helper.
  The interactive example keeps the drawing at a known origin/size.
- The interactive window path was compiled but could not be exercised without a
  display in this environment. Headless tests cover mapping, clipping, CSS and
  device-scale composition, dirty regions, cache invalidation, repeated updates,
  and export stability.
- Measurements use pixel-layer caching off. Caching changes the cost profile
  (one raster rebuild per distinct view) but not the retained geometry story;
  that interaction is documented above rather than benchmarked here.
- The example's event handler uses the current `with_event_handler` path, which
  requests a structure refresh when it consumes an event. Geometry is still
  shared, but a production editor may prefer scoped/fragment refreshes or the
  frame-aligned drag work from the related input issue.

## Compatibility

`SvgScene` now has a public `view: Option<VectorView>` field. Direct struct
literals need `view: None`; `SvgScene::new`/`with_paint_servers` default it, and
`with_view`/`set_view` are additive. `VectorScene` gained a private `view` field
and additive accessors, so its public constructors and methods are unchanged.
