# Native vector frame diagnostics

`cssimpler::renderer::latest_frame_timing_stats()` returns a fixed-size copy of
the last **completed window frame**. Issue #18 extends that existing snapshot
instead of adding a second telemetry system: `FrameTimingStats::vector` carries
native vector work and `FrameTimingStats::layer_cache_hits` /
`FrameTimingStats::layer_cache_misses` carry promoted raster-layer lookups for
the same frame. `layer_cache_stats()` continues to report retained layer totals
and cumulative builds.

These values describe the renderer's painting work. They do not include
application geometry construction, style resolution, layout, or scene capture
(`FrameTimingStats::update_us` and `scene_prep_us` cover the frame phases before
painting). No per-path data is retained, and collecting diagnostics never
allocates raster layers.

## Reading the snapshot

| Field | Unit | Meaning |
| --- | --- | --- |
| `vector.paths_painted` | count | Path paint attempts that survived invalid geometry/bounds and coarse clipping. Counted once per paint job or damage band, not per unique path, and not a guarantee of visible pixels. Attempts whose geometry later fails to convert are included. |
| `vector.timings_enabled` | bool | Whether the detailed duration fields were measured for this frame. Sampled when the frame begins painting; false for idle frames and unless enabled with `set_vector_timing_enabled(true)`. |
| `vector.path_build_work_us` | µs | Renderer geometry to tiny-skia path conversion. Excludes application-side vector construction. |
| `vector.stroke_work_us` | µs | Stroke outline construction in source space. |
| `vector.projection_work_us` | µs | Source-to-screen projection of fill and stroke outlines. |
| `vector.coverage_work_us` | µs | Coverage rasterization and color blending, including clipping. |
| `layer_cache_hits` | count | Promoted raster-layer lookups that found a retained layer during the frame. |
| `layer_cache_misses` | count | Promoted raster-layer lookups that missed, including lookups whose layer could not be retained because of limits or bounds. |
| `paint_us` | µs | Wall-clock time spent in the frame's painting phase. |
| `present_us` | µs | CPU-side presentation elapsed time. This is not GPU execution time. |

The four `*_work_us` fields are sums of elapsed measurements taken inside paint
jobs. Parallel workers and damage bands overlap, and the measurements include
scheduler interruptions, so they are neither CPU-clock time nor frame wall time.
They can exceed `paint_us` when jobs overlap. Durations are accumulated in
nanoseconds and reported in whole microseconds; all counters saturate rather
than wrap.

## Frame kinds and counting rules

- **Idle** frames paint nothing, so vector counts and durations are zero and
  `timings_enabled` is false. Inspect a frame that actually repainted when
  investigating a slow drawing.
- **Full redraw** visits every vector leaf once per row band/task, so a path can
  be counted once per band.
- **Incremental redraw** visits only the damage bands. A path that intersects
  several bands is counted once per band, even if the bands come from the same
  damage region.
- **Clipped** paths whose transformed bounds miss the coarse clip are not
  counted. A counted attempt can still produce no visible pixels after fine
  clipping.
- **Cached raster layers** avoid path painting on a hit. A layer build paints
  its subtree (twice for the alpha-evaluation buffers), so one frame can count
  the same path more than once while building a layer that will serve later
  frames.
- **Worker aggregation** uses process-wide atomic counters. Work durations are
  summed across workers; that sum is not wall time. The snapshot is published
  once the frame's paint jobs have been joined, so it is consistent. Collection
  is scoped to the frame's jobs; an unrelated concurrent headless render does
  not contribute. The window event loop serializes window frames. The snapshot
  store and counters are process-wide, as with the existing latest-frame API,
  so run one window event loop at a time when reading them.

There is no prepared-geometry cache in the renderer yet, so no
prepared-geometry hit/miss counts are reported. `paths_painted` and the work
durations are the available preparation/painting signals. If such a cache is
added, extend this snapshot instead of adding another telemetry system.

## Retained layers versus cumulative builds

`layer_cache_stats()` reports:

- `entries`: raster layers retained right now.
- `bytes`: bytes held by those retained layers.
- `builds`: cumulative successful raster builds since process start. Clearing
  the cache or lowering limits does not reset it, so growth across frames
  indicates repeated rebuilds.

The per-frame `layer_cache_hits`/`layer_cache_misses` are separate from these
retained totals. A miss is counted before the build lock; if another job builds
the same layer first, the later second lookup can hit without being counted
again. Repeated misses together with a rising `builds` count on unchanged
artwork suggest invalidation or cache-limit problems rather than expensive
geometry.

## Enabling detailed timers

Detailed clocks are opt-in and off by default:

```rust
use cssimpler::renderer::{latest_frame_timing_stats, set_vector_timing_enabled};

set_vector_timing_enabled(true);
// ... run the window; the switch applies to the next frame that starts painting.
let stats = latest_frame_timing_stats();
eprintln!("{:?}", stats.vector);
```

The switch is process-wide and cheap when disabled: work counts are still
collected, but no per-path clock reads or duration accumulation happen. The
runnable example prints the snapshot after the window closes:

```sh
cargo run --example native_vector_paths -- --diagnostics
```

For scripted, non-interactive runs add `--frames N`: the example shifts the
artwork slightly on every frame so each frame paints, exits after `N` frames,
and then prints the snapshot. For example:

```sh
cargo run --example native_vector_paths -- --diagnostics --frames 3
```

When interpreting a slow frame, first confirm `paint_mode` is not idle and look
at `paint_us` with `render_workers`/`dirty_jobs`. Then compare
`coverage_work_us` (rasterization and blending) with `path_build_work_us`,
`stroke_work_us`, and `projection_work_us` (preparation). Large preparation
time combined with repeated `layer_cache_misses` and rising `builds` points at
cache invalidation; large coverage time points at pixels or blending rather than
path preparation.

## Instrumentation overhead

The overhead harness is a release-mode, ignored test that measures three
collection modes on the same scenes and buffers: `off` (no frame collection,
the closest equivalent to the pre-diagnostics renderer), `counters` (the default
frame diagnostics), and `timers` (the opt-in detailed clocks). It alternates
mode order across five rounds and reports medians, so shared-machine drift does
not consistently favour one mode. Reproduce with:

```sh
cargo test -p cssimpler-renderer --release --lib vector_timing_overhead -- --ignored --nocapture --test-threads=1
```

Measured on 2026-11-17 on an Apple M1 Pro (8 cores, macOS 27.0.1) with the
release profile. Five outer runs, each performing five alternating rounds per
mode, gave these medians (µs):

| Workload | off | counters | timers | counters vs off | timers vs counters |
| --- | ---: | ---: | ---: | ---: | ---: |
| 3,000 × 24×24 filled path | 46,932 | 46,797 | 46,672 | −0.3% | −0.3% |
| 100 × 16-path 400×300 fill+stroke scene | 242,446 | 245,091 | 245,451 | +1.1% | +0.1% |

The machine was loaded throughout (1-minute load average 14–17 from unrelated
builds), and per-run medians varied by up to about 10%, with the per-mode
ordering inconsistent across runs. The differences above are therefore not
distinguishable from noise; this run does not establish a measurable overhead
for either always-on work counts or the opt-in detailed clocks. The tiny-path
workload is the worst case for per-path clocks, so the detailed timers are safe
to enable for investigation and remain off by default. This is a microbenchmark;
it does not predict application frame times. Full run data is recorded in
`agent_coms/issue-18.md`.
