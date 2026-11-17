# Shared native vector geometry (issue #17)

This document records the ownership model, the pipeline audit, the cache
invalidation strategy, and the reproducible before/after measurements for
sharing immutable native vector geometry. The public API guide lives in
[`native-vector-paths.md`](native-vector-paths.md); this page covers internals
and measurement evidence.

## Ownership model

A native drawing is split into two layers:

- `VectorGeometry` is an immutable `Arc` handle to the validated authored
  commands plus their conservative control-point bounds and a process-local
  identity. Cloning it only bumps the reference count.
- `VectorPath` pairs a `VectorGeometry` with an independent `VectorPaint`, and
  `VectorScene` pairs paths with an independent viewBox.

```rust
let geometry = VectorGeometry::new(vec![
    PathCommand::MoveTo(VectorPoint::new(4., 4.)),
    PathCommand::LineTo(VectorPoint::new(28., 4.)),
])?;
let red = VectorPath::from_geometry(geometry.clone(), red_paint)?;
let stroked = VectorPath::from_geometry(geometry.clone(), stroke_paint)?;
```

Editing means constructing replacement geometry with `VectorGeometry::new`
(usually through `VectorPath::new`/`VectorScene::new`); the old handle is
released when its last owner drops. There is no geometry registry and no manual
revision counter: every construction receives a new identity automatically.

`VectorGeometry::default()` is a canonical empty handle, so legacy
contour-only `SvgPathGeometry` values share one allocation instead of each
carrying an empty `Vec`.

## Pipeline audit: what is actually shared

| Stage | Path through the pipeline | Command storage |
|---|---|---|
| Application scene | `VectorScene`/`VectorPath` clone | `Arc` bump; paint and viewBox per instance |
| DOM native element | `Node::vector(scene)` → `VectorScene::into_render_scene` | moved, not copied; bounds handed to the renderer |
| Runtime sync | `authored_from_node` stores `SvgScene` | `Arc` bump per path; element/path `Vec`s cloned |
| Runtime round-trip | `root_as_node` / `entity_as_node` clone `SvgScene` | `Arc` bump per path |
| Style/layout resolution | `ResolvedElement.svg_scene = scene.clone()` | `Arc` bump per path; layout uses only the viewBox |
| Render-tree construction | `RenderNode::kind = RenderKind::Svg(scene)` | `Arc` bump per path |
| Paint refresh | `build_svg_path` iterates the shared slice into a tiny-skia `Path` | read-only; no command `Vec` copy |
| SVG export | `to_svg_path_data` iterates the shared slice | read-only; curves are never flattened |

The shared buffer is the command array (and the derived bounds). What is still
freshly allocated per stage is O(number of paths), not O(number of commands):
the `Vec<SvgPathInstance>`, per-instance paint, paint servers, and legacy
flattened contour storage. Geometry-heavy drawings therefore stop copying their
large buffers while the small per-instance bookkeeping remains. Prepared
geometry caches (for example reusing the tiny-skia `Path` across paints) are a
separate concern and are not part of this change.

## Invalidation, fingerprints and cache keys

- `VectorGeometry::identity()` is a `u64` allocated once per construction from a
  process-local atomic counter. Clones keep it; content-identical replacement
  geometry gets a new identity.
- The renderer's layer-cache key already hashes the `Debug` representation of
  the render-tree node. `VectorGeometry`'s `Debug` prints
  `VectorGeometry { identity, commands: len }` instead of serializing the
  command slice, so geometry edits invalidate the key and unchanged geometry
  does not repeatedly format command arrays. The retained cost is one atomic
  increment per construction; there is no per-paint geometry hashing.
- Paint, layout, viewBox, bounds and style remain independently hashed as
  before. Two instances sharing a geometry handle can therefore carry different
  paints or viewBoxes without sharing cache entries.
- The identity is process-local and not persisted. It is an invalidation token,
  not a content hash; a content-identical reconstruction intentionally misses a
  cache it has never populated.

`SvgPathGeometry.commands` dereferences to `&[PathCommand]`; low-level callers
that replace it in place must update `SvgPathGeometry.bounds` together with the
handle (and `SvgScene.bounds` if they keep that cached field). The
`VectorPath`/`VectorScene` constructors do this automatically.

## Lifetime and bounded memory

The only strong references are the owners: paths, scenes, DOM/runtime/render
nodes, and render trees. Once those are replaced or dropped the command buffer
is freed. There is no global registry or history, so repeated edits retain one
revision at a time. `Arc` reference counting can delay reclamation only while a
live cache or render tree still references the old geometry.

## Compatibility

`SvgPathGeometry.commands` changed from `Vec<PathCommand>` to the shared
`VectorGeometry` handle. Code that constructed `SvgPathGeometry` with a struct
literal must wrap commands with `VectorGeometry::new(...)?` and set `bounds`,
or use `VectorGeometry::default()` for contour-only geometry. Reads continue to
work because the handle dereferences to a slice. `VectorPath`/`VectorScene`
constructors are unchanged; `VectorPath::from_geometry` and `VectorPath::geometry`
are additive. Precision and color handling are untouched; no pixel buffer or
raster layer is introduced.

## Measurement methodology

The harness is designed so the *same source file* compiles at the baseline
revision and with sharing:

- `examples/vector_sharing_bench.rs` measures, per scenario, construction
  (scene clone + native node), preparation (`build_render_tree_in_viewport`),
  per-frame update (scene rebuild plus the scenario mutation, then preparation),
  and uncached painting (`render_to_buffer`, pixel-layer caching explicitly
  off). A wrapping global allocator reports allocation calls and requested
  bytes per operation.
- Each process also runs a fixed allocation-free ALU calibration loop and
  reports `calibration_ms`. Timings are reported raw and normalized as
  `*_norm = median_ms / calibration_ms`. Normalization compensates for a shared
  machine giving different processes different effective CPU speed (P/E core
  placement and background load); allocation counts and requested bytes are
  deterministic and load-independent.
- `core/examples/vector_clone_measure.rs` builds one path with 1000 commands,
  retains 10,000 clones, and reports allocation calls, requested bytes, elapsed
  time and RSS before/after. It is portable across Linux and macOS (RSS from
  `/proc/self/status` or `ps`).
- `scripts/vector_sharing_ab.py` runs the two binaries alternately (baseline
  first on even runs, head first on odd runs), collects 7 runs per scenario and
  prints the median and min..max spread. Raw samples are written next to the
  markdown summary.

Reproduce:

```sh
# Baseline worktree at the revision named in the PR (own target directory!).
git worktree add --detach /tmp/cssimpler-issue17-baseline 6d49b79
cp examples/vector_sharing_bench.rs /tmp/cssimpler-issue17-baseline/examples/
mkdir -p /tmp/cssimpler-issue17-baseline/core/examples
cp core/examples/vector_clone_measure.rs /tmp/cssimpler-issue17-baseline/core/examples/
(cd /tmp/cssimpler-issue17-baseline && CARGO_TARGET_DIR=/tmp/cssimpler-issue17-target \
    cargo build -j 2 --release -p cssimpler --example vector_sharing_bench)
(cd /tmp/cssimpler-issue17-baseline && CARGO_TARGET_DIR=/tmp/cssimpler-issue17-target \
    cargo build -j 2 --release -p cssimpler-core --example vector_clone_measure)

cargo build -j 2 --release -p cssimpler --example vector_sharing_bench
cargo build -j 2 --release -p cssimpler-core --example vector_clone_measure

python3 scripts/vector_sharing_ab.py bench \
    --baseline /tmp/cssimpler-issue17-target/release/examples/vector_sharing_bench \
    --head target/release/examples/vector_sharing_bench --runs 7 \
    --markdown docs/reports/issue-17-vector-sharing-bench.md
python3 scripts/vector_sharing_ab.py clone \
    --baseline /tmp/cssimpler-issue17-target/release/examples/vector_clone_measure \
    --head target/release/examples/vector_clone_measure --runs 7 \
    --markdown docs/reports/issue-17-vector-sharing-clone.md
```

Machine for the recorded run: Apple M1 Pro, 8 logical CPUs, 16 GiB RAM, macOS
`Darwin 27.0.0 arm64`, `rustc 1.101.0-nightly`, `cargo 1.101.0-nightly`,
release profile (`opt-level = 3`, thin LTO, `codegen-units = 1`). The machine is
shared with other agents: load average was around 16-21 during the benchmark
window, so wall-clock numbers carry a validity caveat and normalized values are
the primary timing evidence.

## Recorded results

Recorded on 2026-11-17 from baseline `6d49b79` and head `9b599b3` with 7
alternating runs per scenario. The load average moved from 30.9 to 26.7 during
the bench window (other agents were active), so the raw medians below are
accompanied by the calibration-normalized ratio; the two agreed within a few
percent in this window. Full per-metric tables, min..max spreads and raw JSON
samples are in [`reports/issue-17-vector-sharing-bench.md`](reports/issue-17-vector-sharing-bench.md)
and `...bench.md.jsonl`.

Per-operation medians (baseline → head; allocations per operation):

| Scenario | construct | prepare | per-frame update | uncached paint | construct allocs | update allocs |
|---|---|---|---|---|---|---|
| `small-icon` (128 icon paths, 512²) | 5.29 → 1.17 µs (-77.9%) | 13.63 → 3.33 µs (-75.6%) | 19.04 → 4.50 µs (-76.4%) | 5.93 → 5.95 ms (+0.3%) | 132 → 4 | 528 → 16 |
| `curves` (12 curved/stroked paths, 960×640) | 0.54 → 0.25 µs (-53.7%) | 1.88 → 1.08 µs (-42.6%) | 2.42 → 1.29 µs (-46.7%) | 3.41 → 3.41 ms (0.0%) | 16 → 4 | 64 → 16 |
| `transform-update` (curved scene, transform-only frames) | 0.54 → 0.25 µs | 1.88 → 1.13 µs (-39.9%) | 2.50 → 1.42 µs (-43.2%) | 3.44 → 3.49 ms (+1.4%) | 16 → 4 | 68 → 20 |
| `paint-update` (curved scene, one path repainted) | 0.58 → 0.21 µs | 1.88 → 1.13 µs (-39.9%) | 2.42 → 1.29 µs (-46.7%) | 3.60 → 3.53 ms (-2.0%) | 16 → 4 | 64 → 16 |
| `repeated-edits` (curved scene, one path replaced) | 0.58 → 0.25 µs | 1.88 → 1.13 µs (-39.9%) | 2.42 → 1.37 µs (-43.4%) | 3.46 → 3.56 ms (+2.7%) | 16 → 4 | 65 → 18 |

Requested bytes per operation fell by 17-68% for construction/preparation and
the per-frame update; the remaining bytes are the per-path `Vec`/`SvgPathInstance`
bookkeeping, not command storage. `paint_allocs` and `paint_bytes` were
byte-for-byte identical between revisions in every scenario (for example
3715 allocations / 1,812,000 bytes for `small-icon`), which is the expected
outcome: painting still builds a tiny-skia path from the shared commands.

Retained-storage microbenchmark (10,000 retained clones of one 1,000-command
path; 7 alternating runs, medians with min..max):

| Metric | baseline `6d49b79` | head | change |
|---|---|---|---|
| allocation calls | 10,000 (10,000..10,000) | 1 (1..1) | -100% |
| requested bytes | 280,400,000 (280,400,000..280,400,000) | 240,000 (240,000..240,000) | -99.9% |
| RSS after retaining clones | 283,584 KiB (283,584..284,000) | 2,176 KiB (2,160..2,176) | -99.2% |
| clone loop elapsed | 23.7 ms (23.5..24.0) | 0.049 ms (0.039..0.070) | -99.8% |

The allocation counts, requested bytes and retained RSS are deterministic and
not sensitive to machine load; the elapsed times are indicative only. The
single head allocation is the `Vec` holding the 10,000 `VectorPath` handles;
baseline also allocated one command buffer per clone. Full table and raw samples:
[`reports/issue-17-vector-sharing-clone.md`](reports/issue-17-vector-sharing-clone.md)
and `...clone.md.jsonl`.

## Honest limitations

- The paint phase does not benefit from geometry sharing in this change: the
  renderer still builds a tiny-skia `Path` from the shared commands on every
  paint. Measured paint allocations are identical and paint time is unchanged
  within noise. A prepared-geometry cache is a separate backlog item.
- Construction/preparation improvements scale with command count times instance
  count. Small drawings are already microsecond-scale, so absolute savings are
  small even when relative savings are large; the issue does not predict a
  frame-rate increase.
- Wall-clock timings were taken on a busy shared machine. They are indicative,
  not a controlled benchmark; the deterministic allocation and requested-byte
  counts are the reliable part of the comparison.
- Legacy contour-only SVG paths still store and clone their `Vec<SvgContour>`;
  only native geometry is shared.
- RSS values are process-level and include the retained clone vector and
  allocator bookkeeping; they are evidence of storage growth, not an exact
  accounting of the geometry bytes.
