# Verification thread

Agents append test coverage observations, benchmark design, and validation results here.

## 2026-08-12 - benchmark and test-infrastructure audit (`benchmark_audit`)

### What exists

- There is no Cargo benchmark target and no Criterion, divan, iai-callgrind, `#[bench]`, or `black_box` benchmark in the committed baseline. HEAD currently contains 447 `#[test]` attributes across 70 Rust files; `origin/old-fast` contains 364 across 62 files.
- The main in-app timing harness is `examples/gui_effect_pressure.rs`. It has a fixed preset (28 tiles, 8 passes/tile, 3 animated tiles), 30 warm-up frames, then 120 samples each for idle, animated-paint, and pulse-layout. Set `CSSIMPLER_PRESSURE_BASELINE=1` and run the release example. It prints averages/maxima for render-tree, scene-prep, paint, present, and total time plus paint-mode counts.
- The pressure `.rs` and `.css` files have identical SHA-256 hashes on working HEAD and `origin/old-fast`, so the GUI workload itself is suitable for branch comparisons. It still opens a platform window, samples compositor/presentation noise, does not terminate automatically when the report completes, reports integer averages/max (not raw samples or percentiles), and reports no memory data.
- The public `build_render_tree_in_viewport`, `render_to_buffer`, and `render_scene_update` APIs exist on both branches. This makes a fully headless branch comparator feasible without production changes.
- `examples/headless_render_bench.rs` is now the smallest repeatable CPU/allocation probe. It builds 336 overlapping zero-radius tiles at a configurable resolution, warms eight draws, and reports median/p95/min render time, cold/warm live/peak allocation deltas, total allocated bytes, allocation count, and a pixel checksum. `opaque` isolates the row-fill fast path; `mixed` changes only tile alpha and isolates the generic coverage/blend path.

### Recommended repeatable protocol

Use the same machine, AC power plan, Rust toolchain, dependency lock, viewport, allocator, and logical CPU availability for both branches. Close foreground workloads. Run each process separately and alternate branch order to reduce thermal bias. Do not run competing branch processes simultaneously. Build outside the timed run, then execute the binary directly; save every raw line rather than only a hand-picked result.

```powershell
cargo build --release --offline --locked --example headless_render_bench
target\release\examples\headless_render_bench.exe opaque 50 1920 1080
target\release\examples\headless_render_bench.exe mixed 50 1920 1080
```

For `origin/old-fast`, use `git archive` or a detached worktree, copy the exact same harness source into its `examples/` directory, build with an isolated `--target-dir`, and run that binary directly. Avoid switching the shared worktree while other agents have changes. Record commit SHA and dirty diff for every measurement.

For the exact application workload, also run the GUI pressure harness in release mode on each branch:

```powershell
$env:CSSIMPLER_PRESSURE_BASELINE='1'
cargo run --release --offline --locked --example gui_effect_pressure 2> pressure.log
```

Wait for `status=complete`, then close the window. Run at least five trials per branch. Treat `paint_us` as the primary GUI result; retain render-tree, scene-prep, present, total, mode/reason, damage-pixel, dirty-region/job, and worker-count data so a faster result cannot hide a different workload.

### Exploratory branch results

Environment: Windows x86_64 MSVC, rustc 1.97.0, 1920x1080, release, exact harness source, alternating isolated processes. `old-fast` was commit `e024279030e68fe1b7e5205827008ecc565bdde1`. The current tree was changing during this audit (direct worker-band rendering was already applied), so these are diagnostic results, not a final acceptance record.

Two low-contention 30-sample runs per branch produced:

| Scenario | Current median | old-fast median | Direction |
| --- | ---: | ---: | --- |
| `opaque` | 1.213-1.416 ms | 2.923-3.222 ms | current is 2.1-2.7x faster |
| `mixed` | 49.431-49.602 ms | 42.965-43.381 ms | current is 14-15% slower |

The shapes explain the split: old-fast `draw_rounded_rect` derives a row span once and blends it; current evaluates `rounded_rect_coverage` for every bounding-box pixel before blending. The benchmark uses zero radius, so the translucent gap is not useful antialiasing work. Keep current opaque gains, but restore a span/interior fast path for alpha fills and reserve multisampling for actual edges. Success must beat old-fast here, not merely reach parity.

Memory/allocation data from the same 30-sample runs:

| Metric (per process/run) | Current | old-fast | Interpretation |
| --- | ---: | ---: | --- |
| cold retained allocator bytes | 0 | 8,294,592 | current direct-band work removes old worker-buffer retention |
| cold peak extra allocator bytes | ~852,608-852,680 | ~9,037,320-9,037,344 | current has a much lower first-paint allocation peak |
| warm peak extra allocator bytes | ~852,896-852,920 | ~743,160-743,184 | current warm transient peak is ~110 KiB higher |
| allocated bytes / 30 warm frames | 35,931,600 | 31,265,520 | current allocates 14.9% more total bytes |
| allocations / 30 warm frames | 72,151 | 52,081 | current performs 38.5% more allocations |

This allocator measures Rust `GlobalAlloc`, not RSS, GPU/compositor memory, stack reservation, mapped font files, or native backend allocations. Add process peak working set/private bytes to the release acceptance protocol; use allocator counts to diagnose churn.

An exact 28-tile/8-pass pressure scene was also exercised headlessly through DOM build/layout/full paint/incremental paint. Sequential 10-sample medians were approximately: build current 24.8 ms vs old 24.3 ms; full current 432 ms vs old 251 ms; incremental current 185 ms vs old 137 ms. However current and old build 2102 vs 2092 render nodes and have different pixel hashes because renderer/style functionality changed, so this is a realistic regression signal but not yet a strict equivalent-output gate. The simple `opaque`/`mixed` harness is the cleaner cross-branch microbaseline.

### Correctness status observed

- `cargo test --workspace --all-targets --no-fail-fast` on the in-progress current tree ran headlessly but failed two targets: `transform_3d_playground` had deterministic `rotate_y_in_the_example_keeps_a_broad_visible_face` failure (then two lock-poison followers), and the renderer text-raster LRU test failed in the parallel suite but passed alone and with `--test-threads=1`, indicating shared-cache test interference/flakiness. Other reported targets passed.
- Re-running `cargo test -p cssimpler-renderer --lib -- --test-threads=1` passed all 185 renderer tests.
- `origin/old-fast` cannot complete `cargo test --workspace --all-targets` as archived: its test-only initializer at `core/src/extracted_scene.rs:236` omits the required `BorderStyle.line_style` field. This does not block release/example benchmark builds but means an unmodified old-fast all-target test pass is unavailable.

Before accepting optimizations, rerun current tests serially when global caches are involved, fix the deterministic transform example failure, then run the complete suite normally to catch concurrency defects. Pixel equality/full-vs-incremental tests remain the quality gate; benchmark checksums alone are too weak and cross-branch checksums are expected to differ where feature output changed.

### Coverage gaps / next harness work

- Add incremental equivalents of `opaque` and `mixed`; the current committed headless harness measures full redraw only.
- Add isolated scenes for rounded translucent fill, translucent border/ring, gradient, box shadow, text mask/glow, transformed surface, SVG, backdrop blur, and native-glass alpha. Report work counters alongside time.
- Add a DOM/style/layout benchmark separately from paint. Otherwise paint optimizations can mask render-tree regressions.
- Add raw sample output (CSV/JSON), p50/p90/p95/p99, coefficient of variation, branch/commit/toolchain/CPU metadata, and an optional auto-comparator with gates. Suggested initial gates: no pixel mismatch; no workload-counter mismatch for equivalent scenes; current p50 and p95 at least 10% faster than old-fast after five alternating trials; zero steady-state retained growth; allocation bytes/count no worse unless justified by a larger feature-equivalent workload.
- Add long cache-churn/resize/animation runs. Sample private bytes/RSS after warm-up and after thousands of frames to detect retained-cache growth that a 30-frame allocator delta misses.
- Make worker count explicitly controllable for benchmarks. `available_parallelism()` and OS scheduling currently add platform/noise variance, and a single-thread mode is needed for algorithmic comparisons.
- Add automatic completion/output to the GUI baseline (or a headless `SceneProvider` driver) so CI can run it without a human closing the window.

## 2026-08-12 - final isolated current vs `old-fast` headless comparison

This supersedes the exploratory numbers above for the four microbenchmark scenarios. The current binary includes the completed direct-band/public-render, shape-span, and gradient-span work present in the shared worktree at build time.

### Reproduction identity and method

- Machine/toolchain: Windows x86_64 MSVC, rustc 1.97.0, 8 processors visible to the process.
- Current source base: `59ab60b47f8483f75441c1bbb364b4d33611eeec` plus the uncommitted optimization work. Frozen binary SHA-256: `DDDCC0D5A29FBC6283379E804F4A5CC7E5FF9D5190124EA92367043FA774316C`.
- Baseline: `origin/old-fast` at `e024279030e68fe1b7e5205827008ecc565bdde1`. Frozen binary SHA-256: `787EF293F953DD1EFD5834DADFE4F391D6BF579E37D3AD084480525D8F860B8D`.
- Both source trees used byte-identical `examples/headless_render_bench.rs`, SHA-256 `AEB41F9BD65ACE77DED53655B78FCA70858A2FC221ACBB8C2AFB858A64CFE366`.
- Builds were release, offline, locked, and isolated in `target/final-bench-current` and `target/final-bench-old-fast`. Timed runs invoked the frozen executables directly, so Cargo/linker activity was outside every sample and the binaries could not collide.
- Viewport was 1920x1080. Each invocation was a fresh process with one cold paint and eight warm-up paints. There were five process-level trials per scenario. Branch order alternated for each trial and branch processes never overlapped.
- `opaque` used 100 timed frames per process. `mixed`, `rounded`, and `gradient` used 30 because their frame costs are much higher.
- The table reports the median of the five process medians and median of the five process p95 values. Parentheses give the complete trial range for the respective statistic. No CPU affinity was pinned, so small deltas should be interpreted with the observed trial ranges in mind.

Build commands:

```powershell
cargo build --release --offline --locked `
  --target-dir target\final-bench-current `
  --example headless_render_bench

cargo build --release --offline --locked `
  --manifest-path target\agent-old-fast\Cargo.toml `
  --target-dir target\final-bench-old-fast `
  --example headless_render_bench
```

### Timing results

Lower is better. Delta is current relative to old-fast.

| Scenario | Frames/trial | Current p50 ms (trial range) | old-fast p50 ms (trial range) | p50 delta | Current p95 ms (trial range) | old-fast p95 ms (trial range) | p95 delta |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `opaque` | 100 | 0.836 (0.476-0.966) | 3.513 (2.877-4.440) | -76.2% / 4.20x faster | 1.708 (0.858-2.038) | 4.928 (3.820-13.411) | -65.3% |
| `mixed` | 30 | 70.093 (59.358-73.861) | 71.903 (52.552-72.972) | -2.5% | 75.869 (73.926-78.836) | 79.828 (73.537-80.939) | -5.0% |
| `rounded` | 30 | 42.121 (41.257-46.776) | 41.951 (41.286-42.735) | +0.4% | 47.402 (42.385-70.945) | 47.543 (42.943-66.378) | -0.3% |
| `gradient` | 30 | 28.527 (27.558-31.853) | 29.407 (28.708-31.203) | -3.0% | 30.751 (28.307-53.785) | 32.816 (29.365-38.538) | -6.3% |

Conclusions:

- Opaque full redraw is a decisive win over old-fast, comfortably beyond parity.
- Mixed translucent zero-radius rendering has recovered the earlier regression and is now modestly ahead. The process-median ranges overlap, so the 2.5% p50 lead is directional rather than a large margin.
- Rounded translucent rendering is at practical parity: current p50 is 0.4% slower while aggregate p95 is 0.3% faster. This remains the clearest CPU target if the goal is to outperform old-fast rather than merely match it.
- The large uncached Oklab gradient is 3.0% faster at p50 and 6.3% faster at p95. Its ranges overlap, but it now beats old-fast while preserving the sampled output exactly.

### Allocation and retained-memory results

These values come from the benchmark's process-wide Rust `GlobalAlloc` counter after warm-up. They exclude stacks, mapped files, compositor/GPU memory, and native allocations.

| Scenario | Current bytes/frame | old-fast bytes/frame | Current allocations/frame | old-fast allocations/frame | Current / old cold retained | Current / old cold peak extra | Current / old warm peak extra |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `opaque` | 26,872 | 1,042,184 | 38 | 1,736 | 0 / 8,294,592 B | 26,864 / 9,037,320 B | 27,664 / 743,744 B |
| `mixed` | 26,872 | 1,042,184 | 38 | 1,736 | 0 / 8,294,592 B | 26,792 / 9,037,296 B | 27,104 / 743,184 B |
| `rounded` | 26,872 | 1,042,184 | 38 | 1,736 | 0 / 8,294,592 B | 26,792 / 9,037,296 B | 27,080 / 743,160 B |
| `gradient` | 5,720 | 10,320 | 69 | 82 | 0 / 8,294,592 B | 4,752 / 8,303,744 B | 4,992 / 9,584 B |

For the 336-tile scenes, current reduces allocated bytes/frame by 97.4% and allocation count/frame by 97.8%. For the single-gradient scene it reduces bytes/frame by 44.6% and allocation count by 15.9%. Current retains no additional allocator bytes after the cold paint in any scenario; old-fast retains 8,294,592 bytes, primarily its full-frame worker-buffer pool. The direct public render path and direct output-band painting are therefore substantial RAM/allocation wins, not only CPU wins.

### Checksum verification

Every scenario produced one stable checksum per branch across all five fresh-process trials:

| Scenario | Current checksum | old-fast checksum | Cross-branch result |
| --- | ---: | ---: | --- |
| `opaque` | 4,125,452,846,413 | 4,125,452,846,413 | exact match |
| `mixed` | 4,696,471,911,817 | 4,694,432,025,225 | expected branch difference |
| `rounded` | 4,676,096,338,609 | 4,672,895,779,493 | expected branch difference |
| `gradient` | 7,856,208,937,485 | 7,856,208,937,485 | exact match |

The mixed/rounded cross-branch differences are consistent with the known quality difference: old-fast used binary row spans while current preserves fractional edge coverage/antialiasing. They are deterministic within each implementation. The gradient optimization is output-stable against old-fast for this workload. This benchmark checksum samples roughly 4,096 buffer positions, so renderer pixel-equivalence tests remain the stronger correctness gate.

### Remaining benchmark gap

Add a thin/fractional translucent rounded case with rows that have no full-coverage interior. The current full-coverage span search can otherwise scan edge-only rows before the main coverage loop, and the broad-tile `rounded` scenario does not isolate that worst case. Freeze a new harness hash and measure it separately rather than altering this recorded comparison.
