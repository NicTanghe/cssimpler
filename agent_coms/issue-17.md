# Issue #17 - shared immutable native vector geometry

## 2026-11-17T20:32:00Z - issue-17 agent

Status: PR #24 on branch `codex/shared-vector-geometry`. Core sharing landed in
`cce8246`; this entry records the post-audit fixes and the A/B measurement that
closed the known-revision gap.

### Audit result (code, not just description)

- DOM: `Node::vector` consumes `VectorScene` via `into_render_scene`; geometry is
  moved, not copied.
- Runtime: `authored_from_node`, `entity_as_node`/`root_as_node` clone
  `SvgScene`; each `SvgPathGeometry.commands` is an `Arc` handle, so command
  buffers are shared and only per-path `SvgPathInstance`/paint/`Vec` bookkeeping
  is allocated.
- Style/layout: `resolve_element_tree` clones the scene into `ResolvedElement`
  and then `RenderNode`; layout only reads the viewBox. A CSS width/height/
  transform/background rebuild test proves geometry stays shared
  (`style_and_layout_rebuild_reuses_geometry`).
- Paint: `renderer/src/svg.rs::build_svg_path` walks the shared slice into a
  tiny-skia `Path`; no command `Vec` is copied. Prepared-path caching is out of
  scope (issue #22).
- SVG export: `to_svg_path_data` walks the shared slice and keeps `Q`/`C`
  commands (`svg_export_preserves_original_curves_and_precision`).
- Which buffers can be shared: the command array + conservative control-point
  bounds. Not shared: `Vec<SvgPathInstance>`, per-instance paint, legacy
  `Vec<SvgContour>`, paint servers. Costs are O(#paths), not O(#commands).
- Invalidation: `VectorGeometry::identity()` is a process-local `u64` allocated
  once at construction. The layer-cache key hashes the render node `Debug`, and
  `VectorGeometry`'s `Debug` prints `identity` + command count instead of the
  command slice, so geometry edits invalidate and unchanged geometry is not
  re-serialized. Debug length is asserted constant (<80 bytes for 4096
  commands) and the renderer test proves paint/replacement invalidation.
- Lifetime: no registry. `repeated_revisions_release_old_geometry` holds 64
  `Weak` handles and asserts every replaced revision is released and the live
  handle has strong count 1.

### Fixes/coverage added in this session

- `core/src/vector.rs`: bounded-cost `Debug` (identity + command count);
  repeated-edit release test.
- `tests/issue_17_shared_geometry.rs` (9 tests): independent instances with
  different paints and viewBoxes, style/layout-only rebuild sharing, runtime
  paint patch + geometry replacement identity/bounds, render-tree sharing and
  paint-dependent bounds, independent red/blue rendering of the same geometry,
  SVG curve/precision export, Debug cost, identity uniqueness, Send+Sync.
- `core/examples/vector_clone_measure.rs`: portable (Linux `/proc`, macOS `ps`),
  JSON output, configurable `--clones/--commands`, warmup.
- `examples/vector_sharing_bench.rs`: compiles against baseline and head; five
  scenarios; pixel-layer caching off (asserted); allocation counting; fixed ALU
  calibration loop so timings can be normalized against per-process machine
  speed.
- `scripts/vector_sharing_ab.py`: alternating A/B driver, 7 runs, medians and
  min..max; raw JSONL next to the markdown.
- `docs/vector-geometry-sharing.md`: ownership/pipeline/invalidation audit,
  methodology, results, limitations. Removed the stale "measurement not
  completed" section from `docs/native-vector-paths.md` (coordination: issue #18
  also edits that file; its content was moved to the new document to avoid
  appending to the shared file).

### Measurement evidence

Machine: Apple M1 Pro, 8 logical CPUs, 16 GiB, macOS Darwin 27.0.0 arm64,
rustc/cargo 1.101.0-nightly, release profile (opt-level 3, thin LTO,
codegen-units 1). The machine is shared with other agents; load average during
the recorded runs was 24-31 (it had been 16-21 and up to 197 around reboot).
Because of that, allocation counts/bytes and normalized timings are the primary
evidence; raw wall times are indicative.

Baseline worktree: `6d49b79` in `/tmp/cssimpler-issue17-baseline` with its own
`CARGO_TARGET_DIR=/tmp/cssimpler-issue17-target` (never shared with the head
target directory). The same harness files were copied into the baseline
worktree (sha256 verified equal).

Head vs baseline (`6d49b79`) medians, 7 alternating runs:

| Scenario | construct | prepare | per-frame update | uncached paint |
|---|---|---|---|---|
| small-icon (128 paths) | 5.29 -> 1.17 us (-77.9%) | 13.63 -> 3.33 us (-75.6%) | 19.04 -> 4.50 us (-76.4%) | 5.93 -> 5.95 ms (+0.3%) |
| curves (12 stroked paths) | 0.54 -> 0.25 us | 1.88 -> 1.08 us (-42.6%) | 2.42 -> 1.29 us (-46.7%) | 3.41 -> 3.41 ms (0.0%) |
| transform-update | 0.54 -> 0.25 us | 1.88 -> 1.13 us (-39.9%) | 2.50 -> 1.42 us (-43.2%) | 3.44 -> 3.49 ms (+1.4%) |
| paint-update | 0.58 -> 0.21 us | 1.88 -> 1.13 us (-39.9%) | 2.42 -> 1.29 us (-46.7%) | 3.60 -> 3.53 ms (-2.0%) |
| repeated-edits | 0.58 -> 0.25 us | 1.88 -> 1.13 us (-39.9%) | 2.42 -> 1.37 us (-43.4%) | 3.46 -> 3.56 ms (+2.7%) |

Allocations/op: construct 132 -> 4 (icons), 16 -> 4 (curves); update 528 -> 16
(icons), 64 -> 16 (curves). Requested bytes fell 17-68%. Paint allocations and
bytes were identical in every scenario (e.g. 3715 allocs / 1,812,000 bytes for
icons), so no paint improvement is claimed.

Retained clones (10,000 clones of one 1,000-command path), 7 alternating runs,
medians: baseline 10,000 allocations / 280,400,000 bytes / RSS 283,584 KiB;
head 1 allocation / 240,000 bytes / RSS 2,176 KiB. Clone-loop elapsed 23.7 ms ->
0.049 ms (load-sensitive, huge margin).

Raw: `docs/reports/issue-17-vector-sharing-bench.md{,.jsonl}`,
`docs/reports/issue-17-vector-sharing-clone.md{,.jsonl}`.

### Validation commands

- `cargo check --workspace` (pass; one pre-existing unused `anyhow` dependency
  warning on the root package).
- `cargo test -p cssimpler-core vector::tests --lib` (5 passed).
- `cargo test --test issue_17_shared_geometry` (9 passed).
- `cargo test --test vector_sharing` (1 passed).
- `cargo test -p cssimpler-renderer native_shared_geometry_cache_invalidates_paint_and_replacement --lib`
  (1 passed).
- `cargo test -j 2 --workspace --no-fail-fast` (557 passed, 0 failed: 81 root,
  9 `issue_17_shared_geometry`, 1 `vector_sharing`, 52 core, 7 macro, 270
  renderer, 137 style; doc-tests 0). Run after the focused suites above; all
  passed.

### Open items / caveats

- Paint does not get faster; prepared geometry caching is issue #22.
- Wall-clock numbers come from a busy shared machine; normalization and
  deterministic allocation counts are the reliable part. A quiet re-run is
  only needed if reviewers distrust the timing direction; the allocation
  evidence does not depend on load.
- Full-workspace tests were run after the focused suites (557 passed, 0 failed).
