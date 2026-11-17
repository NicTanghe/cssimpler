# Issue #22 — bounded reuse of prepared native paths and stroke outlines

## 2026-11-17 21:05 UTC - agent/issue-22-prepared-geometry-cache

Status: investigation complete; **decision is no cache** (documented no-change
outcome). Branch `agent/issue-22-prepared-geometry-cache` is stacked on
`codex/shared-vector-geometry` at `7772cf3` so it can reuse
`VectorGeometry::identity()`/`bounds()`; PR
[#29](https://github.com/NicTanghe/cssimpler/pull/29) targets that branch and
must be retargeted to `main` after PR #24 merges. Head at recording time:
`65e7b807711dc485121909205cb528000032ea3b`. No edits to
`FrameTimingStats`/`VectorFrameStats` or the `--diagnostics` example (owned by
PR #23 / issue #18).

### What was added (all new files except three small hooks)

- `renderer/src/svg_preparation.rs`: opt-in per-stage probe. Stages: path
  construction, stroke outline preparation, projection, coverage rasterization,
  coverage blending, plus `paths_painted`/`stroke_outlines`/`coverage_tiles`
  counters and `geometry_prep_ns()`/`attributed_ns()` helpers. Disabled by
  default (one relaxed atomic load per stage when off; no clock read). Public
  API: `set_vector_preparation_probe_enabled`, `vector_preparation_probe_enabled`,
  `reset_vector_preparation_stats`, `vector_preparation_stats`.
- `renderer/src/svg.rs`: 4 hook sites (build, stroke, fill/stroke projection,
  coverage tile raster+blend). Behaviour is unchanged; the existing
  `if let ... && let ...` chains were split into nested blocks only to time the
  projection stage separately.
- `renderer/src/lib.rs`: module + re-exports (24 lines incl. the probe file).
- `examples/vector_prepared_geometry_bench.rs`: 9 workloads (existing 128-icon
  and 12-curve scenes from #17's bench, plus dense-strokes, curves-serial,
  translate, zoom, paint-update, edit) and stage/wall/traversal/finalize output
  as one JSON line.
- `scripts/vector_prepared_geometry_bench.py`: N fresh-process runs,
  round-robin across scenarios, medians + min..max, writes the markdown report
  and raw `.jsonl`.
- `tests/issue_22_prepared_geometry.rs`: probe on/off pixel equality, every
  paint rebuilds paths/outlines (no hidden cache/history), paint-only changes
  still rebuild but recolor, worker-band aggregation is lossless and
  deterministic.
- `docs/prepared-geometry-cache.md`: decision, numbers, limits, revisit criteria.
- `docs/reports/issue-22-prepared-geometry.md` + `.md.jsonl`: recorded run.

### Measurement setup

Machine: Apple M1 Pro, 8 logical CPUs, 16 GiB, macOS `Darwin arm64`,
`rustc 1.101.0-nightly (32dba69d6 2026-10-09)`, release profile. 7 fresh
processes per scenario, round-robin; load average 5.26 6.16 10.53 at start and
5.75 6.22 10.40 after the benchmark window (other agents build concurrently).
Per-process ALU calibration medians: 3.79–3.92 ms, stable.

Commands:

```sh
cargo build -j 2 --release --example vector_prepared_geometry_bench
python3 scripts/vector_prepared_geometry_bench.py --runs 7 \
    --markdown docs/reports/issue-22-prepared-geometry.md
```

Pixel-layer caching explicitly off in every tree; `render_to_buffer` only.
Stage sums are aggregate CPU across paint workers; wall frame time is measured
with the probe off. `Empty scene` = same node tree with empty vector scenes.

### Medians (7 processes/scenario)

| scenario | frame ms | build µs | stroke µs | raster µs | blend µs | build+stroke % of paint (wall-equiv.) |
|---|---:|---:|---:|---:|---:|---:|
| small-icon (128 stroked icons, serial) | 5.930 | 22.2 | 209.6 | 2238 | 3394 | 3.95 % |
| small-icon-edit | 5.895 | 21.0 | 209.3 | 2241 | 3400 | 3.94 % |
| dense-strokes (16×128-segment strokes) | 13.978 | 23.3 | 349.0 | 8807 | 4696 | 2.67 % |
| curves (parallel CPU) | 3.414 | 50.1 | 343.2 | 8383 | 12558 | 1.93 % |
| curves-serial (560×360 window) | 4.511 | 1.4 | 16.9 | 802 | 3678 | 0.40 % |
| curves-translate | 3.504 | 51.6 | 357.0 | 8814 | 12606 | 1.89 % |
| curves-zoom (1.00–1.35×) | 3.836 | 52.5 | 383.0 | 9309 | 14988 | 1.75 % |
| curves-paint-update | 3.390 | 49.1 | 385.6 | 8341 | 12685 | 2.03 % |
| curves-edit | 3.401 | 54.2 | 339.2 | 8354 | 12758 | 1.88 % |

Additional facts from the same run:

- Probe overhead (probe-on minus probe-off wall median): −0.10…+0.08 ms,
  i.e. within noise; it inflates the stage numbers rather than hiding them.
- Parallel paint builds/strokes 80 paths per frame for 12 authored paths
  (≈6.7× band duplication). Removing *all* of it is 0.394 ms of 21.4 ms
  aggregate paint CPU → ≈0.066 ms of the 3.41 ms wall frame.
- Headless RGBA8 finalize costs 0.55–1.12 ms per frame on top of a paint,
  3–43× the wall-equivalent build+stroke total (6–15× for icon/dense/curves).
  OS presentation is not measurable headless here.
- Process RSS medians (before → after, KiB): small-icon 7280 → 13712,
  dense-strokes 7376 → 13584, curves 7248 → 17920. Growth is the pixel and
  RGBA buffers; no geometry is retained by design.

### Decision and rationale

Geometry preparation (build + stroke) is 0.4–4.0 % of an uncached frame;
rasterization + blending is 95–99 %. A cache's ceiling (100 % hits, no lookup or
contention cost) is therefore a few percent, and only for repaints of identical
geometry with pixel-layer caching off. The existing pixel-layer cache already
removes geometry preparation entirely for unchanged drawings, and the edit /
paint-only / transform-update scenarios add no preparation benefit beyond the
static case. Requirements the issue lists (identity keys, resolution-quantized
stroke keys, cross-worker dedup without global serialization, byte budget + LRU,
zero-budget path, magnification/perspective/band correctness) would consume that
margin. `tiny-skia::Path` does not expose its `Vec` capacities, so the
"actual buffer capacities" accounting requirement cannot be met through the
public backend API.

Outcome: cache unimplemented and disabled; direct preparation remains the only
path. Tests pin that invariant. See `docs/prepared-geometry-cache.md` for the
full discussion, limitations, and the criteria to revisit.

### Validation (all run on the branch tip)

- `cargo check --workspace -j 2` — finished OK; only the pre-existing
  `unused dependency anyhow` manifest warning.
- `cargo test -j 2 --test issue_22_prepared_geometry` — 4 passed, 0 failed.
- `cargo test -j 2 -p cssimpler-renderer svg_preparation` — 3 passed, 0 failed
  (270 filtered out).
- `cargo test -j 2 -p cssimpler-renderer svg::tests` — 12 passed, 0 failed
  (261 filtered out), including the tiled-coverage, worker-band/dirty-clip,
  nonuniform-stroke and perspective tests that exercise the hooked paint loop.
- `cargo test -j 2 --test issue_17_shared_geometry --test vector_sharing` —
  9 passed + 1 passed, 0 failed (base shared-geometry regressions).
- `cargo build -j 2 --release --example vector_prepared_geometry_bench` — OK;
  `python3 scripts/vector_prepared_geometry_bench.py --runs 7 …` wrote the
  report and JSONL committed above.
- Full `cargo test --workspace` was not run (slow on this shared machine);
  focused vector/renderer tests stand in for it.

### Open items / follow-ups

- Fold coverage raster/blend stage clocks and the probe into the #23
  (`VectorFrameStats`) snapshot after PR #23 merges; today they are separate to
  avoid editing #23-owned structs.
- Coverage rasterization/blending is the real cost (measured), not geometry.
  Candidate follow-ups outside this issue: per-path coverage-tile clear cost
  (256² mask cleared per tile; ~16 MiB/frame for the 128-icon scene), linear-light
  blend cost, headless finalize cost.
- No prepared-geometry hit/miss/retained-byte counters were added because no
  cache exists; adding them would be dead API surface.
