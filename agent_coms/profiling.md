# Profiling thread

Agents append current-code hot spots, allocation findings, and benchmark measurements here.

## 2026-08-12 — current-branch CPU/RAM hot-path audit

This is a static audit, not measured profiling yet. `N` means render/authored nodes, `D` tree depth, `L` text length, and `A` affected pixels. Line numbers are current-checkout anchors; function names are the durable references.

### Prioritized findings

#### P0 — eliminate whole-scene copies in the draw path

**Evidence**

- `renderer/src/lib.rs:1020-1031`, `SceneProvider::capture_scene`: the default implementation calls `self.scene().to_vec()`. `App` and `FragmentApp` use that default, so every capture deeply clones the `RenderNode` forest even if the scene is unchanged or the frame is later rejected by `should_present_scene`.
- `renderer/src/runtime.rs:390-397`, `render_frame`, and `:525`, the event rerender path, capture after update and can therefore pay this copy more than once around a rendered event.
- `core/src/extracted_scene.rs:47-59`, `ExtractedScene::from_render_roots`, traverses the captured roots to create items and then performs another `roots.to_vec()` deep clone.
- `core/src/extracted_scene.rs:355-394`, `push_item`, clones the path, transform, full `VisualStyle`, transitions, text layout, and identifiers for every emitted paint phase. One node can emit several items. The final `items.sort_by_key` is redundant because `next_stable_sort_key` is monotonically assigned during traversal.
- Production renderer painting consumes `ExtractedScene::roots`, while `items` is used internally only for glass queries (`requires_native_glass`, `glass_regions`, and `preferred_glass_tint`). Public API compatibility means items cannot simply disappear, but they need not be eagerly materialized as owned copies.
- `renderer/src/runtime.rs:95,697` retains the previous `ExtractedScene`; the `App` also retains its render cache. During capture/extraction, several complete forests coexist.

**Candidate**

- Give scenes a revision and immutable shared storage such as `Arc<[RenderNode]>`; return the same snapshot for an unchanged revision.
- Move or share roots through extraction instead of `to_vec`; use copy-on-write only for the sparse scrollbar decoration mutation.
- Represent extracted items by root/path indices or lazily materialize the owned public view. Compute the three native-glass summaries directly during traversal.
- Remove the stable-key sort unless later traversal stops emitting in stable order.

**Measurable target:** after warmup, an unchanged pointer/animation redraw should perform zero deep scene clones and ideally zero scene-preparation allocations. A changed frame should retain at most one new owned scene forest. Track `scene_prep_us`, allocation count/bytes, `RenderNode` clone count, and peak live bytes for 100/1,000/10,000-node scenes.

#### P0 — remove the `Node -> RuntimeWorld -> Node` round trip and quadratic patch clones

**Evidence**

- `src/app.rs:302-340`, `render_root_with_schedule`, calls `RuntimeWorld::root_as_node` before resolving styles.
- `core/src/runtime_ecs.rs:197-223`, `root_as_node` / `entity_as_node`, recursively clones tag/id/classes/attributes, full `Style`, text, and children to reconstruct the authored tree.
- The application already built a complete `Node` tree in `src/app.rs:527-550` (`view`, materialization, and synchronization), so the ECS copy is immediately copied back out.
- `core/src/runtime_ecs.rs:309-365`, `patch_node`, first materializes `authored_from_node`, then clones `element.children` at each recursive level (`:336`) and clones entity children (`:339`). Cloning a subtree at every ancestor is `O(N*D)` and `O(N^2)` for a chain. `node_matches_shape` (`:287-307`) also performs a full prewalk before patching.
- `core/src/runtime_ecs.rs:450-461`, `authored_from_node`, clones all authored strings and style data per node.
- `RuntimeComputedNode` (`core/src/runtime_ecs.rs:99-107`) contains an inline `Option<Style>` plus layout/scroll state. The resolved-style and computed-layout fields are currently initialized/reset but not populated or consumed outside this module, so every entity reserves substantial unused inline capacity.
- `RuntimeWorld::clear_dirty_flags` (`:191-195`) scans every entity after refresh, even if very few entities were dirty. Entity/free-list vectors retain their high-water capacities.

**Candidate**

- Immediate low-risk change: recurse over borrowed `node.children`, clone only the entity-ID list required to satisfy borrowing, skip the separate shape prewalk, and avoid replacing identical authored fields.
- Larger change: resolve directly from `RuntimeWorld`, or retain the original `Node` for style resolution, instead of reconstructing a second tree.
- Remove/sparsify unused computed payloads and maintain a dirty-entity list for flag clearing.

**Measurable target:** synchronization is `O(N)` with `O(D)` scratch and no descendant-subtree clones; a full refresh has no more than two complete tree representations alive; dirty clearing is `O(dirty)`. Benchmark a wide 10k-node tree and a 2k-deep chain separately.

#### P0 — stop full-tree cloning throughout style and layout

**Evidence**

- `style/src/render_tree.rs:122-139`, `layout_resolved_render_tree_in_viewport`, deeply clones the complete `ResolvedElement` tree merely to apply root viewport stretching (`:131`).
- `build_layout_tree` (`:428-469`) copies text, `Style`, `SvgScene`, and path data into another tree; leaf context creation (`:446-450`) clones text and `TextStyle` again.
- Render extraction (`:703-774`) clones SVG scenes, text, `VisualStyle`, transitions, and paths into `RenderNode`. The cached-layout rebuild (`:881-967`) still rebuilds the whole render forest and repeats these copies.
- `resolve_element_tree` (`:253-400`) allocates/copies an ancestor vector per node (`:300-302`). `ElementPath::with_child` (`core/src/interaction.rs:15-21`) clones its vector. Selector candidate collection/sorting allocates per element in `style/src/lib.rs:142-172,224-272`.
- Custom-property inheritance clones every property name/value down descendants (`core/src/custom_properties.rs`, `inherit_from`; called from `style/src/lib.rs:697-699`).
- `TextStyle::default` (`core/src/fonts.rs:129-143`) creates a heap `Vec` containing one generic family. Default `VisualStyle` objects are constructed in `RenderNode::{container,text,svg}` (`core/src/lib.rs:850-899`) and then commonly overwritten by the final style, causing at least one avoidable small allocation per node.

**Candidate**

- Apply root layout constraints without cloning the resolved tree. Transfer ownership between one-shot stages or share immutable style/text/SVG payloads with `Arc`.
- Persist layout nodes and update dirty subtrees rather than rebuilding Taffy and `RenderNode` trees for cached-layout refreshes.
- Reuse a push/pop ancestor/path stack and selector scratch storage; iterate small candidate sets without allocating/sorting when possible.
- Store the common single font family inline (`SmallVec` or a dedicated primary-family field), and make constructors accept the final style.

**Measurable target:** no full `ResolvedElement` clone, and 70-90% fewer allocations on 1,000-node full and cached-layout refresh benchmarks. Cached-layout work should scale with the dirty subtree, not total scene size.

#### P0 — focused native text input currently turns decoration into `O(N^2)` copying

**Evidence**

- `src/app/input.rs:154-203`, `NativeTextInputs::decorate_scene` / `decorate_node`: while any input is focused, every `RenderNode` is visited; for each node with a path, `runtime_world.root_as_node(path.root)` (`:176`) deeply clones the entire authored root before checking whether that node is the target input.
- `focus_input_at_pointer` (`:219`) and input dispatch (`:398`) independently clone a whole root too.
- `caret_index_from_pointer` (`:586-613`) lays out the complete text and then every character prefix. Together with repeated prefix allocation this is `O(L^2)` during pointer movement. `char_boundaries` (`:615-621`) allocates another vector.
- `NativeTextInputs::states` (`:36`) is populated during materialization/focus but there is no removal-generation sweep for vanished/renamed inputs, allowing dynamic keys to accumulate indefinitely.

**Candidate**

- Add a `RuntimeWorld` entity accessor keyed by `ElementPath`, or maintain a path-to-entity index, and decorate only the focused render node by direct path lookup.
- Cache the prepared text layout/glyph advances and binary-search a caret position.
- Mark live input keys during materialization and prune old generations; optionally keep a small bounded history for focus restoration.

**Measurable target:** focused-input decoration is `O(D)` and creates no DOM clones; caret hit testing is `O(L)` preprocessing plus `O(log L)` per query; retained state count never exceeds live input count plus a documented bound.

#### P0 — promoted transformed-surface cache does expensive work even on a hit

**Evidence**

- `renderer/src/lib.rs:580-680`, `cached_promoted_surface`, begins by calling `neutralized_surface_root`; that helper (`:418-424`) deeply clones the render subtree.
- `hash_surface_subtree` (`:426-449`) creates multiple `format!("{:?}", ...)` strings per node for kind, style, text edit, inset, and scrollbars on every lookup.
- On a miss, `translated_render_subtree` (`:569-577`) calls `node.clone()` at each recursion level and then replaces children with a recursively cloned list. That repeats descendant cloning, becoming `O(N*D)`/`O(N^2)` for a chain.
- This is reached for eligible transformed flat containers from `draw_node_transformed_internal_impl` (`:4080-4116`) on every paint, including transform animations. A hit avoids rasterization but not neutralizing-clone and debug-string hashing.
- Surface reconstruction renders the subtree over black and white (`:629-665`), doubling rendering work.

**Candidate**

- Compute/store a structural content revision/hash when the scene changes and combine it with dimensions; hash fields directly while ignoring only the root transform.
- Pass an origin/translation into rendering rather than producing translated tree clones.
- Explore one-pass premultiplied RGBA+alpha surface construction.

**Measurable target:** promoted-cache hits are `O(1)` (or at worst `O(N)` field hashing) with zero heap allocation; misses are `O(N)`, not `O(N*D)`. Measure transformed 100/1,000-node static and animated subtrees, including hit rate and bytes allocated.

#### P0 — cache entry-count limits do not bound RAM; one metadata cache is unbounded

**Evidence**

- Shadow masks (`renderer/src/shadow.rs`, `MAX_SHADOW_MASK_CACHE_ENTRIES`, `ShadowMaskCache`) are capped at 256 entries but each entry owns `width * height` bytes. There is no byte or single-entry cap. In the theoretical worst case, 256 1080p masks occupy about 506 MiB before transient blur storage.
- Text raster/effect caches (`renderer/src/fonts.rs`, `MAX_TEXT_RASTER_CACHE_ENTRIES = 256`, `MAX_TEXT_EFFECT_CACHE_ENTRIES = 512`, `TextRasterCaches`) are also count-only and each `AlphaMask` owns `width * height` bytes. They are process-global.
- `cached_text_mask` (`renderer/src/fonts.rs:410-454`) creates an owned `Arc<str>` text copy plus owned family strings/vectors through `TextRasterCacheKey::new` (`:1061-1115`) before it knows whether the lookup is a hit.
- `GradientLayerCache::seen_counts` (`renderer/src/gradient.rs:152-157,527-552`) is never aged/evicted outside tests. Unique animated layout/gradient keys grow metadata without bound. When the layer cache reaches capacity (`:568-572`), new keys return `None` rather than replacing cold entries, making those keys permanent rasterization misses.

**Candidate**

- Use a unified byte-budgeted LRU with per-entry limits and expose configured/current/peak bytes. Keep entry caps as secondary protections.
- Bound or periodically age the gradient admission map; perform real LRU replacement.
- Support borrowed key lookup or stable text/style content IDs so cache hits do not first allocate owned keys.

**Measurable target:** cache RSS is deterministic (for example, a configurable 16-64 MiB global budget), adversarial unique-key tests reach a steady entry/metadata count, and warmed cache hits allocate zero key memory.

#### P1 — renderer repeatedly traverses and allocates complete bounds trees

**Evidence**

- `should_present_scene` (`renderer/src/lib.rs:2239-2253`) recursively compares the whole scene.
- If changed, `prepare_scene_diff` (`:5490-5505`) allocates cached bounds for both previous and current roots and then walks them again for dirty-region generation. `scene_max_backdrop_blur_radius` (`:6047-6058`) scans both scenes again.
- `redraw_auto_scroll_indicator_regions` (`:2265-2315`) is invoked unconditionally after paint from `renderer/src/runtime.rs:625-640`. It calls `cache_scene_subtree_bounds(scene)` even when both indicator arguments are `None`, guaranteeing an otherwise unnecessary full traversal/allocation tree on ordinary frames.
- `visual_styles_match_ignoring_projection` (`renderer/src/lib.rs:2352-2357`) clones both complete `VisualStyle` values, including heap vectors, only to normalize three fields before comparison.

**Candidate**

- Immediately return from indicator redraw when both indicators are absent; otherwise reuse current bounds already generated for the diff.
- Retain bounds/revisions with the previous scene and merge `should_present` determination into diff preparation.
- Compare style fields directly while skipping projection fields, without cloning.

**Measurable target:** zero indicator work without indicators; at most one current-tree bounds build and no previous-tree rebuild per changed frame; no style allocations during diff. Add counters for bounds nodes built and scene traversals/frame.

#### P1 — parallel painting duplicates full-frame buffers, and presentation ignores damage

**Evidence**

- `renderer/src/lib.rs:1648-1788`, `render_to_buffer_parallel`, paints into worker bands totalling approximately one full `u32` frame and copies every band into the main buffer (`:1768-1772`). Alpha bands are copied separately (`:1774-1782`). Worker-pool caps are 32 MiB color plus 8 MiB alpha (`:82-94,337-350`).
- The runtime also retains its main color and alpha buffers (`renderer/src/runtime.rs:116-119`). Resize helpers (`renderer/src/lib.rs:6145-6163` and the alpha equivalent) do not shrink capacity after a large window is downsized.
- `copy_render_buffer_into_surface` (`renderer/src/runtime.rs:1021-1072`) converts/dithers every surface pixel, even when the renderer repainted only a small damage region; `draw_frame` then presents the full surface (`:661-684`).
- A 1920x1080 `u32` frame is 7.91 MiB and its byte alpha plane is 1.98 MiB. At 60 Hz, just the worker-to-main color copy moves roughly 475 MiB/s, excluding surface conversion and alpha.

**Candidate**

- Give workers disjoint mutable row slices of the main buffer or retain stable bands without a second full copy.
- Convert only damage into a persistent surface and use damage-aware presentation when supported.
- Add shrink hysteresis for buffers/caches after sustained downsizing.

**Measurable target:** eliminate at least one full-frame copy (~7.91 MiB/frame at 1080p); presentation conversion scales with damaged pixels; peak retained framebuffer memory drops after a large-to-small resize.

#### P1 — backdrop blur has very high transient memory and disables parallel painting

**Evidence**

- `renderer/src/backdrop.rs:131-230`, `blurred_snapshot` / `box_blur`, creates a full `Vec<LinearRgba>` source, then full horizontal and vertical arrays plus prefix scratch. The three 16-byte/pixel planes imply roughly 48 bytes/pixel peak: about 94.9 MiB at 1920x1080 and 379.7 MiB at 4K for a fullscreen blur, before allocator overhead.
- `renderer/src/lib.rs:1570-1578` forces serial scene painting whenever backdrop blur is present.

**Candidate**

- Pool blur scratch, operate directly from packed source where possible, and implement a two-plane or tiled separable pass with small line-prefix scratch.
- Reuse snapshots/results for overlapping glass regions/radii and investigate a dependency-aware parallel path.

**Measurable target:** at most two full linear-color planes (preferably one plus tile scratch), zero allocator calls after warmup, and documented 1080p/4K peak RSS and `paint_us` for 1/4/16 overlapping blur nodes.

#### P1 — scene-transition creation builds the same plan three times and retains three forests

**Evidence**

- `src/app.rs:694-704`, `replace_scene`, calls `SceneTransition::should_create` and then `new`.
- `SceneTransition::should_create` (`src/scene_transition.rs:42-49`) calls `max_scene_transition_duration`; that function (`:113-115`) constructs and drops a full transition plan.
- `new` (`:51-64`) calls `should_create` again and then builds the plan again, so a successful transition builds it three times. Even no-transition structure matches build and discard a plan on each replacement.
- `sample` (`:67-70`) deeply clones `to`. `SceneTransition` retains `from` and `to`, while `App` stores the sampled forest, so an active transition can retain three complete scene forests plus the plan.
- `sample_render_node_in_place` (`:143-200`) repeatedly assigns cloned transform vectors/text-edit values; transform interpolation (`:478-519`) creates a new vector each sample.

**Candidate**

- Replace the check/new pair with `try_new(from, to)`, which builds one sparse plan and returns `None` when duration is zero.
- Keep one mutable/sample scene plus compact changed-property endpoints instead of full `from`, `to`, and sample forests.
- Reuse transform storage after transition start.

**Measurable target:** one plan traversal/allocation pass; active storage near one forest plus sparse endpoints; zero per-frame heap allocation after a transition is initialized.

#### P1 — text wrapping, layout cloning, caret hit testing, and raster effects churn

**Evidence**

- `core/src/fonts.rs:489-522`, `layout_text_block`, always allocates transformed text; `TextTransform::None` still calls `to_string` (`:813-819`).
- `wrap_source_line` (`:651-709`), preserved wrapping (`:711-751`), and `wrap_long_word` (`:753-785`) repeatedly build formatted candidates and remeasure their complete prefix per word/grapheme, producing `O(L^2)` behavior. `collapse_whitespace` (`:788-790`) collects a vector and joins it.
- `style/src/render_tree.rs:1043-1058`, `cached_text_layout`, clones `PreparedTextLayout` on hit and stores another clone. The layout owns `Vec<String>` (`core/src/fonts.rs:162-179`) and is copied again through render extraction/capture.
- `renderer/src/fonts.rs:410-454`, `cached_text_mask`, clones prepared layout on raster miss. Rasterization builds a glyph vector (`:555-590`) and outlines every glyph once for bounds and again for drawing (`:381-405`). Effect blurs allocate full masks for horizontal/vertical passes and worker chunks (`:798-972`).

**Candidate**

- Store prepared layouts in `Arc`; use `Cow<str>` for no-op transforms/whitespace modes.
- Shape/measure once into cumulative advances and wrap by indices, yielding linear behavior.
- Retain glyph outlines/bounds from one pass and pool effect-mask scratch.

**Measurable target:** linear wrapping on 100/1k/10k-character lines, zero deep layout clone on cache hit, and warmed text raster/effect hits allocate nothing.

#### P1 — hit testing and scrollbars repeatedly rebuild traversal state

**Evidence**

- `renderer/src/lib.rs:3611-3653`, deferred-child collection, is invoked recursively from `hit_test_element_path_node` (`:3723-3805`) at normal nodes. This repeatedly scans descendant stacking contexts and allocates deferred lists, producing `O(N*D)` and `O(N^2)` on a deep tree. Transformed hit testing (`:3867-3924`) additionally builds projected-child vectors.
- `settle_element_interaction` (`:3927-3965`) can repeat capture/hit-test/update up to four times for one pointer event.
- `renderer/src/scrollbar.rs:96-119`, `apply_to_scene`, allocates a `HashSet` and traverses the full scene on every captured frame even when there are no scrollbars. Pointer handling can call `find_scrollbar_hit` twice (`:264-284`).
- `RuntimeWorld::set_interaction` (`core/src/runtime_ecs.rs:160-163`) immediately performs a full interaction-component refresh. `patch_node` recalculates interaction per entity, and `src/app.rs:296-300` then unconditionally synchronizes interaction again, allowing up to three whole-world updates for one state change.

**Candidate**

- Precompute flattened stacking/hit order and scrollbar/path indices once per scene revision; use reusable path/scratch stacks.
- Reuse a single hit result across hover, event dispatch, and scrollbar handling; skip scrollbar work based on scene metadata.
- Update only the symmetric-difference ancestor paths when interaction changes, or fold interaction into one patch pass.

**Measurable target:** one `O(N)` index build per changed revision, near-`O(D)` point queries with no recurring heap allocation, one interaction pass per event, and zero scrollbar traversal for scrollbar-free scenes.

#### P1 — SVG painting scales as pixels times path segments

**Evidence**

- `SvgScene` is a deeply owned vector hierarchy (`core/src/svg.rs:86-217`) and is copied in authored, resolved, layout, render, and extracted-scene representations.
- `renderer/src/svg.rs:89-212`, `draw_svg_scene_with_matrix`, iterates each path's bounding-box pixels and takes four coverage samples/pixel (`:145-175`). Each sample runs fill/stroke tests that scan contour segments (`point_in_svg_fill`, `:334-370`; stroke distance, `:372-408`): approximately `O(paths * bbox_pixels * samples * segments)`.
- Paint-source preparation builds resolved gradient stops/segments per path/draw (`:214-280`), and parallel bands can repeat preparation.

**Candidate**

- Share immutable SVG geometry and prepared paint servers with `Arc`.
- Tessellate/scan-convert once or cache raster/coverage masks by scene revision, size, and quantized transform; prepare gradient interpolation tables once.

**Measurable target:** a static SVG cache hit performs no geometry cloning or per-pixel segment scans. Benchmark 100-path and 10k-segment scenes over several output sizes.

#### P2 — fragment refresh still performs repeated full-tree searches

**Evidence**

- `src/app.rs:577-679`, `refresh_fragments`, rebuilds the full view and, for each fragment ID, calls full-tree boundary searches (`find_unique_node_boundary` / `find_unique_render_boundary`, `:1294-1349`) followed by another full-tree mutable search (`find_render_node_mut`, `:1359-1371`). This is `O(kN)` for `k` fragments before style/layout work.

**Candidate and target:** maintain stable ID/path/entity indices and mutate by direct path. Aim for `O(N + kD)` synchronization and avoid rebuilding the complete view when a fragment-specific view is available.

#### P2 — small recurring stats allocation

**Evidence**

- `RuntimeStats.phase_order` (`src/app.rs:33-79`) is a `Vec` pushed on each refresh. `advance` (`:476-490`) clones the stats payload before storing the original globally.

**Candidate and target:** use a fixed array/`SmallVec` and reuse storage; the phase-timing path should allocate zero bytes per refresh.

### Benchmark and instrumentation gates

Before/after comparisons should use identical scenes and collect median/p95 update, scene-prep, paint, present, allocation count/bytes, and peak RSS. Existing timing stats are useful but need allocation/cache counters to explain regressions.

1. **Idle/static:** 100, 1k, and 10k nodes; 600 pointer redraws without style/layout changes. Gate: unchanged scene prep has zero deep clone and zero allocation after warmup.
2. **Full refresh:** wide 10k tree and 2k-deep chain. Gate: time/allocation growth stays linear and the chain does not expose descendant-subtree cloning.
3. **Cached-layout/fragment:** mutate 1 of 1k nodes and 10 of 10k. Gate: work is proportional to changed subtree/path rather than total nodes or `kN` searches.
4. **Focused input/text:** 1k-node scene with one focused input; 100/1k/10k-character caret drags and wraps. Gate: decoration has no root clone and caret queries do not relayout prefixes.
5. **Transformed promotion:** warmed 100/1k-node promoted subtrees, static and transform-animated. Gate: cache-hit allocation count is zero.
6. **Pixels:** 1080p and 4K full damage plus 1%, 10%, and 50% damage. Report bytes copied/converted, worker scratch high-water, and FPS; small damage must not convert the whole surface.
7. **Blur/cache pressure:** fullscreen/overlapping backdrop blurs and adversarial unique shadow/text/gradient sizes. Gate: peak scratch and retained cache bytes remain within explicit budgets.

Recommended counters: scene revision; deep-cloned nodes/bytes; extracted item count/bytes; bounds nodes built; scene traversals/frame; text/SVG cache hit/miss/eviction and live bytes; blur scratch high-water; worker-band bytes copied; surface pixels converted; live/high-water runtime entities and input states.

## 2026-08-12 follow-up — highest-impact remaining low-risk change

**Remove the recursive DOM-subtree clone from `RuntimeWorld::patch_node`.** The live diff improves renderer buffers/extraction/caches, but `core/src/runtime_ecs.rs:309-365` is unchanged. `patch_node` currently evaluates `element.children.clone()` at every element and then recursively patches that clone. Because `Node: Clone` is deep, total copied data is the sum of all subtree sizes: `O(N*D)` (`O(N^2)` for a chain), with ancestor subtree clones kept alive on the recursive stack. This path is broadly exercised: `src/app.rs:1374-1381`, `runtime_sync_policy`, selects `PreferPatch` for every non-initial, non-structural refresh, including fragment refreshes at `src/app.rs:577-604` and `:1024-1042`.

The implementation is a semantics-preserving borrowing refactor inside `RuntimeWorld::patch_node`:

```rust
let children = {
    let data = self.entity_mut(entity).expect(/* existing message */);
    // Keep the existing current-node field updates.
    data.children.clone() // clone only compact Entity IDs to end the mutable borrow
};

let Node::Element(element) = node else { return };
debug_assert_eq!(children.len(), element.children.len());
for (child_entity, child_node) in children.into_iter().zip(&element.children) {
    // Existing path calculation and recursive patch call.
}
```

In other words, delete the owned `authored_children` tuple member and walk the caller-owned `&element.children` directly. `authored_from_node(node)` already copies only the current node's authored fields, so no ownership or observable behavior depends on cloning descendants. Retaining the small `Vec<Entity>` clone avoids a risky borrow/unsafe rewrite and keeps this change narrowly scoped.

**Acceptance gate:** existing `core/src/runtime_ecs.rs` patch/reuse tests remain unchanged; add allocation benchmarks for a 10k-node wide/balanced tree and a 1k/2k-deep chain. Patched sync must allocate/copy linearly, and the deep-chain allocated bytes/peak live bytes must no longer grow quadratically. This should land before larger `Node -> RuntimeWorld -> Node` architectural work because it removes the worst churn with minimal correctness surface.

### Implementation result

Implemented in `core/src/runtime_ecs.rs`, `RuntimeWorld::patch_node` (current line 309): descendant nodes are now borrowed from `&element.children`; only the compact `Vec<Entity>` child-ID list is cloned to release the mutable world borrow. Added `prefer_patch_reuses_branched_descendants_and_updates_authored_data` (current line 592), covering mixed text/element siblings, two nested branches, attribute/text updates, stable entity IDs, seven reused entities, and zero spawn/despawn. Verification: `cargo test -p cssimpler-core runtime_ecs` passed 6/6; `cargo test -p cssimpler-core` passed 39/39 unit tests and doc tests; `cargo fmt -p cssimpler-core -- --check` is clean after formatting.

## 2026-08-12 RuntimeApp extracted-scene retention removal

`RuntimeApp` now retains `Option<Vec<RenderNode>>` as its previous presented scene. Its paint, incremental-paint, and present-decision calls use roots-based internal entry points, so the production window loop no longer constructs or retains `ExtractedScene.items`. The public `ExtractedScene` type and the existing extracted-scene internal wrappers remain available for API compatibility and tests.

Native-glass synchronization now uses a borrowed `NativeGlassSummary { required, preferred_tint }` walk. The walk mirrors extraction's stacking order: current context, negative deferred contexts sorted by `(z-index, discovery order)`, normal-flow descendants, then non-negative deferred contexts. It stores only borrowed node references in temporary deferred lists and exits after the first tinted glass item; it never clones a `VisualStyle`, path, text layout, SVG scene, or handler payload.

Allocation measurement used a release-mode counting allocator around the exact operation removed from RuntimeApp. The scene was the headless 1920x1080 mixed pressure layout: one root plus 24x14 translucent tiles (337 background paint items). Roots were built before counters were reset and moved into `ExtractedScene::from_render_roots_owned`, so the reported delta isolates paint-item extraction rather than root construction.

| Operation | Allocations | Cumulative allocated | Peak extra live | Retained extra live |
| --- | ---: | ---: | ---: | ---: |
| `ExtractedScene::from_render_roots_owned` | 2,028 | 850,024 B | 828,512 B | 504,992 B |
| Borrowed native-glass summary | 0 | 0 B | 0 B | 0 B |

The 337 extracted items alone occupy 323,520 inline bytes (`size_of::<ExtractedPaintItem>() == 960`), before their cloned heap payloads. The new RuntimeApp path therefore removes roughly 493 KiB of retained state for this modest scene and about 830 KiB of extraction peak pressure per presented frame. A scene with positioned z-index contexts can temporarily allocate the compact borrowed-reference list, but it still retains no summary-side heap storage after the walk.

Correctness coverage compares the lightweight result directly with `ExtractedScene::{requires_native_glass, preferred_glass_tint}` for empty/plain scenes, untinted and tinted nesting, multiple roots, equal-level deferred contexts, negative z-index before normal flow, and positive promoted contexts after normal flow. Verification: `cargo test -p cssimpler-renderer --lib -- --test-threads=1` passed 194/194 tests.

## 2026-08-12 follow-up — focused native-input decoration

Implemented the immediate clone reduction in `src/app/input.rs`. `NativeTextInputs::decorate_scene` now resolves its focused state once and builds a per-call authored-root cache keyed by `ElementPath::root`. Every referenced runtime root is reconstructed at most once, then the render-tree walk borrows that snapshot for input identification and per-path `::selection` resolution. This changes the previous `O(render nodes * authored root size)` deep cloning to `O(distinct referenced roots * authored root size)` while preserving duplicate `id`/`name` key behavior across different ancestors and roots.

Regression coverage:

- `decorate_scene_preserves_duplicate_key_and_per_path_selection_semantics` verifies that three inputs sharing one key across two roots are all decorated, each path retains its ancestor-specific selection cascade, and an unrelated input remains undecorated.
- `decorate_scene_preserves_focused_caret_semantics` verifies the no-selection caret output is unchanged.
- Existing application-level uncontrolled and controlled native-input focus/edit tests also pass.

Verification: `cargo test -p cssimpler --lib app::input::tests::` passed 2/2; `app_native_text_input_focuses_and_edits_text` passed; `controlled_native_text_input_updates_app_state_through_oninput` passed. `git diff --check` is clean.

## 2026-08-12 follow-up - scrollbar stacking traversal

Implemented collect-once-per-context traversal in `renderer/src/scrollbar.rs` for wheel targeting (`ScrollbarController::handle_wheel_on_node`, current line 334), scrollbar part hit testing (`find_scrollbar_hit_node`, current line 973), and middle-button auto-scroll targeting (`find_auto_scroll_target_node`, current line 899). Context entries collect/sort positioned descendants once; ordinary descendants recurse without collection. Deferred roots start their own collection, and transform/perspective/preserve-3D boundaries deliberately start a fresh collection because the parent collector stops descent there. Clip propagation and the existing reverse-paint ordering remain unchanged: non-negative deferred contexts, own scrollbar where applicable, normal descendants, negative deferred contexts, then the current scroll target.

`collect_deferred_scrollbar_stacking_contexts` (current line 812) now uses one push/pop path scratch and clones a path only for an actual deferred entry. Test-only instrumentation covers a 192-node no-z-index chain across all three operations (`scrollbar_traversals_collect_deep_nodes_once_per_context`, current line 1671): scrollbar hit, auto-scroll target, and wheel targeting each record exactly 192 collector visits. The previous per-descendant collection would record `192 * 193 / 2 = 18,528`. `scrollbar_traversals_preserve_nested_z_index_across_perspective_boundary` (current line 1709) verifies all three operations still choose a promoted positive-z scroll container over later normal content after a perspective boundary.

Verification: focused scrollbar tests passed 10/10, including the existing visible-overflow and nested positioned-z-index wheel tests. `cargo test -p cssimpler-renderer --lib -- --test-threads=1` passed 196/196; `cargo check -p cssimpler-renderer`, formatting, and diff checks pass.
