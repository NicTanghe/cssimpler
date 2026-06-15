# Rust UI Engine Specsheet Part 2
Continuation roadmap for transform, depth, vector graphics, and glass-style effects.

This document continues [specsheet.md](./specsheet.md).

Planning note:

- Part 2 keeps several topics as whole new epics for roadmap clarity.
- Some of these would ideally have been extensions of earlier part 1 epics.
- Where that is true, each epic calls it out explicitly instead of hiding the relationship.

---

# Epic P - 2D transforms and paint-time geometry

## P1. Transform style model
Depends: C2, C4  
Status: planned  

Purpose:
- Represent a controlled 2D transform subset in the core style model without changing Taffy layout rules

Should have been an extension:
- This would ideally have extended C2 and C4 because it is fundamentally new style data plus style resolution

Support:
- `transform-origin`
- `transform` subset:
  - `translateX(...)`
  - `translateY(...)`
  - `translate(...)`
  - `scaleX(...)`
  - `scaleY(...)`
  - `scale(...)`
  - `rotate(...)`
- Deterministic transform-list ordering
- Clear unsupported handling for transform functions outside the subset

Acceptance:
- Supported transform declarations resolve into core style data
- Omitted transform components use predictable defaults
- Unsupported transform functions fail clearly instead of degrading silently

---

## P2. Transform-aware paint and hit testing
Depends: P1, E2, F1, G3  
Status: planned  

Purpose:
- Apply 2D transforms at paint and interaction time while keeping layout boxes axis-aligned and deterministic

Should have been an extension:
- This would ideally have extended E2, F1, and G3 because it changes rendering, hit testing, and invalidation behavior

Support:
- Paint-time transform application for container and text nodes
- Inverse-transform hit testing for supported 2D transforms
- Transformed clipping behavior for supported overflow cases
- Borders, corner radius, gradients, shadows, and text continue to follow transformed geometry

Acceptance:
- Rotated, translated, and scaled nodes paint in the expected screen position
- The deepest transformed interactive target still receives hover and click events
- Paint-only transforms do not require layout recomputation

---

# Epic Q - Composited subtree surfaces

## Q1. Offscreen subtree surfaces
Depends: P2, E3, G3  
Status: planned  

Purpose:
- Raster a subtree into an intermediate surface so later transforms and effects can treat it as one visual unit

Should have been an extension:
- This would ideally have extended E2, E3, and G3 because it is really renderer and invalidation infrastructure

Support:
- Offscreen rasterization for selected subtrees
- Surface bounds that conservatively cover child paint, shadows, and clipping
- Alpha compositing of a cached surface back into the main scene
- Deterministic surface invalidation when source content or effect inputs change

Acceptance:
- A card subtree can be flattened and composited back as one unit
- Surface bounds cover transformed and shadowed content conservatively
- Cached surfaces invalidate when content, clip, transform, or effect inputs change

---

## Q2. Surface-aware dirty regions and reuse
Depends: Q1, N2  
Status: planned  

Purpose:
- Keep composited subtree rendering efficient and compatible with the existing partial rerender model

Should have been an extension:
- This would ideally have extended G3 and N2 because it refines scene reuse and dirty-region behavior

Support:
- Dirty-region expansion for composited subtrees
- Safe reuse of an unchanged offscreen surface across frames
- Conservative invalidation for moving or transformed surfaces

Acceptance:
- Repainting a composited subtree does not force a full-window redraw in common cases
- Reused surfaces stay visually correct across hover and motion updates
- Dirty regions remain deterministic and conservative

---

# Epic R - 3D transforms and perspective

## R1. Perspective and 3D transform subset
Depends: P2, Q1  
Status: planned  

Purpose:
- Support the minimum 3D transform model needed for card tilt, layer lift, and shallow depth effects

Should have been an extension:
- This would ideally have extended Epic P, but it is kept separate so the basic 2D foundation stays independently shippable

Support:
- `perspective`
- `translateZ(...)`
- `rotateX(...)`
- `rotateY(...)`
- `rotateZ(...)`
- Deterministic local projection for supported 3D transforms
- Clear unsupported handling for the rest of the CSS 3D transform surface

Acceptance:
- Nested layers can render at visibly different depths inside a perspective context
- A tilted card projects consistently frame to frame
- Unsupported 3D functions fail clearly instead of producing undefined visuals

---

## R2. Preserve-3d and flattening rules
Depends: R1  
Status: planned  

Purpose:
- Define how nested 3D UI subtrees preserve depth, flatten, and resolve painter order

Should have been an extension:
- This would ideally have extended R1 as a later slice of the same transform stack, but it remains separate for planning clarity

Support:
- `transform-style: preserve-3d`
- Deterministic flattening boundaries
- Stable local depth ordering for siblings in the supported subset
- Projected hit testing for supported 3D cases

Acceptance:
- Front layers render in front without unstable ordering glitches
- Nested 3D containers flatten predictably at explicit boundaries
- Hit testing respects the projected geometry of supported 3D transforms

---

## R3. Full 3D transform function coverage
Depends: R2  
Status: planned  

Purpose:
- Expand the initial 3D subset into a broader real-world CSS 3D transform surface so production examples do not fail on common functions like `scale3d(...)`

Should have been an extension:
- This would ideally have extended R1 as a later completeness pass over the same transform stack, but it remains separate so the minimum 3D milestone stays explicit

Support:
- `translate3d(...)` coverage is completed and documented alongside the existing depth-translation subset
- `scaleZ(...)`
- `scale3d(...)`
- General `rotate3d(...)`, not just axis-aligned special cases
- `matrix3d(...)`
- `perspective(...)` as a transform function in addition to the standalone property
- A documented and deterministic fallback policy for any remaining excluded 3D transform functions

Acceptance:
- The Uiverse hover card no longer fails on `scale3d(...)`
- Supported 3D transform functions resolve into core transform data without ad hoc special-casing
- Remaining unsupported 3D transform functions fail with specific diagnostics instead of silent misrendering

---

# Epic RB - Transform-aware anti-aliasing and reconstruction

## RB1. Transformed text resampling
Depends: H4, R1  
Status: implemented  

Purpose:
- Keep text readable under supported 2D and 3D transforms without requiring full-frame supersampling

Should have been an extension:
- This would ideally have extended H4 and R1 because it refines how already-supported transformed text is sampled at paint time

Support:
- Bilinear or equivalent smooth resampling for transformed text masks
- Grayscale anti-aliasing behavior for transformed text paths
- Deterministic handling for transformed text stroke and shadow masks
- Explicit guidance that small body text should prefer flat presentation when visual quality would otherwise degrade

Acceptance:
- Tilted or rotated labels no longer show nearest-neighbor stair stepping at normal UI sizes
- Transformed text remains visually stable across fractional movement and repeated frames
- Text stroke and shadow effects continue to align with the transformed glyph body

---

## RB2. Coverage AA for transformed shapes
Depends: E3, P2, R1  
Status: planned  

Purpose:
- Replace binary inside-or-outside sampling for transformed boxes, borders, and rounded corners with cleaner edge coverage at CPU-friendly cost

Should have been an extension:
- This would ideally have extended E3, P2, and R1 because it is a paint-quality improvement over the existing transformed shape path

Support:
- Edge coverage AA for transformed filled rectangles and rounded rectangles
- Edge coverage AA for transformed border rings and rounded border rings
- Coverage evaluation that stays localized to edge pixels instead of turning into full-scene supersampling
- Deterministic clipping interaction with transformed overflow and rounded corners

Acceptance:
- Projected card silhouettes no longer show hard binary stair steps along curved edges
- Rounded borders remain visually continuous under rotateX and rotateY
- The AA path does not require a full-frame supersampled render target

---

## RB3. Selective composited AA for transformed layers
Depends: Q1, Q2, RB1, RB2  
Status: probably just a waiste of memmory

Purpose:
- Add a higher-quality fallback for transform-heavy UI like tilted cards by flattening selected subtrees into intermediate surfaces and applying a localized cleanup pass

Should have been an extension:
- This would ideally have extended Q1 and Q2 because it builds directly on offscreen subtree surfaces and their invalidation rules

Support:
- Raster selected transform-heavy subtrees into intermediate surfaces
- Optional higher-resolution surface rasterization for selected layers only
- A localized CMAA-like or equivalent post-process cleanup pass on the composited surface instead of on the full frame
- Conservative memory and invalidation budgeting so the feature stays practical on the CPU renderer

Acceptance:
- A transformed card subtree can render more cleanly without enabling full-frame post-AA
- Small transformed text and curved card edges improve together when the subtree is promoted to a surface
- Surface allocation, reuse, and invalidation stay deterministic under hover and motion updates

---

# Epic S - SVG and vector graphics subset

## S1. Vector DOM and render tree support
Depends: B1, B2, E1, E2, J2  
Status: planned  

Purpose:
- Add a controlled inline SVG subset so icons and logos can be renderer-owned instead of browser-dependent

Should have been an extension:
- This would ideally have extended B2, E1, and J2 because it broadens accepted markup and render-tree node types

Support:
- Inline `<svg>`
- Inline `<g>`
- Inline `<path>`
- `viewBox`
- `width`
- `height`
- Render-tree support for vector content

Acceptance:
- Simple inline SVG icons render on screen
- Unsupported SVG tags and attributes fail clearly
- Vector nodes integrate into the existing render tree deterministically

---

## S2. SVG styling and path paint subset
Depends: S1, C4, K3  
Status: planned  

Purpose:
- Style supported SVG content through the same deterministic CSS pipeline used for HTML-like nodes

Should have been an extension:
- This would ideally have extended C4, K3, and E3 because it is mostly CSS resolution plus paint behavior for a new node class

Support:
- `fill`
- `stroke`
- `stroke-width`
- `currentColor`
- Class and id selector matching on supported SVG elements
- ViewBox mapping for supported path paint

Acceptance:
- Icons can recolor through CSS and `currentColor`
- Filled and stroked path output stays stable at different element sizes
- Supported SVG styling participates in state-aware style resolution deterministically

---

## S3. SVG local paint-server references
Depends: S1, S2  
Status: planned  

Purpose:
- Extend SVG paint from solid colors, `currentColor`, and `none` to local paint-server references shared by both fill and stroke

Should have been an extension:
- This extends S2 because it broadens the supported SVG paint value model while keeping references local and deterministic

Support:
- Inline `<defs>` containers inside supported `<svg>` roots
- Paint-server collection by local `id` before supported shape paint is resolved
- `fill: url(#paint-id)` on supported SVG shapes
- `stroke: url(#paint-id)` on supported SVG shapes
- Shared URL-paint resolution for fill and stroke; stroke URL paint is required, not optional
- Clear rejection for external URLs, missing paint-server ids, duplicate paint-server ids, unsupported paint-server tags, and unsupported fallback paint syntax

Acceptance:
- A supported SVG path can resolve separate local paint-server references for fill and stroke
- `stroke: url(#paint-id)` behaves as first-class SVG paint rather than a fill-only special case
- Unsupported SVG paint-server references fail clearly instead of silently rendering black or dropping paint

---

## S4. SVG linearGradient paint servers
Depends: S3, G3  
Status: implemented

Purpose:
- Render the common inline SVG pattern where supported vector shapes reference local `<linearGradient>` definitions through `fill: url(#id)` or `stroke: url(#id)`

Should have been an extension:
- This extends S3 by adding the first concrete paint-server type and reusing the renderer-owned gradient sampling model

Support:
- `<linearGradient>` paint definitions with local `id` lookup
- `<stop>` children with supported stop colors and offsets
- Supported `x1`, `y1`, `x2`, and `y2` gradient endpoints
- Gradient fills and gradient strokes on supported SVG shapes
- Clear rejection for unsupported gradient units, spread methods, inherited/template gradients, and stop styling outside the supported subset

Acceptance:
- A supported SVG path can render a local `<linearGradient>` fill referenced through `url(#id)`
- A supported SVG path can render a local `<linearGradient>` stroke referenced through `url(#id)`
- Gradient coordinates map deterministically through the same viewBox/layout transform used for solid SVG path paint
- Unsupported SVG gradient references fail clearly instead of silently rendering black or dropping paint

---

# Epic T - Backdrop and glass effects

## T1. Filter-capable backdrop surfaces
Depends: Q1, E3  
Status: planned  

Purpose:
- Sample already-painted content behind a node so supported glass effects can be renderer-owned and deterministic

Should have been an extension:
- This would ideally have extended E3 because it is a visual-effects feature built on renderer infrastructure

Support:
- Backdrop sampling region for a clipped node
- Rounded-corner clipping on the sampled backdrop
- Controlled blur subset over the sampled backdrop
- Deterministic fallback for unsupported backdrop graphs

Acceptance:
- A glass panel can blur content behind it inside its rounded bounds
- Backdrop sampling does not leak outside the supported clip region
- Unsupported backdrop effect graphs fail clearly

---

## T2. CSS backdrop-filter subset
Depends: T1, C4  
Status: planned  

Purpose:
- Parse and apply a controlled `backdrop-filter` subset without opening the door to arbitrary filter pipelines

Should have been an extension:
- This would ideally have extended M2 and C4 because it is mostly filter parsing plus effect application

Support:
- `backdrop-filter: blur(...)`
- `-webkit-backdrop-filter: blur(...)` alias handling
- Clear non-goals for general filter chains

Acceptance:
- Supported backdrop blur syntax resolves into deterministic effect data
- The vendor-prefixed alias behaves consistently with the supported subset
- Unsupported backdrop-filter values fail clearly

---

## T3. CSS `backdrop-occlude` paint barrier
Depends: T1, T2, E3, Q1  
Status: implemented  

Purpose:
- Add a controlled scene paint barrier that lets an element hide all previously painted CSS/UI renderer content behind its own visual box, without cutting, masking, or modifying the app-level acrylic/window backdrop

Should have been an extension:
- This would ideally have extended E3, Q1, and T1 because it is a renderer compositing behavior over already-painted scene content rather than a layout feature or a normal CSS filter

Support:
- `backdrop-occlude: scene`
- `backdrop-occlude: false`
- Default value: `false`
- Occlusion shape derived from the element's used border box and used border radius
- The occlusion operation happens before painting the element's own shadow, background, border, and children
- The `scene` operation clears all previously painted CSS/UI scene content in the current renderer layer
- The operation does not clear or modify the platform/app-level acrylic backdrop behind the renderer
- Deterministic behavior for nested occluding elements based on normal painter order

Paint order:
```text
platform/app acrylic background
↓
previous CSS/UI scene content
↓
backdrop-occlude: scene clear inside this element's rounded border shape
↓
element shadow
↓
element background
↓
element border
↓
element children
```

Non-goals:
- Do not behave like `backdrop-filter`
- Do not blur, sample, brighten, darken, or otherwise filter pixels
- Do not cut holes in the app/window acrylic layer
- Do not affect layout, hit testing, event routing, or scroll bounds
- Do not clear content outside the element's used border shape
- Do not clear the element's own shadow, background, border, or descendants
- Do not target one specific element, selector, layer, or z-level; that is T4

Acceptance:
- A card with `backdrop-occlude: scene` can hide page-level gradients, glows, and decorative CSS effects that were painted earlier behind the card
- The app-level acrylic backdrop remains visible through transparent or translucent parts of the card background
- Rounded corners do not show stale gradient or glow pixels behind the card
- Nested occluding elements produce deterministic output using normal painter order
- CPU renderer behavior is treated as the reference result
- GPU renderer either matches the CPU behavior or reports the feature as unsupported through the backend capability/fallback policy

Implementation notes:
- Treat `backdrop-occlude` as a compositing operation, not as a filter.
- In the CPU renderer, this can be implemented by clearing the current CSS/UI scene buffer inside the element's rounded border mask before drawing that element's own visual primitives.
- In a GPU renderer, this can be implemented with destination-out alpha, a stencil mask, a scene-layer clear pass, or an equivalent compositing strategy.
- The extracted scene should preserve an explicit `BackdropOcclude` or `PaintBarrier` primitive so render backends do not need to infer the operation from style during painting.

---

## T4. Element-targeted `backdrop-occlude`
Depends: T3, E3, Q1  
Status: planned  

Purpose:
- Extend `backdrop-occlude` so an element can hide the previously painted output of one named element or subtree behind its own visual box, while leaving other earlier CSS/UI scene content and the app-level acrylic/window backdrop intact

Should have been an extension:
- This builds on T3, but is kept separate because targeting a specific element requires extracted paint identity, target lookup, and renderer-side compositing that the scene-wide clear does not need

Support:
- `backdrop-occlude: element(#element-id)`
- Target lookup is limited to one element id in the same extracted scene
- The target includes the named element's own paint and descendant paint that was produced before the occluding element in normal painter order
- Occlusion shape is still derived from the occluding element's used border box and used border radius
- The operation happens before painting the occluding element's own shadow, background, border, and children
- If the target element is missing or has not painted before the occluder, the operation is a deterministic no-op
- Extracted paint items preserve enough element path/id metadata for render backends to determine the targeted contribution without selector matching during paint

Non-goals:
- Do not support arbitrary CSS selectors, class selectors, combinators, pseudo-classes, or multiple targets in the first targeted version
- Do not support z-level, layer, or painter-phase targeting in this milestone
- Do not clear future paint from the target element if the target paints after the occluding element
- Do not clear or modify the platform/app-level acrylic backdrop
- Do not affect layout, hit testing, event routing, or scroll bounds for either element

Acceptance:
- A card with `backdrop-occlude: element(#accent-glow)` can hide that earlier accent/glow element behind the card while leaving unrelated earlier scene content visible
- Targeted occlusion respects the occluding element's rounded border shape
- Missing or later-painted targets produce stable output and do not crash
- Nested targeted occluders remain deterministic under normal painter order
- CPU renderer behavior is treated as the reference result
- GPU renderer either matches the CPU behavior or reports the feature as unsupported through the backend capability/fallback policy

Implementation notes:
- Prefer explicit extracted-scene paint identity over repaint-time selector evaluation.
- A straightforward CPU implementation can collect or replay the targeted element's earlier paint contribution into a mask/source buffer, then clear only those pixels inside the occluder's rounded shape.
- If the first implementation cannot isolate the target cheaply, it should report unsupported rather than silently falling back to `scene`.

---

# Epic U - Motion for transforms and surfaces

## U1. Transform and surface transitions
Depends: P2, Q2, R1, T2, G1  
Status: planned  

Purpose:
- Animate transform-driven and surface-driven visual changes inside the explicit render loop

Should have been an extension:
- This would ideally have extended M3 because it is a continuation of transition support for new property families

Support:
- Typed interpolation for supported 2D transforms
- Typed interpolation for supported 3D transform values in the controlled subset
- Transition support for supported surface and backdrop properties
- Deterministic snapping for unsupported animated properties

Acceptance:
- Hover tilt and lift effects animate over time in the explicit frame loop
- Unsupported transition targets snap deterministically
- Paint-only transitions avoid layout recomputation when possible

---

## U2. Transition-aware compositing and invalidation
Depends: U1, Q2  
Status: planned  

Purpose:
- Keep transform and backdrop animation efficient, conservative, and compatible with partial rerender

Should have been an extension:
- This would ideally have extended G3, N2, and M3 because it refines invalidation and reuse for animated scenes

Support:
- Dirty-region updates for animated transformed surfaces
- Correct invalidation of cached surfaces during active animation
- Reuse of static backdrop inputs when the source scene is unchanged

Acceptance:
- Transform and glass animations do not leave stale pixels behind
- Stable backgrounds and static subtrees remain reusable during overlay motion
- Partial rerender remains deterministic while animations are active

---

# Epic V - Projected-scene invalidation and repaint tightening

## V1. Projected dirty-region descent
Depends: R2, U2, G3  
Status: planned  

Purpose:
- Reduce CPU cost for 3D and perspective motion by repainting less of a projected scene while keeping the current analytic raster path

Should have been an extension:
- This would ideally have extended G3, R2, and U2 because it is mostly a smarter invalidation policy for already-supported projected rendering

Support:
- Descend into projected containers when their own visual style is unchanged instead of always invalidating the whole projected branch
- Dirty only the conservative union of previous and current projected bounds for safe transform-only leaf updates
- Conservative fallback to whole-branch invalidation when depth ordering, clipping, perspective inheritance, flattening boundaries, or subtree structure could change the result
- Preserve existing full-redraw parity checks so partial rerender remains deterministic

Acceptance:
- Animating a single tilted child under a perspective parent no longer forces the whole projected ancestor subtree to repaint in common safe cases
- Incremental render remains visually identical to a full redraw for the supported projected cases
- CPU-side 3D motion improves without introducing blur, AA, or text-quality regressions

---

# Epic W - Budgeted subtree surfaces and reconstruction policy

## W1. Subtree-sized offscreen surfaces
Depends: Q2, RB3, V1  
Status: planned  

Purpose:
- Redesign subtree compositing as an explicitly budgeted renderer feature for carefully selected flat subtrees after projected invalidation has already been tightened

Should have been an extension:
- This would ideally have extended Q1, Q2, and RB3 because it is a safer second pass over compositing infrastructure rather than a new rendering model

Support:
- Subtree-sized temporary buffers instead of full-window matte allocation
- Premultiplied-alpha surface storage and reconstruction rules
- A documented edge-sampling policy for promoted surfaces so transparent borders do not darken or otherwise distort nearby blur and AA
- Strict byte-budgeting, eviction, and promotion heuristics that prefer skipping promotion over memory spikes
- Promotion limited to explicitly safe subtree classes until parity and memory behavior are proven

Acceptance:
- Promoting a safe flat subtree no longer scales memory cost with the full window size
- Promoted subtrees match the non-promoted analytic path closely enough that no obvious blur fringe or AA halo is introduced
- Surface promotion remains optional and deterministic under memory pressure

---

# Suggested implementation order (part 2)

1. P1 + P2  
2. Q1 + Q2  
3. S1 + S2  
4. R1  
5. R2  
6. R3  
7. T1 + T2 + T3  
8. T4  
9. U1 + U2  
10. V1  
11. W1  

---

# Outcome

If part 2 lands, the engine should be able to support:

- 2D transform-driven UI without browser involvement
- Controlled 3D card and layer effects with a broader CSS 3D transform surface
- Smoother CPU-side projected 3D motion through tighter invalidation before any compositor-heavy fallback is used
- Renderer-owned inline SVG icons and logos
- Glass-style backdrop blur in a narrow deterministic subset
- Budgeted subtree compositing only where it materially helps and stays visually stable
- Motion that keeps working within the explicit render loop and partial rerender model
