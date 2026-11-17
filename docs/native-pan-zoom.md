# Native vector pan and zoom

Keep the `VectorScene` as the authored document, and convert it to a render
scene once. `VectorView` stores only a uniform scale, translation, and zoom
limits. For each input update, change the render scene's `view_box` using the
actual logical size of the vector element. The renderer applies that mapping
before CSS transforms and device scaling. See
[`native_pan_zoom.rs`](../examples/native_pan_zoom.rs) for a complete runnable
example, including simulated drag and wheel input.

`pan` takes a delta in element-local logical pixels. `zoom_at` takes a positive
multiplicative factor and a pointer in the same coordinates; the document point
under the pointer stays fixed, including when the scale reaches a limit.
`to_document` maps a local pointer back to drawing coordinates. If the element
or an ancestor has a CSS transform, first map the pointer through its inverse;
window/device pixels must likewise be converted to logical coordinates. Use the
same content-box dimensions for `view_box` that the renderer uses for layout.
A resize preserves the view's scale and translation.

The constructor requires finite, positive limits containing the identity scale.
Invalid or overflowing pan and zoom inputs return an error without changing the
view. Mapping returns `None` on non-finite results. `view_box` rejects dimensions
that would produce a non-finite or singular renderer mapping. Select practical
limits appropriate to the drawing and coordinate range; very large coordinates
can lose floating-point precision.

Clip the drawing to its viewport with an overflow-hidden ancestor sized to the
vector element. In a low-level renderer, provide the appropriate clip instead.
This matters when paths extend outside the viewport. Dirty clips and transformed
ancestor clips continue to restrict painting. View changes change the render
scene's view box, so normal scene invalidation repaints it. Keep the authored
scene for SVG export: the view does not alter path commands or curve controls.
For geometry edits, create a new path/scene so the renderer sees the change.

The renderer rebuilds temporary paint paths from retained commands as needed;
it does not store a bitmap per zoom level. Optional raster layers can speed
unchanged content but view-box changes invalidate them. Existing layer cache
magnification falls back to drawing vectors sharply. Disable pixel layers when
measuring vector painting (as the example does); see
[SVG rendering and caching](svg-rendering.md) for memory and cache behavior.

### Measurement status

The example prints mean view-update and paint times plus Linux process RSS before
and after repeated updates. For a useful comparison, run it in an otherwise idle
release environment with several frame counts and fresh processes. Exploratory
runs on a shared host before integrating the shared-geometry change measured
100 frames at 0.0001 ms/update and 6.17 ms/paint (RSS 3,004 to 3,076 KiB),
and 1,000 frames at 0.0001 ms/update and 3.42 ms/paint (RSS 3,160 to 3,172
KiB). Other builds were active, so these numbers are not a controlled performance
comparison or proof of bounded process memory. The integrated deterministic test
checks that 1,000 view changes retain the same geometry storage. An uncontended
longer-run memory and paint measurement remains to be performed.
