# Issue 18 - native vector frame diagnostics

## 2026-11-17T20:24Z - issue-18 agent (codex/vector-diagnostics)

### What this branch adds

- `renderer::VectorFrameStats` in the existing `FrameTimingStats` snapshot:
  `paths_painted`, `timings_enabled`, `path_build_work_us`, `stroke_work_us`,
  `projection_work_us`, `coverage_work_us`.
- `FrameTimingStats::layer_cache_hits` / `layer_cache_misses` for promoted
  raster-layer lookups in the same last-completed frame.
- `renderer::set_vector_timing_enabled(bool)` (process-wide, sampled at frame
  start, off by default).
- Counters are saturating atomics reset at frame start and read after the
  frame's paint jobs have been joined. Collection is thread/task-scoped, so an
  unrelated concurrent headless render does not contribute.
- Example `--diagnostics` output and `docs/vector-diagnostics.md`.

Measurements verified against the code:

- `timings_enabled` copies the request at `reset_frame_vector_stats`, so toggling
  mid-frame applies to the next painted frame.
- Work durations accumulate in nanoseconds per paint job and are reported in
  whole microseconds with saturating conversion.
- An idle frame never calls `reset_frame_vector_stats`/`take_frame_vector_stats`;
  `FrameTimingStats` is built from `Default` each frame, so idle frames publish
  zero vector work. `timings_enabled` is false for those frames.
- `layer_cache_hits`/`misses` count the first lookup in
  `cached_promoted_surface`; the second lookup after the build lock is not
  counted, so a concurrent build can satisfy a later lookup uncounted. Documented.
- No prepared-geometry cache exists in the renderer, so no prepared-geometry
  hit/miss count is reported. The docs say so explicitly.

### Validation (commands run in the worktree, CARGO_TARGET_DIR=$PWD/target)

```
cargo check --workspace --all-targets
# Finished dev profile; only pre-existing warnings (unused imports in unrelated
# examples, unused `anyhow` dependency).

cargo test -p cssimpler-renderer --lib vector_
# 4 passed, 0 failed, 1 ignored (release overhead harness).

cargo test -p cssimpler-renderer --lib
# 273 passed; 1 ignored (vector_timing_overhead).

cargo test -p cssimpler --test issue_18_vector_diagnostics
# 2 passed: pixel output identical with timers enabled; fixed-size snapshot and
# detailed timers off by default.
```

### Instrumentation overhead (release, measured)

Harness: `renderer/src/svg.rs::tests::vector_timing_overhead` (ignored by
default). Three modes on the same buffers/scene: `off` (no frame collection),
`counters` (default frame work counts), `timers` (opt-in detailed clocks).
Each outer run does 5 alternating rounds per mode; reported value is that run's
median. Command:

```
cargo test -p cssimpler-renderer --release --lib vector_timing_overhead -- --ignored --nocapture --test-threads=1
```

Machine: Apple M1 Pro, 8 cores, macOS 27.0.1, release profile, 2026-11-17.
1-minute load average was 14-17 during the runs (other agent builds).

Per-run medians in microseconds (5 outer runs):

| workload | mode | run1 | run2 | run3 | run4 | run5 | median of run medians |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 3000 x 24x24 fill | off | 47377 | 51196 | 46932 | 46383 | 46055 | 46932 |
| 3000 x 24x24 fill | counters | 47260 | 52403 | 46797 | 46380 | 45894 | 46797 |
| 3000 x 24x24 fill | timers | 47545 | 61148 | 46672 | 46621 | 46430 | 46672 |
| 100 x 16-path 400x300 fill+stroke | off | 243362 | 266941 | 242446 | 239943 | 239764 | 242446 |
| 100 x 16-path 400x300 fill+stroke | counters | 245625 | 253636 | 245091 | 241822 | 239733 | 245091 |
| 100 x 16-path 400x300 fill+stroke | timers | 246731 | 257413 | 245451 | 240212 | 241231 | 245451 |

Derived from the medians: counters vs off -0.3% (tiny) / +1.1% (mixed);
timers vs counters -0.3% (tiny) / +0.1% (mixed). Run-to-run medians varied by
up to ~10% and run 2 (highest load) has timers at 61148us versus 46.4-47.5ms in
quieter runs. The deltas are not distinguishable from noise, so this machine
does not establish a measurable overhead. Detailed clocks stay opt-in and off by
default. Raw run output: `/tmp/issue18-overhead-runs.txt` (machine-local).

### Open items

- Window example `--diagnostics` functional run: pending (requires an interactive
  window; trying UI-scripted close on this macOS host).
- Full `cargo test --workspace` not run; renderer library and focused integration
  tests were run instead.
- `docs/native-vector-paths.md` was left untouched to avoid conflicting with
  #17's appended section; diagnostics docs live in `docs/vector-diagnostics.md`
  with an index link from `docs/README.md`.

## 2026-11-17T20:32Z - issue-18 agent (resume after host reboot)

Committed and pushed `789333a` ("Clarify vector diagnostics boundaries and measure
overhead") to `codex/vector-diagnostics`, then added the scripted example mode
below. No code in the measured renderer paths changed after the overhead runs
(only doc comments and example/doc edits), so the recorded release medians still
apply to this revision.

### Scripted example validation (no manual window interaction)

`App::with_exit_when` made the example runnable non-interactively. `--frames N`
now shifts the artwork by one pixel on each scripted frame (so every counted
frame paints), exits after N frames, and then prints the snapshot. Without
`--frames` the example keeps its previous interactive behaviour.

```
cargo build -j 2 --example native_vector_paths
./target/debug/examples/native_vector_paths --diagnostics --frames 3
# exit 0, stderr:
# last frame: mode=Incremental reason=IncrementalDamage paint=25997us present=13094us workers=8
# vector: paths_painted=17 build=34us stroke=128us projection=31us coverage=143012us timers=true
# layer cache: hits/misses=0/0 this frame; retained 0 entries / 0 bytes; cumulative builds=0
```

That run is debug-profile and on the loaded shared machine; it is evidence that
the plumbing publishes a painted frame, not a performance claim. It also shows
the documented boundary in practice: summed `coverage_work_us` across 8 workers
(143012us) is not the 25997us frame wall time.

`./target/debug/examples/native_vector_paths --export /tmp/issue18-curves.svg`
still exits 0 and writes the same SVG export path.

### Validation rerun after reboot

```
cargo check --workspace -j 2                          # ok (only pre-existing warnings)
cargo check -j 2 --example native_vector_paths -p cssimpler  # ok
cargo test -j 2 -p cssimpler-renderer --lib vector_   # 4 passed, 1 ignored
cargo test -j 2 -p cssimpler --test issue_18_vector_diagnostics  # 2 passed
cargo test -j 2 -p cssimpler-renderer --lib           # 273 passed, 1 ignored
```

### Remaining

- Full `cargo test --workspace` still not run (renderer lib + focused tests run).
- Overhead numbers come from the loaded shared host; they are recorded with load
  average and raw per-run data so they can be reproduced.
