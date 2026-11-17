# Issue #19 — native vector pan/zoom reusing geometry

## 2026-11-17T20:52Z - issue-19 agent

Branch: `agent/issue-19-pan-zoom-geometry`, worktree
`worktrees/issue-19`. Stacked on the open PR #24 branch
`codex/shared-vector-geometry` at `7772cf3` (not on `main`). The PR must be
created with `--base codex/shared-vector-geometry` and retargeted to `main`
once #24 merges.

### What was implemented

- `VectorView` (`core/src/vector.rs`): a validated, `Copy`, 12-byte
  document-to-view transform (`view = document * zoom + offset`) with
  document/view and document/viewport mapping, inverse pointer mapping,
  cursor-anchored zoom, panning, practical zoom limits (`1/64 ..= 64`), and
  rejection of non-finite/non-positive input. Private fields keep the transform
  non-singular by construction.
- `VectorScene::{view, with_view, set_view}`: the view lives beside the authored
  `VectorGeometry` handles. `set_view` is O(1), allocation-free, and does not
  touch commands. `to_svg` deliberately ignores the view.
- `SvgScene.view: Option<VectorView>` (one new public field; constructors
  default to `None`, parsed SVG keeps legacy behavior). `into_render_scene`
  carries the view through the shared pipeline.
- `renderer/src/svg.rs`: composes `CSS/layout matrix * viewBox map * view`; when
  `Some(view)` is set, reuses `ClipState::push_layout_clip` to treat the
  element's layout box as the viewport and clip drawing to it. `None` preserves
  legacy overflow behavior.
- `examples/vector_pan_zoom.rs`: headless demo by default (pan, cursor-anchored
  zoom, inverse mapping, 400 repeated view changes, export stability),
  `--interactive` (drag/wheel/status line), `--export FILE`, and `--bench`.
- `tests/issue_19_pan_zoom.rs`: 10 tests (inverse mapping, cursor-anchored zoom,
  limits/invalid input, zero-allocation repeated view updates, export/curve
  stability, viewport clipping, CSS transform + layout composition, device
  scaling, dirty-region incremental==full paint, raster-cache invalidation).
- `docs/vector-pan-zoom.md` plus one link line in `docs/README.md` and raw
  benchmark samples in `docs/reports/issue-19-pan-zoom-runs.jsonl`.

### Determination: helper vs camera abstraction

A live view cannot safely ride the existing `Style.transform` field: an
element's inline `Style` is the base that matching stylesheet rules overwrite.
Verified on this revision with a temporary example: inline base
`scale(3)` + stylesheet `#draw { transform: translate(20px, 20px) }` resolves to
`Translate { 20px, 20px }` (scale lost). The focused scene-level `VectorView`
is therefore the smallest composition-safe pattern: a zoom and offset only, no
scene ownership, registry, history, camera graph, or per-zoom geometry/raster
copies. It composes with the existing CSS/layout/device transform chain and
clipping pipeline.

### Validation commands and results

Run from the worktree with `-j 2` where applicable:

| Command | Result |
| --- | --- |
| `cargo check --workspace -j 2` | pass (only the pre-existing unused `anyhow` manifest warning) |
| `cargo test -j 2 -p cssimpler-core` | 52 passed, 0 failed |
| `cargo test -j 2 -p cssimpler-renderer` | 270 passed, 0 failed |
| `cargo test -j 2 -p cssimpler-style` | 137 passed, 0 failed |
| `cargo test -j 2 --test issue_19_pan_zoom` | 10 passed, 0 failed |
| `cargo test -j 2 --test issue_17_shared_geometry --test vector_sharing` | 9 + 1 passed, 0 failed |
| `cargo run -j 2 --example vector_pan_zoom` | headless demo pass; export stable over 400 view changes |
| `cargo run -j 2 --example vector_pan_zoom -- --export work/pan-zoom.svg` | wrote authored SVG (view not baked in) |
| `cargo build --release -j 2 --example vector_pan_zoom` | pass |
| `target/release/examples/vector_pan_zoom --bench` ×5 | see measurements below |

The interactive window mode was compiled but not run: this environment has no
usable display/window server (headless agent). All behavior it covers is also
covered by headless tests.

### Measurements (recorded 2026-11-17, macOS, busy shared machine)

Harness details: release, pixel-layer caching explicitly off (both cache limits
zero, no `cache_layer`), 12 curved/filled/stroked paths at 960×640, counting
global allocator, calibration ALU loop, RSS from `ps` on macOS (`/proc` absent,
so peak RSS is `null`). Five fresh processes, medians of run medians; load
average 9.26 → 9.64 during the window. Raw JSONL:
`work/issue-19-bench.jsonl` (copy committed at
`docs/reports/issue-19-pan-zoom-runs.jsonl`).

| Phase | Median | Min..max | Allocs/op | Bytes/op |
| --- | ---: | ---: | ---: | ---: |
| view-only `set_view` (5000 ops) | ~0.00002 ms | 0.00000..0.00004 ms | 0 | 0 |
| scene rebuild + node (400 ops) | 0.00025 ms | 0.00025..0.00025 ms | 4 | 1,662 |
| rebuild + preparation (200 ops) | 0.00137 ms | 0.00133..0.00142 ms | 16 | 27,886 |
| uncached full paint (8 ops) | 3.26308 ms | 3.06192..3.63479 ms | 2,440 | 1,476,045 |

Retained memory (RSS deltas, KiB): view-only 1,000→6,000 updates +128
(one-time harness/allocator warm-up), 6,000→11,000 updates 0; full pipeline
100→1,000 frames +240 (160..272), 1,000→2,000 frames 0. `VectorView` is 12 bytes
(16 as `Option<_>`); `VectorScene` is 56 bytes independent of command count.
Conclusion: retained memory plateaus and does not grow with pan/zoom count;
view changes allocate nothing; uncached paint still dominates.

### Gaps / notes

- View supports translation + uniform zoom only (issue scope); rotation/skew
  remain in the CSS transform channel.
- The app must pass the element's actual layout box to the pointer-mapping
  helpers; the example pins a known 800×600 layout (documented).
- Measurements are pixel-layer-caching-off. With caching on, a view change
  invalidates/rebuilds the raster at the new magnification (cache key includes
  the scene view); entries stay bounded by the existing LRU byte budget. This is
  documented, not benchmarked.
- `SvgScene` gained a public `view` field, so external struct literals need
  `view: None`; constructors are unchanged. Related to the #24 compatibility
  note for `SvgPathGeometry.commands`.
- The default example mode is headless so `cargo run --example vector_pan_zoom`
  is meaningful without a display; `--interactive` is the window path.

### Anticipated conflicts

- `renderer/src/svg.rs` also being touched by issue #22 (prepared geometry):
  this change only adds view composition/clipping around the existing
  `build_svg_path`/coverage path; keep the `draw_svg_scene*` and
  `svg_view_box_matrix` changes when resolving conflicts.
- `core/src/vector.rs` is also touched by #17 (already in the stacking base) and
  #21 (builder): additive changes only; view field is private.
- #20 (input) may replace the event-handler invalidation pattern used by the
  interactive example; the example's pan/zoom state is independent.
- #18 (statistics) touches renderer counters; no new counters were added here.
