# Performance and memory strategy

## Goal

Outperform `origin/old-fast` where practical while preserving the current library's rendering quality, behavior, and feature set. The old branch is a baseline and source of proven ideas, not a ceiling.

## Guardrails

1. Establish a repeatable current baseline before judging an optimization.
2. Compare algorithms and data flow with `origin/old-fast`; do not blindly restore code that drops newer functionality, and improve on its techniques where current architecture allows it.
3. Prefer removing repeated work, temporary allocations, redundant copies, and oversized retained buffers.
4. Keep changes narrow enough to attribute benchmark movement to a cause.
5. Run the full correctness suite after each meaningful optimization and add regression coverage where existing tests are weak.
6. Record both improvements and rejected experiments so later work does not repeat them.

## Work phases

### 1. Inventory and baseline

- Map layout, style, scene extraction, rendering, and application-loop ownership.
- Find existing stress examples and test coverage.
- Add deterministic workloads that can be exercised without a visible window where practical.
- Capture elapsed time, allocation/retained-memory proxies, and workload dimensions.

### 2. Regression archaeology

- Diff `origin/old-fast` against the current branch by subsystem.
- Identify changed data structures, cache lifetimes, traversal counts, framebuffer handling, and invalidation scope.
- Port only techniques compatible with current semantics.

### 3. Optimization

- Start with high-frequency paths and large allocations.
- Reuse buffers and caches with explicit invalidation/lifetime rules.
- Avoid cloning scene, style, text, and pixel data in inner loops.
- Reduce whole-tree and whole-frame work when dirty-region information is already available.

### 4. Verification

- Require correctness tests and release-mode measurements.
- Compare identical workload, dimensions, and iteration counts across current HEAD, `origin/old-fast`, and the optimized current branch where practical.
- Report median/dispersion for noisy timings and peak/steady-state memory where tooling permits.
- Retain a concise before/after table and note platform limitations.

## Initial success criteria

- All existing tests remain green.
- No documented visual or interaction feature is removed or weakened.
- At least one representative workload shows a reproducible improvement in runtime, allocation count/volume, or retained memory, with `origin/old-fast` used as a secondary target to beat where an equivalent workload can be built.
- New performance harnesses are straightforward to rerun.
