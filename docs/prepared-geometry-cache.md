# Prepared native vector geometry: measured decision (issue #22)

**Decision: do not add a prepared-geometry cache.** Path construction and stroke
outline preparation together cost **0.4–4.0 % of an uncached native-vector
frame** in the measured workloads; coverage rasterization plus color blending
cost **95–99 %**. Reusing prepared `tiny-skia` paths or stroke outlines therefore
has a hard ceiling of a few percent, reached only when the exact same geometry
is repainted with pixel-layer caching off and nothing else changes. The renderer
keeps the direct preparation path: no retained geometry, no new budget, no cache
keys, no eviction policy, and no default memory growth.

What shipped instead is the evidence and the tooling to re-open the question
with data:

- an opt-in, per-stage probe (`cssimpler::renderer::vector_preparation_stats`)
  that attributes paint time to path construction, stroke preparation,
  projection, coverage rasterization, and coverage blending;
- `examples/vector_prepared_geometry_bench.rs` with the existing icon and curve
  workloads plus transform, zoom, paint-only, repeated-edit, and dense-stroke
  variants, and `scripts/vector_prepared_geometry_bench.py` to drive several
  fresh release processes and report medians and spreads;
- `tests/issue_22_prepared_geometry.rs`, which pins that the probe does not
  change pixels and that every paint really does rebuild paths and outlines
  (i.e. there is no hidden cache, retained history, or paint-keyed reuse);
- the raw measurement report in
  [`reports/issue-22-prepared-geometry.md`](reports/issue-22-prepared-geometry.md)
  and `...md.jsonl`.

## What was measured, and how

The probe is disabled by default. When enabled it reads two clocks per path
(construction, stroke, projection as applicable) and two per coverage tile
(rasterization, blending) and accumulates nanoseconds into process-wide counters,
so parallel render workers contribute their own CPU work. Stage sums are
therefore *aggregate CPU*, not wall time; the harness reports wall frame time
separately with the probe off, and a scenario is only interpreted as a wall
share when the paint is serial.

Every frame is painted through the plain, uncached `render_to_buffer` path with
`cache_layer = false`; pixel-layer caching is never requested. Scene traversal is
measured by painting the identical node tree with empty vector scenes, so
`Frame − Empty` is all vector path work. Presentation is approximated by the
headless RGBA8 finalize (`render_to_rgba8` minus a paint of the same tree);
there is no window or swap chain in this environment, so OS presentation is not
measured.

Workloads use the same 128-icon and 12-curve scenes as the existing vector
benchmarks:

| Scenario | Scene |
|---|---|
| `small-icon` | 128 stroked icons, 512×512 (serial paint) |
| `small-icon-edit` | same, one replacement path per frame |
| `dense-strokes` | 16 stroke-only motifs, 128 authored segments each, 512×512 |
| `curves` | 12 curved stroked drawings, 960×640 (parallel paint) |
| `curves-serial` | same scene in a 560×360 window (serial paint) |
| `curves-translate` | geometry reused, translation-only transform per frame |
| `curves-zoom` | geometry reused, 1.00–1.35× magnification per frame |
| `curves-paint-update` | geometry reused, one fill repainted per frame |
| `curves-edit` | one path replaced with new geometry per frame |

## Results

Seven fresh release processes per scenario, round-robin over scenarios, medians
with min..max spreads in the report. Machine: Apple M1 Pro, 8 logical CPUs,
16 GiB, macOS `Darwin arm64`; load average was 5.3–6.2 at the start of the run
and 5.8–6.2 afterwards (other agents build concurrently on this machine).
Per-process ALU calibration medians were 3.79–3.92 ms across every scenario and
run, so the samples were taken at a stable effective clock.

Geometry preparation is path construction + stroke outline only, and it is
converted to a *wall-equivalent* by multiplying by `frame / attributed CPU`
(1.0 for serial paint, ≈6.4 for the parallel 960×640 paint):

| Scenario | Frame ms | Build µs | Stroke µs | Raster µs | Blend µs | Prep wall-equiv. | % of frame |
|---|---:|---:|---:|---:|---:|---:|---:|
| small-icon | 5.930 | 22.2 | 209.6 | 2238 | 3394 | 0.234 ms | **3.95 %** |
| small-icon-edit | 5.895 | 21.0 | 209.3 | 2241 | 3400 | 0.232 ms | **3.94 %** |
| dense-strokes | 13.978 | 23.3 | 349.0 | 8807 | 4696 | 0.374 ms | **2.67 %** |
| curves-serial | 4.511 | 1.4 | 16.9 | 802 | 3678 | 0.018 ms | **0.40 %** |
| curves (parallel CPU) | 3.414 | 50.1 | 343.2 | 8383 | 12558 | — | 1.93 %* |
| curves-translate | 3.504 | 51.6 | 357.0 | 8814 | 12606 | — | 1.89 %* |
| curves-zoom | 3.836 | 52.5 | 383.0 | 9309 | 14988 | — | 1.75 %* |
| curves-paint-update | 3.390 | 49.1 | 385.6 | 8341 | 12685 | — | 2.03 %* |
| curves-edit | 3.401 | 54.2 | 339.2 | 8354 | 12758 | — | 1.88 %* |

\* wall-equivalent (`prep CPU × frame / attributed CPU`) for the parallel paint;
the raw CPU share of attributed paint work is the same 1.8–2.0 %.

Reading the serial scenarios directly: for the icon workload, path
construction is **0.37 %** and stroke preparation **3.53 %** of the frame, while
rasterization is 37.7 % and blending 57.2 %; traversal of the single vector node
and its bounds checks is about 1 %. For the dense-stroke scene (the most
preparation-heavy shape measured), preparation is 2.7 % against 63.0 % raster and
33.6 % blend. For the curved drawings, preparation is 0.40 % (serial window) to
1.75–2.03 % (parallel), with raster + blend at 98–99 %.

Three details strengthen the conclusion:

- **Worker bands already duplicate preparation, and removing it all would still
  be small.** The parallel `curves` frame builds and strokes 80 paths for 12
  authored paths: each band that intersects a path recomputes it (≈6–7
  recomputations per path). Even a perfect cross-worker cache that removed
  *every* duplicate would save 0.394 ms of 21.4 ms aggregate paint CPU, about
  0.066 ms of the 3.41 ms wall frame.
- **The intermediate scenarios do not add preparation work.** `curves-edit`
  (a new geometry identity every frame), `paint-update` (paint changed only) and
  `translate`/`zoom` (transform changed only) all sit at 1.75–2.03 %, so a cache
  would either miss (edits) or save nothing beyond the static case (paint and
  transform updates).
- **The headless finalize costs more than all geometry preparation.** The
  RGBA8 conversion path (buffer allocation + 10-bit unpack for 512²–960×640)
  measured 0.55–1.12 ms on top of a paint, i.e. 3–43× the wall-equivalent
  build+stroke total (6–15× for the icon, dense and curve scenes), even before
  any real window presentation. Prepared geometry is not where the frame time
  is.

## Why a cache is not worth it here

The measured ceiling is **0.4–4.0 % of the frame**, and it is an upper bound:
it assumes a 100 % hit rate, zero key lookup cost, zero synchronization, and no
invalidation. The requirements the issue lists are exactly the parts that would
consume that margin:

- Cache keys would have to include geometry identity, stroke width, caps/joins,
  and the stroker's resolution scale (which changes with zoom and is continuous,
  so it must be quantized *upward* to preserve quality), plus any perspective
  projection parameters. Paint must stay out of geometry keys.
- Worker bands and dirty clips paint the same path concurrently; avoiding
  duplicate cold preparation without globally serializing paint means sharded
  maps or per-key once-cells, i.e. synchronization in the hottest loop.
- Retained storage needs a byte budget, LRU eviction, and honest accounting.
  `tiny-skia::Path` exposes verbs and segments but not its `Vec` capacities, so
  the "actual buffer capacities" requirement cannot be met through the public
  backend API without vendoring or forking it; the content-size estimate is
  only a lower bound on retained bytes.
- Magnification, nonuniform scaling, perspective, clipping and row bands all
  have to keep byte-identical output.
- The only case a geometry cache helps is painting identical geometry again
  with *pixel-layer caching off*. When the drawing is unchanged, the supported
  and already-implemented answer is the pixel-layer cache
  ([`layer-caching.md`](layer-caching.md)), whose hit skips geometry preparation
  entirely; when the drawing changes, geometry preparation is the wrong target.

For scale: retained prepared geometry for the 128-icon scene would be on the
order of 70–100 KB of content bytes (fill paths plus round-joined outlines,
from tiny-skia's verb/point storage), and 60–70 KB for the dense-stroke scene —
small, but purchased for a ~0.2–0.4 ms per-frame saving that a raster cache
already delivers for the real repeated-frame case. The issue explicitly prefers a
documented no-change outcome over complexity without evidence; this is that
outcome.

## Limits of this measurement

- Synthetic scenes, not application traces. The icon and curve scenes are the
  repository's existing vector benchmark workloads, but real applications vary
  in path count, size, opacity, and clip complexity.
- Path count and command count are moderate (≤128 authored commands per path,
  9–128 paths per scene). Construction cost grows roughly linearly with commands
  (≈18 ns per command here) and stroke cost grows with outline segments, so a
  scene that repaints a few *thousands-of-segment* stroked paths uncached every
  frame is the plausible case where the share could exceed 10 %. That case was
  not measured; see "when to revisit".
- The stage clocks themselves add work to the timed stages (two `Instant::now`
  calls per stage sample). Probe-on wall time differed from probe-off by
  −0.10…+0.08 ms (medians) across scenarios, i.e. within noise at these frame
  sizes, and the effect inflates the preparation numbers rather than hiding
  them. Very small per-path costs (path build is ~0.17 µs per icon) are
  correspondingly less precise than the multi-millisecond coverage stages.
- Wall times were taken on a shared machine with other agents building and
  testing; medians of seven processes with min..max spreads are reported, and
  per-process calibration confirms a stable effective clock, but this is not a
  quiet-machine, cycle-accurate measurement.
- Presentation is approximated by the headless RGBA8 finalize. Softbuffer/WGPU
  submission and compositor time are not measured and could dominate a real
  frame; nothing here predicts an end-to-end frame-rate change.
- `build_svg_path` also has a legacy flattened-contour input route (SVG element
  parsing). The probe covers it because it runs in the same paint loop, but the
  workloads use authored native commands only.

## When to revisit

Re-open this decision with new measurements, not estimates, if all of the
following hold for a real application scenario:

1. Its drawings are repainted with pixel-layer caching off (e.g. every frame has
   a paint or layout change that invalidates the raster), **and**
2. preparation measured with this probe (`geometry_prep_ns` per frame) is more
   than ~10 % of the frame, typically because paths carry thousands of authored
   commands or are stroked at high magnification, **and**
3. the same geometry identities survive across frames (otherwise a cache only
   adds key cost and memory churn).

If those hold, the cache would have to ship with: identity-based keys (not
formatted command strings — `VectorGeometry::identity()` already provides the
token), style/resolution-quantized stroke keys, sharded per-key preparation to
avoid duplicate cold work without globally serializing paint, a configurable
byte budget with LRU eviction and a zero-budget direct-preparation path, no new
RGBA layers, and cache-on/cache-off pixel-equality tests including worker bands,
dirty clips, magnification and perspective. Fold its hit/miss/build/retained-byte
counters into the vector diagnostics snapshot once PR #23 lands rather than
adding a second public stats surface.

## Reproduce

```sh
cargo build -j 2 --release --example vector_prepared_geometry_bench
# 7 fresh processes per scenario, round-robin, medians + spreads:
python3 scripts/vector_prepared_geometry_bench.py --runs 7 \
    --markdown docs/reports/issue-22-prepared-geometry.md
# one scenario in detail (single JSON line):
cargo run --release --example vector_prepared_geometry_bench -- --scenario small-icon
```

Tests that pin the decision: `cargo test -j 2 --test issue_22_prepared_geometry`
and the probe unit tests in `renderer/src/svg_preparation.rs`
(`cargo test -j 2 -p cssimpler-renderer svg_preparation`).
