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

# Suggested implementation order

1. O1 sparse-moving-damage regression investigation  
2. O2 baseline and measurement discipline  
3. O3 fast paths for simple fills and borders  
4. O4 gradient caching and prerasterization  
5. O6 cache stability and eviction  
6. O7 blur and glow pass efficiency  
7. O9 unified scene diff and dirty-region collection  
8. O10 shared subtree bounds for all paint modes  
9. O8 full and incremental paint traversal reduction  
10. O5 text paint layout reuse  
11. O12 hot-screen partitioning with `FragmentApp` where tree-build cost still dominates  
12. O13 + O14 style-system reductions  
13. O15 + O16 runtime cleanup around transitions and font resolution  
14. O17 draw-list or spatial plan only if the earlier work still leaves meaningful paint bottlenecks

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
