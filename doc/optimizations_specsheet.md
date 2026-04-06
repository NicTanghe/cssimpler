# Optimization Specsheet
Focused backlog for renderer, paint, and runtime performance work, with the primary target being `examples/gui_effect_pressure.rs`.

---

# Goal

Improve frame time in the effect pressure demo first, then lift the same wins into the general renderer and app runtime.

Primary success condition:

- `paint_us` drops materially on `examples/gui_effect_pressure.rs`
- frame spikes from cache churn are reduced
- the engine keeps deterministic output and correctness-first fallbacks

Secondary success condition:

- `render_tree_us`, `scene_prep_us`, and transition overhead improve on larger UI trees
- optimizations stay measurable through the existing runtime stats

---

# Problem statement

The pressure demo is intentionally biased toward the most expensive paint paths:

- layered gradients
- glows and blur-heavy text or shadow effects
- box shadows
- many small effect nodes with a narrow moving band of animated regions

The current engine already has good foundations:

- incremental dirty-region repaint
- parallel full and incremental paint
- shadow-mask caching
- text raster and effect caches
- scoped rerender support in `App` and `FragmentApp`

The remaining cost is concentrated in repeated per-pixel math, repeated scene traversal, expensive cache miss behavior, and some duplicated layout or diff work around paint.

---

# Epic O - Rendering and paint optimization

## O1. Sparse moving damage can be slower than full repaint
Depends: -  
Status: in progress

Purpose:

- Capture and fix the pressure-demo regression where a smaller moving live band can draw slower than an all-live scene

Support:

- compare all-live, narrow moving-band, and pulse-layout cases directly
- track when the renderer stays in incremental mode versus when it falls back to full repaint
- account for active or inactive switching that expands or shrinks glow and shadow bounds
- review whether fragmented dirty regions, dirty-job setup, scene traversal, and scattered copy-back are outweighing the benefit of a smaller changed area

Acceptance

- We can reproduce and explain the case where sparse moving damage is slower than full repaint
- Stats clearly show the chosen paint mode plus the dirty-region and dirty-job shape for this workload
- Heuristics or paint planning are adjusted so this case no longer regresses just because the changed band is smaller

---

## O2. Pressure demo baseline and acceptance harness
Depends: O1  
Status: in progress

Purpose:

- Use the existing runtime stats in the pressure demo as the official optimization baseline
- Keep the work grounded in `paint_us` first, while still tracking `render_tree_us`, `scene_prep_us`, `present_us`, and total frame time

Acceptance

- The pressure demo is the first optimization gate
- Before and after measurements are captured using the existing stat output
- We can compare idle, animated paint-only, and layout-pulsing modes separately

---

## O3. Fast paths for simple opaque fills and borders
Depends: O2  
Status: in progress

Purpose:

- Reduce paint cost for common UI chrome that does not need the fully generic rounded-pixel path

Support:

- solid axis-aligned rectangle fill fast path
- solid border ring fast path when radius is zero
- row-fill style loops for fully opaque spans

Acceptance

- Plain rectangular panels and borders do not call the most expensive rounded-corner test per pixel
- Paint output stays identical to the generic path
- Fast paths are chosen conservatively and fall back when shape or alpha is not simple

---

## O4. Gradient caching and prerasterization
Depends: O2  
Status: planned

Purpose:

- Reduce repeated per-pixel gradient sampling cost in the pressure demo

Support:

- cache rasterized gradient layers when layout, radius, and gradient parameters are stable
- reuse prepared gradient data across frames
- keep correctness when a gradient is clipped, resized, or animated

Acceptance

- Static linear, radial, and conic gradients avoid full resampling every frame
- The pressure demo shows lower `paint_us` when only a subset of tiles changes
- Cache invalidation is deterministic

Notes:

- This is expected to be one of the highest-impact paint wins for `gui_effect_pressure`

---

## O5. Text paint layout reuse
Depends: O2  
Status: planned

Purpose:

- Stop paying for text layout again during paint when layout already computed the wrapped text result

Support:

- carry `TextLayout`, equivalent line metadata, or positioned glyph data into paint
- let text raster caching use that data directly on cache misses

Acceptance

- Text measurement is not repeated on the paint path for the same resolved text block
- Layout and paint continue to agree on line breaks
- Text-heavy screens show lower paint-time cache miss cost

---

## O6. Text and shadow cache stability
Depends: O2  
Status: planned

Purpose:

- Reduce frame spikes caused by coarse cache invalidation and expensive cache key construction

Support:

- replace full-cache clears with bounded eviction
- avoid allocation-heavy cache keys where possible
- reduce lock contention in font and text lookup paths

Acceptance

- Text raster, text effect, and shadow caches do not clear wholesale at capacity
- Cache churn no longer causes obvious paint spikes on varied scenes
- Cache lookups reduce avoidable string allocation and lock pressure

---

## O7. Blur and glow pass efficiency
Depends: O2  
Status: planned

Purpose:

- Improve the expensive glow and text-effect paths that the pressure demo stresses heavily

Support:

- reuse prefix-sum scratch buffers inside blur passes
- avoid repeated allocation inside row and column blur loops
- keep multithreaded blur output identical to the single-threaded result

Acceptance

- Text glow, drop-shadow, and related blur effects allocate less during paint
- Pressure demo animation reduces `paint_us` and frame spikes under effect-heavy load
- Effect output remains pixel-stable

---

## O8. Full and incremental paint traversal reduction
Depends: O2  
Status: planned

Purpose:

- Reduce CPU spent walking the full scene repeatedly during paint

Support:

- avoid having each worker traverse the full scene independently where a shared draw list or spatial partition would be cheaper
- reduce duplicate subtree-culling work in full redraws
- preserve the existing dirty-region correctness model

Acceptance

- Full redraw parallel paint does not require every worker to walk the entire scene when a cheaper plan is available
- Incremental repaint reduces repeated scene traversal for each dirty-job group
- Large-node-count scenes show lower paint-side CPU cost

---

## O9. Unified scene diff and dirty region collection
Depends: O2  
Status: planned

Purpose:

- Remove duplicate recursive walks before paint

Support:

- fuse scene visual comparison and dirty-region collection into one traversal
- preserve the existing correctness rules for full redraw fallback and dirty region collapse

Acceptance

- The renderer does not first perform a full equality walk and then a second dirty-region walk for the same frame
- `paint_us` or `scene_prep_us` improves on animated frames
- Dirty-region output remains deterministic

---

## O10. Shared subtree bounds for full and incremental paint
Depends: O2  
Status: planned

Purpose:

- Reuse subtree visual bounds broadly instead of recomputing them on demand

Support:

- compute subtree visual bounds once per scene where practical
- feed the cached bounds into full redraw culling as well as incremental repaint

Acceptance

- Full redraw culling does not recursively rebuild subtree bounds for every node visit
- Incremental and full paint share the same visual-bounds model
- No visual regressions appear around overflow clipping or shadow bounds

---

## O11. Pressure-demo specific paint shaping
Depends: O3, O4, O6, O7  
Status: planned

Purpose:

- Make the pressure demo a focused proving ground for the renderer's hottest paths

Support:

- prioritize tiles with layered gradients, glows, and box shadows
- keep text relatively low priority there unless measurements say otherwise
- treat animated paint-only mode and pulse-layout mode as separate workloads

Acceptance

- The pressure demo is measurably faster in animated paint-only mode
- The pressure demo is still correct when toggling pulse-layout mode
- The demo remains a reliable stress scene for future regressions

---

# Additional optimization backlog captured from broader review

## O12. Hot-screen partitioning with `FragmentApp`
Depends: O2  
Status: planned

Purpose:

- Use explicit fragment boundaries for screens where tree rebuild cost still matters after paint improvements

Acceptance

- Screens with stable islands can avoid rebuilding unrelated roots
- `FragmentApp` remains the strongest explicit performance contract for hot scenes
- The pressure demo can continue to serve as a comparison point between normal `App` and fragment-based refresh

---

## O13. Style rule indexing
Depends: O2  
Status: planned

Purpose:

- Reduce style matching overhead from linear rule scans on every element

Support:

- index rules by cheap selector anchors such as id, class, tag, and interactive requirements
- keep full selector semantics by filtering indexed candidates through the existing matcher

Acceptance

- Resolved styles do not require scanning every rule in common cases
- Output remains identical to the current matcher
- Large DOM and stylesheet combinations reduce `render_tree_us`

---

## O14. Style resolution pass reduction
Depends: O13  
Status: planned

Purpose:

- Lower style resolution overhead after candidate rules are found

Support:

- reduce temporary allocations around matching rule collection
- reduce duplicate loops when applying declarations
- keep custom property ordering semantics intact

Acceptance

- Style resolution produces the same result with less intermediate allocation and iteration
- `render_tree_us` improves on large trees

---

## O15. Transition sampling cost reduction
Depends: O2  
Status: planned

Purpose:

- Reduce per-frame cloning and subtree sampling overhead during scene transitions

Support:

- avoid cloning whole animated subtrees when a smaller sampled representation is possible
- keep transition correctness and final-scene convergence

Acceptance

- Active transitions allocate less and clone less per frame
- Visual output remains identical
- Transition-heavy scenes show lower CPU overhead

---

## O16. Text and font resolver lock cleanup
Depends: O2  
Status: planned

Purpose:

- Reduce contention in font lookup and text raster preparation

Support:

- avoid taking exclusive access on simple lookups when no mutation is needed
- preserve lazy system-font loading and font registration behavior

Acceptance

- Read-heavy font resolution paths no longer serialize unnecessarily
- Parallel paint or text prep does not contend on avoidable writer locks

---

## O17. Draw-list or spatial paint plan exploration
Depends: O8  
Status: planned

Purpose:

- Explore a more explicit paint plan when scene size grows beyond simple recursive traversal

Non-goal:

- Do not commit to a full retained renderer rewrite before proving the need

Acceptance

- We have a documented direction for draw-list or spatial binning if O8 alone is not enough
- Any adopted plan improves hot-frame traversal cost without weakening correctness

---

## O18. Full-redraw subtree bounds precomputation
Depends: O8, O10  
Status: planned

Purpose:

- Eliminate repeated subtree-bounds rebuilding during parallel full redraw

Support:

- precompute a bounds tree that full redraw workers can reuse instead of recomputing subtree bounds on demand
- target the current `CullMode::Subtree` path where `node_intersects_clip()` can end up rebuilding `subtree_visual_bounds(node)` repeatedly per worker band
- preserve the same overflow, shadow, and clip semantics as the current recursive bounds model

Acceptance

- Parallel full redraw no longer rebuilds subtree bounds repeatedly for the same scene across worker bands
- `paint_us` improves on large scenes that trigger multi-worker full redraw
- Bounds reuse remains correct around clipped subtrees and shadow expansion

Notes:

- Current hotspot reference: `renderer/src/lib.rs:1533`

---

## O19. Bounded LRU eviction for paint caches
Depends: O6  
Status: planned

Purpose:

- Replace abrupt whole-cache flushes with smoother bounded eviction behavior

Support:

- swap the current "clear the whole cache when full" behavior in shadow and text-related caches for LRU-style eviction
- keep cache size bounded and deterministic
- avoid performance cliffs caused by synchronized full-cache churn

Acceptance

- Shadow, text raster, and text effect caches evict incrementally instead of clearing wholesale at capacity
- Varied scenes no longer show obvious frame spikes when caches hit their size limit
- Cache bookkeeping stays cheap enough that eviction policy does not erase the benefit

Notes:

- Current hotspot references: `renderer/src/lib.rs:314`, `renderer/src/fonts.rs:272`

---

## O20. Static gradient layer preraster cache
Depends: O4  
Status: planned

Purpose:

- Avoid repeated per-pixel gradient resampling for stable large surfaces

Support:

- cache rasterized gradient layers for static nodes when layout, radius, and gradient parameters are unchanged
- favor large or frequently repainted gradients first, especially in animated scenes where only neighboring content changes
- preserve correctness when clip, transform, opacity, or gradient inputs change

Acceptance

- Large static gradient-backed surfaces do not resample every pixel every frame
- Animated scenes with stable backgrounds show lower `paint_us`
- Cache invalidation remains deterministic and visually correct

Notes:

- This is the explicit renderer-level spin-out of the broader O4 gradient caching work

---

## O21. Span-based alpha and rounded-shape raster batching
Depends: O3, O7  
Status: planned

Purpose:

- Reduce per-pixel loop overhead in mask-heavy and rounded-shape paint paths

Support:

- add more row-span based filling for alpha masks, rounded rectangles, rings, and related shape fills where coverage can be batched safely
- reduce repeated bounds checks, indexing work, and branch-heavy inner loops in the software rasterizer
- keep conservative fallbacks for shapes that do not admit safe span batching

Acceptance

- Hot mask and rounded-shape loops spend less time in per-pixel bookkeeping
- Output remains identical to the existing generic raster path
- Effect-heavy scenes show lower `paint_us` without regressions at fractional edges

---

## O22. Dirty-region tightening for self-only visual changes
Depends: O1, O9  
Status: planned

Purpose:

- Shrink safe-but-overbroad damage when only a node's own visuals change and its children stay stable

Support:

- refine the current dirty-region rule that can mark the union of previous and current subtree bounds when a node-level visual mismatch is detected
- distinguish self-visual changes from child-subtree changes where possible
- preserve the existing correctness-first fallback when the narrower damage cannot be proven safe

Acceptance

- Nodes whose own visuals changed but whose children did not can repaint a tighter region than the full subtree union
- Dirty-region output remains deterministic
- Incremental repaint does not miss pixels around shadows, borders, or clipped descendants

Notes:

- Current hotspot reference: `renderer/src/lib.rs:2645`

---

## O23. SIMD acceleration for mask blend loops
Depends: O7, O21  
Status: planned

Purpose:

- Accelerate the hottest alpha-mask blend loops once scalar algorithmic wins are in place

Support:

- target text-mask, text-shadow, and shadow-mask blend loops that currently process pixels one-by-one
- use SIMD conservatively behind feature or platform gating where needed
- preserve scalar fallbacks and bit-exact or visually identical output expectations

Acceptance

- Mask blend throughput improves on supported CPUs without changing visual output
- Unsupported targets continue to use the scalar path cleanly
- SIMD complexity stays isolated to a small set of hot loops

---

# Suggested implementation order

1. O1 sparse-moving-damage regression investigation  
2. O2 baseline and measurement discipline  
3. O3 fast paths for simple fills and borders  
4. O4 gradient caching and prerasterization, then O20 static gradient preraster cache  
5. O6 cache stability and eviction, then O19 bounded LRU cache eviction  
6. O7 blur and glow pass efficiency  
7. O21 span-based alpha and rounded-shape raster batching  
8. O23 SIMD mask blend acceleration after the scalar path is tight  
9. O9 unified scene diff and dirty-region collection  
10. O22 dirty-region tightening for self-only visual changes  
11. O10 shared subtree bounds for all paint modes, then O18 full-redraw subtree bounds precomputation  
12. O8 full and incremental paint traversal reduction  
13. O5 text paint layout reuse  
14. O12 hot-screen partitioning with `FragmentApp` where tree-build cost still dominates  
15. O13 + O14 style-system reductions  
16. O15 + O16 runtime cleanup around transitions and font resolution  
17. O17 draw-list or spatial plan only if the earlier work still leaves meaningful paint bottlenecks

---

# Non-goals

- No correctness tradeoff that can produce stale or visually wrong output
- No browser-accuracy chase that expands CSS scope just to optimize it later
- No renderer rewrite before the current architecture's straightforward wins are exhausted

---

# Final note

The pressure demo should remain the main optimization proving ground because it stresses the exact paths that look hottest today:

- per-pixel gradient sampling
- glow and blur effects
- shadow-heavy paint
- repeated incremental repaint on a moving active band

If the engine gets faster there first, the broader UI runtime should benefit naturally from the same work.
