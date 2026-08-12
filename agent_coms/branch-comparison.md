# Branch comparison thread

Agents append findings here while comparing the current branch with `origin/old-fast`.

## `old_fast_diff`: evidence from `origin/old-fast` -> `HEAD` (`59ab60b`)

### Scope and history

- The merge base is `0be233b`; `origin/old-fast` adds only `e024279` (a small error-flow fix in `renderer/src/runtime.rs`). The current side adds about 12,173 lines / removes 996 lines across 53 files, with 3,559 additions in `renderer/src/lib.rs` alone.
- The main post-fork performance-risk commits are `7eef7aa` (axis-aligned AA), `d1e9f11`/`c6bf2a6`/`ab8b81b` (native glass and alpha plane), and `cad361f`/`801fdd7` (CSS stacking and hit order). The old optimization machinery (dirty regions, worker row bands, gradient/text/surface caches) is still present, so the safest work is to repair new hot paths rather than roll back functionality.

### P0: restore span-based axis-aligned AA (very high confidence)

Evidence:

- In `origin/old-fast`, `draw_rounded_rect` and `draw_rounded_ring` use `rounded_rect_row_span` plus span blending/filling. Commit `7eef7aa` changed them to visit every pixel in the bounding box and call `rounded_rect_coverage` / `rounded_ring_coverage` (`renderer/src/shapes.rs:61-85`, `108-143`).
- `axis_aligned_shape_coverage` tests four subpixel points for **every** candidate pixel and another sixteen when partially covered (`renderer/src/shapes.rs:526-556`). It has no immediate radius-zero or known-interior shortcut.
- The same coverage routine now runs inside every axis-aligned linear, radial, and conic gradient pixel (`renderer/src/gradient.rs:974`, `1103`, `1219`; likewise prerasterization at `1027`, `1177`, `1261`). `origin/old-fast` narrowed rows with `rounded_rect_row_span` before sampling.
- A later opaque square fast path (`draw_axis_aligned_opaque_rect`, `renderer/src/shapes.rs:88-105`) helps only solid, fully opaque, zero-radius fills. Translucent panels, rounded panels, selection/caret rectangles, blurred backdrops, rings, and gradients still pay the generic loop. Even radius-zero translucent fills do four geometry/clip samples per pixel.

Preserving quality/functionality:

1. Keep the exact 4x/16x coverage calculation only for the fractional outer edge and rounded-corner edge pixels.
2. Compute per-row interior spans. Bulk-fill opaque interiors and run a simple scalar alpha blend over translucent interiors; sample only the at-most-small edge ranges. Add a direct radius-zero path that handles fractional first/last rows and columns while batching the integer interior.
3. For gradients, use row spans/inside regions to avoid coverage sampling in the interior while retaining the current gradient sample at every painted pixel. The AA result at edges remains unchanged.

This is the most direct old-fast technique to restore first and corresponds to the already-documented but unfinished O21 work.

### P0: stop recursively rebuilding stacking plans during hit testing (very high confidence)

Evidence:

- Paint added a useful `draw_deferred_stacking_contexts` flag: the top stacking-context call collects the deferred list once, while recursive normal-flow calls use `draw_node_contents_in_current_stacking_context` and do not recollect it (`renderer/src/lib.rs:2986-3013`, `3015-3327`).
- Hit testing did not get that guard. Each call to `hit_test_node_for_event` allocates a fresh `Vec`, recursively scans all descendants with `collect_deferred_hit_stacking_contexts`, sorts it, then recursively calls the same function for every normal child (`renderer/src/lib.rs:3453-3537`, `3611-3654`). `hit_test_element_path_node` repeats the same design (`3723-3806`). On a deep tree without any z-index contexts, this revisits descendants once per ancestor (O(N * depth)) and allocates once per visited node.
- `settle_element_interaction` calls element-path hit testing whenever a mouse position exists and can loop up to four times after interaction-driven rerenders (`renderer/src/lib.rs:3927-3964`). Thus this affects ordinary animated/interactive frames, not only clicks.
- Scrollbar wheel/target/hit traversal independently builds path-owning `DeferredScrollbarStackingContext` vectors and sorts them at recursive nodes (`renderer/src/scrollbar.rs:758-864` and callers around `336`, `877`, `929`, `990`), compounding allocation/traversal cost.

Preserving functionality:

- Mirror paint's `*_in_current_stacking_context` / `scan_deferred: bool` structure in event, element-path, wheel, auto-scroll, and scrollbar hit traversal. Collect/sort once per actual stacking context, then traverse normal descendants without rescanning.
- Longer term, compute a per-scene flattened stacking/hit plan once and share it among paint, hit testing, and scroll targeting. Do not revert z-index ordering; regression tests from `cad361f` and `801fdd7` should remain the correctness gate.

### P0/P1 memory: avoid duplicating the scene in `ExtractedScene` (very high confidence)

Evidence:

- Each runtime frame first obtains an owned `Vec<RenderNode>` (`renderer/src/runtime.rs:396`), then `ExtractedScene::from_render_roots` deep-clones the full tree into `roots` (`core/src/extracted_scene.rs:57`). The previous extracted scene is retained for diffing, so the runtime can hold the live scene plus previous cloned roots.
- It also creates one or more `ExtractedPaintItem`s per painted node. Every item owns a cloned path, `VisualStyle` (which includes several vectors), transitions, optional text/text-layout/SVG, ids, and element path (`core/src/extracted_scene.rs:22-39`, `371-388`). A node with background + border + shadows duplicates the same style and path repeatedly.
- Production consumers of `items` are only `requires_native_glass`, `glass_regions`, and `preferred_glass_tint` (`git grep` shows all other `.items` uses are tests). Actual renderer paint/diff uses `roots`. This makes the large paint-item representation almost entirely unused at runtime.

Candidate:

- Make the runtime render/diff directly from owned roots and compute `{requires_native_glass, preferred_tint}` in a lightweight tree walk (or store a compact scene summary). Remove paint-item construction from the runtime path, retaining it only behind an explicit/debug API if externally required.
- Best ownership form: move the captured `scene` into the current retained scene after scrollbar/input mutation rather than deep-cloning it into another wrapper. This preserves all rendering behavior while eliminating style/text/SVG duplication and considerable allocation churn.

### P1: fix quadratic DOM cloning in focused native-input decoration (very high confidence)

Evidence:

- While an input is focused, `NativeTextInputs::decorate_scene` walks every render node (`src/app/input.rs:154-203`). At each node it calls `runtime_world.root_as_node(path.root)` (`:175-177`).
- `root_as_node`/`entity_as_node` recursively reconstructs and deep-clones the complete authored DOM, including tags, ids, classes, attribute maps, styles, text, and child vectors (`core/src/runtime_ecs.rs:197-222`). Doing that once per render node is O(N^2) work/temporary allocation on the focused-input path.

Candidate:

- Resolve the single focused input once: obtain its root/path/state and selection style before walking, then mutate only the matching `RenderNode` (or perform one tree walk with borrowed ECS entity/path lookup). Avoid reconstructing any authored root merely to decorate other nodes.

### P1: remove recursive child-subtree clones in ECS patching (high confidence)

Evidence:

- `RuntimeWorld::patch_node` already receives `&Node`, but for every element it clones `element.children` into `authored_children`, then recurses over that clone (`core/src/runtime_ecs.rs:309-364`, especially `:336`). Since each recursive child repeats this, a deep chain clones successively smaller subtrees (quadratic copied data).
- It separately clones `data.children` and replaces `data.authored` via `authored_from_node`, cloning authored strings/maps/style (`:324`, `:339`, `:452-460`). Some copying is required for retained ownership, but cloning the complete child `Node` subtrees is not.

Candidate:

- Borrow `element.children` while patching (or split mutable entity update from recursive traversal so the borrow ends), and only clone the node's own retained authored fields. Reuse/temporarily take the entity-child-id vector instead of cloning where safe.

### P1: native-glass alpha memory and full-redraw costs (high confidence, feature/platform scoped)

Evidence:

- Native glass adds a persistent viewport `Vec<u8>` alpha plane in `RuntimeApp` (`renderer/src/runtime.rs:67-70`, resized at `557-570`). It intentionally forces `render_to_buffer_internal_with_alpha`, bypassing incremental scene updates whenever native glass is active (`:578-604`).
- Parallel alpha redraw retains color and alpha worker bands together. The commit message for `ab8b81b` explicitly notes it "roughly doubles peak worker-band RAM usage". Current retained pool budgets are 32 MiB color plus a new 8 MiB alpha (`renderer/src/lib.rs:93-94`).
- On macOS, each glass presentation creates both content and tint images (`renderer/src/native_glass.rs:457-459`); each `create_alpha_image` allocates a full `Vec<u32>` and boxes it (`:481-566`). That is two transient 4-byte-per-pixel buffers per frame: about 15.8 MiB at 1080p or 63.3 MiB at 4K, in addition to the main 4-byte color and 1-byte alpha planes.

Candidates:

- Keep alpha only for glass-capable/active windows (already true), but enable alpha-aware incremental repaint instead of unconditional full repaint. Clear/update alpha only in dirty regions and present appropriate damage where the platform permits.
- Reuse persistent macOS presenter buffers/images or build one packed representation and avoid generating two full CPU images every frame. Prefer a single mask/tint layer where Core Animation composition can express the same result.
- Consider trimming/dropping the alpha worker pool after glass deactivation or large-window spikes instead of retaining its full 8 MiB budget indefinitely.

### P1/P2: remove TLS alpha checks from every normal paint pixel (high confidence)

Evidence:

- Commit `c6bf2a6` added thread-local `RENDER_ALPHA_TARGET` and routes common blend/fill operations through `current_alpha_at`, `set_current_alpha_at`, and `fill_current_alpha_span` (`renderer/src/lib.rs:817-876`). These helpers check TLS/`Option` even when no native glass alpha plane is active.
- Common normal-window opaque writes now call `set_current_alpha_at`; partially transparent blends call `current_alpha_at` then branch to the old RGB behavior (`renderer/src/lib.rs:6200-6365`, `6520-6577`; shapes/gradients/backdrop also call the helpers).

Candidate:

- Specialize paint at a higher level into opaque-output versus color+alpha targets (generic target trait, two monomorphized functions, or once-per-row branching). The normal path should compile to the old direct RGB math without TLS lookup per pixel; the alpha path retains exact glass behavior.

### P2: smaller allocation wins

- Current extraction's stacking walk constructs/clones `Vec<usize>` paths repeatedly and builds deferred vectors per stacking context (`core/src/extracted_scene.rs:49-57`, `86-181`). Removing unused runtime paint items obviates most of this.
- `collapse_whitespace` now allocates an intermediate `Vec<&str>` and a joined `String` (`core/src/fonts.rs:788-790`); normal wrapping then repeatedly allocates `format!("{current} {word}")` candidates (`:683`). Old-fast already had candidate formatting, but the extra normalization copy is new. A single-pass whitespace iterator/buffer preserves CSS behavior with fewer allocations.
- `redraw_auto_scroll_indicator_regions` always rebuilds a full cached subtree-bounds tree even when both old/new indicator are `None` (`renderer/src/lib.rs:2265-2315`); it is called after every painted frame. Return immediately when both are `None`, and ideally reuse the bounds already computed during paint/diff.

### Suggested implementation/measurement order

1. Baseline current and old-fast with the same headless scenes: opaque square fills, translucent square fills, rounded fills, each gradient type, deep nested hit testing, focused input, and native glass where platform-supported. Record median/p95, allocations, retained/peak bytes, and output checksum.
2. Implement span/interior AA fast paths; pixel-compare against current output, then measure.
3. Fix focused-input O(N^2) cloning and stacking-plan rescans; retain all z-index/input tests.
4. Remove/compact unused extracted paint items and improve scene ownership; measure `scene_prep_us` plus allocations/RSS.
5. Specialize non-glass blending and make native-glass alpha incremental/reusable.
6. Address ECS patch cloning and small allocation/reuse opportunities.

No source-code edits were made by this agent.

## `old_fast_diff`: review of the in-progress AA/span and direct-band optimization

Review result: no output-equivalence defect was found in the new full-coverage shortcut. `rounded_rect_full_coverage_row_span` only labels pixels for which the existing 4x/16x sampler returns exactly `255`, so substituting `255` is byte-equivalent provided the full horizontal interval is contiguous (which follows from the intersection of an axis-aligned clip and rounded-rectangle row). The renderer library suite passes all 187 tests, including serial/parallel color and alpha comparisons.

Concrete remaining hazards/opportunities:

- `rounded_rect_full_coverage_row_span` samples every non-full pixel while searching (`renderer/src/shapes.rs:466`), then the shape and all direct/preraster gradient loops sample those same edge pixels again (`shapes.rs:78-145`; `gradient.rs:970-1338`). A row with no `255` interior (thin/fractional rectangles, pill/circle tips, or a subpixel-height clip) scans its complete width twice. Fuse discovery with edge painting/cache the discovered coverages, or derive a conservative interior span geometrically; benchmark 1 px and subpixel hairlines as well as pills and many tiny rounded nodes.
- The new rounded-rect interior still writes one pixel at a time. For `color.a == 255`, the detected full span can use `buffer[start..end].fill(prepared_color.packed)` plus one `fill_current_alpha_span`; this is exactly equivalent and should materially help opaque rounded panels, which do not qualify for the existing zero-radius opaque-rect fast path.
- Direct destination banding (`renderer/src/lib.rs:1593-1705`) preserves the old band-local coordinate system and removes scratch allocation/copy-back. The current odd-size test covers color only, while the alpha equivalence test uses evenly sized dimensions. Add an odd width/height alpha/native-glass test with transparent holes and rounded/gradient content crossing every worker seam; also cover `worker_count > height` and assert color/alpha slices have the expected `width * height` length so release builds cannot silently truncate a `zip` if internal lengths diverge.
- Add byte-for-byte reference tests for optimized rounded fills and linear/radial/conic gradients over randomized fractional layouts, clips, asymmetric radii, representative source/destination alpha, both no-alpha and alpha targets, and nonzero worker row offsets. This directly guards AA, blend quantization, and band-local indexing rather than checking only a few visible pixels.

Verification run: `cargo test -p cssimpler-renderer --lib` passed 187/187; `cargo test -p cssimpler-core --lib owned_extraction_preserves_roots_and_paint_items` passed.
