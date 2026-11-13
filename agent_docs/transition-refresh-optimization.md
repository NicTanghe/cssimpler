# Transition refresh optimization

A paint refresh can change status text while leaving every element's target
geometry and style unchanged. Previously, updating that target rebuilt the
entire transition start tree and plan, then cloned the candidate target again.
The new fast path checks node kinds, identities, geometry, styles, transitions
and tree shape. If those match, it retains the existing start tree and plan and
moves the new target into place. Text and other target content can update. Any
changed geometry, style, identity or tree shape takes the existing conservative
rebase path. Existing matching and transition semantics are unchanged.

## CPU comparison

Apple M1 Pro (16 GiB), macOS 27.0.1, Rust 1.99.1, release builds. The baseline
is `ac4c2eec` (PR #9); the candidate is this change. Each cell is the median of
five independent runs, with the two binaries executed alternately. Each run
has 32 warm-up and 240 measured simulated 16 ms frames. The harness changes a
status label every frame and toggles a 400 ms layout transition every 32 frames,
with 100 or 500 additional elements. `idle` omits the transition. Times include
`App::frame` and its returned-scene clone, not painting, presentation or GPU.
The counting allocator adds overhead; bytes are cumulative allocation requests,
not retained or peak memory. Raw runs: [JSON](transition-refresh-optimization.json).

| Extra nodes | Refresh | Median baseline → candidate | P95 baseline → candidate | Allocated bytes/frame baseline → candidate |
|---:|---|---:|---:|---:|
| 100 | idle | 214 → 211 µs | 290 → 303 µs | 1,603,173 → 1,603,173 |
| 100 | full | 240 → 213 µs | 283 → 258 µs | 1,871,522 → 1,697,359 |
| 100 | partial | 137 → 111 µs | 183 → 139 µs | 1,011,102 → 836,939 |
| 500 | idle | 974 → 971 µs | 1,084 → 1,050 µs | 6,975,269 → 6,975,269 |
| 500 | full | 1,164 → 1,036 µs | 1,297 → 1,217 µs | 8,281,685 → 7,431,642 |
| 500 | partial | 662 → 535 µs | 818 → 662 µs | 4,590,753 → 3,740,710 |

Both correct versions reached all 7 of 7 checked endpoints in every run and
active case. At 500 nodes, median full/partial times fell approximately 11%/19%
and requested allocation volume fell 10%/19%. Idle has no allocation change;
small timing differences and the P95 are sensitive to desktop activity and
unlocked clocks. This is not a FoldCraft or native-window FPS measurement.

The benchmark source is [examples/transition_refresh_bench.rs](../examples/transition_refresh_bench.rs).
To compare a different revision, use separate temporary worktrees and separate
Cargo target directories; copy the example into a revision which predates it.
For each worktree run `cargo run --release --locked -j 2 --example
transition_refresh_bench -- 500` (and `100`), alternate revisions, and retain
five runs. Do not share target directories across worktrees: that previously
reused the wrong binary. No GPU is required.

Validation: `cargo test --locked --workspace -j 2 -- --test-threads=2` passed.
The focused test compares fast-path output against the existing rebase at several
points on the timeline, confirms changed geometry, style, ID and structure refuse
the fast path, and keeps the candidate intact on refusal. Existing tests cover
full and partial refresh for both App types and sibling matching.
Native window/GPU behavior and `origin/old-fast` were not benchmarked; the latter
is not an equivalent baseline for the newer transition/handler APIs.
