# Performance results

## 2026-08-12 optimization pass

The first optimization pass targets full-frame CPU cost, transient allocation,
retained worker memory, scene cloning, and exact antialiasing overhead. The
comparison baseline is `origin/old-fast` at
`e024279030e68fe1b7e5205827008ecc565bdde1`.

### Release benchmark summary

The current and old-fast binaries were built into isolated target directories
from the same benchmark source. Each result below is the median of five fresh
process trials at 1920x1080; branch order alternated between trials.

| Scenario | Current p50 | old-fast p50 | Result |
| --- | ---: | ---: | ---: |
| Opaque tiles | 0.836 ms | 3.513 ms | 4.20x faster |
| Translucent tiles | 70.093 ms | 71.903 ms | 2.5% faster |
| Rounded translucent tiles | 42.121 ms | 41.951 ms | practical parity (0.4% slower) |
| Uncached Oklab gradient | 28.527 ms | 29.407 ms | 3.0% faster |

The 336-tile scenes allocate 26,872 bytes in 38 allocations per frame, down
from 1,042,184 bytes in 1,736 allocations per frame on old-fast. That is a
97.4% reduction in allocated bytes and a 97.8% reduction in allocation count.
Cold retained allocation is zero, versus 8,294,592 bytes on old-fast.

See [`../agent_coms/verification.md`](../agent_coms/verification.md) for trial
ranges, p95 results, binary and harness hashes, raw methodology, allocation
details, and checksums.

### Implemented changes

- Full redraw workers paint disjoint slices of the destination color and alpha
  buffers directly, removing full-frame scratch buffers and copy-back passes.
- Public full and incremental rendering bypass unused extracted paint-item and
  root-forest construction.
- Runtime scene extraction takes ownership of captured roots instead of deep
  cloning a second root forest.
- Axis-aligned fills and gradients retain exact edge antialiasing while skipping
  multisample coverage work for pixels already proven fully covered.
- Normal RGB rounded-fill blending avoids per-pixel alpha-target lookups.
- No-op auto-scroll indicator redraws return before rebuilding scene bounds.
- Runtime ECS patching walks borrowed authored children instead of recursively
  cloning descendant subtrees.
- Shared text-cache tests now isolate LRU policy checks from unrelated parallel
  renderer activity.

### Quality and follow-up gates

- Opaque and gradient benchmark checksums match old-fast exactly.
- Mixed and rounded output intentionally differs from old-fast because the
  current renderer retains fractional edge antialiasing that old-fast lacked.
- Rounded translucent performance is the next paint target: it is at parity,
  not yet a clear win.
- Add a thin/subpixel rounded workload before changing span discovery so rows
  without a fully covered interior cannot regress.
- Continue with the documented repeated stacking/hit-test traversal, focused
  input root cloning, compact runtime glass metadata, and native-glass presenter
  buffer work after this pass.
